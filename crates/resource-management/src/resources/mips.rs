use std::{alloc::Global, error::Error, fmt, simd::Simd};

use exr::prelude::f16;
use utils::{
	Extent,
	color::{linear_to_srgb, srgb_to_linear},
};

use crate::types::{Formats, Gamma};

#[cfg(any(test, feature = "gpu-mips"))]
pub(crate) mod bc7;
mod encoding;
#[cfg(feature = "gpu-mips")]
pub mod gpu;

pub(crate) use encoding::encode_level_in;

/// The `MipLevel` struct borrows one generated level so the encoder can read it in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MipLevel<'a> {
	pub(crate) width: u32,
	pub(crate) height: u32,
	pub(crate) data: &'a [u8],
}

/// The `Coverage` struct asks a mip chain to keep an alpha-tested image's coverage in every level, so cut-outs thinner
/// than a texel of a smaller level don't average below the cutoff and disappear at a distance.
///
/// The image processor builds one from [`crate::processors::image::ImageProcessorSettings::coverage`] and passes it to
/// [`MipGenerator::encode_mip_chain`] or [`MipGenerator::filter_mip_chain_in_place`]. The reference's
/// [coverage](/docs/reference/assets#coverage) section specifies the result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Coverage {
	/// The channel that holds opacity.
	pub(crate) channel: usize,
	/// The opacity, from 0 to 1, at or above which a material keeps a texel.
	pub(crate) cutoff: f32,
}

/// The `MipGenerator` enum selects where the image processor filters and block-compresses texture mip chains.
///
/// An asset manager's [`crate::processors::image::ImageProcessor`] owns one generator and filters the chain of every
/// image whose [`crate::processors::image::ImageProcessorSettings::mips`] is set with it. `Cpu` filters and encodes on
/// the calling thread. `Gpu` submits to the offline GPU worker and falls back to the CPU path for requests it can't
/// serve or whose GPU work fails.
pub enum MipGenerator {
	/// Filters and encodes every level on the CPU.
	///
	/// Keep this path even though bakes normally use the GPU: it bakes wherever GPU setup or a GPU request
	/// fails, it's what bakes and tests use on machines without a compatible GPU, and its `intel_tex_2` encoder is the
	/// reference the GPU BC7 encoder's quality is measured against.
	Cpu,
	/// Filters and BC7-compresses on the offline GPU worker.
	#[cfg(feature = "gpu-mips")]
	Gpu(gpu::MaterialMipGenerator),
}

/// The `MipClaim` struct holds an image's place at its [`MipGenerator`], from before the image's pixels exist until its
/// chain is encoded.
///
/// The image processor takes one with [`MipGenerator::claim`] before it allocates an image, so images wait for the GPU
/// while they hold no memory, instead of queueing at the GPU with their pixels decoded. The CPU generator has no queue,
/// so its claims never wait.
pub(crate) struct MipClaim {
	#[cfg(feature = "gpu-mips")]
	gpu: Option<crate::gpu_worker::GpuClaim>,
}

impl MipGenerator {
	/// Waits until the generator can take another image soon, and returns the claim its chain is encoded under.
	///
	/// Claim before allocating the image's pixels, and keep the claim until the chain is encoded. Never await another
	/// bake while holding it.
	pub(crate) async fn claim(&self) -> MipClaim {
		match self {
			Self::Cpu => MipClaim {
				#[cfg(feature = "gpu-mips")]
				gpu: None,
			},
			#[cfg(feature = "gpu-mips")]
			Self::Gpu(generator) => MipClaim {
				gpu: Some(generator.claim().await),
			},
		}
	}

	/// Filters `base_level` down to one texel and writes every level, base level first, into `output` encoded as
	/// `output_format`.
	///
	/// `base_level` holds `width` by `height` texels in the [`filtering_format`] of `output_format`. `output` holds
	/// exactly [`encoded_mip_chain_size`] bytes, and each level directly follows the one before it. `claim` comes from
	/// [`Self::claim`]. With `coverage`, each level below the base keeps the base level's coverage. The GPU path awaits
	/// its worker without blocking the calling runtime thread.
	pub(crate) async fn encode_mip_chain(
		&self,
		claim: &MipClaim,
		output_format: Formats,
		gamma: Gamma,
		width: u32,
		height: u32,
		base_level: &[u8],
		output: &mut [u8],
		coverage: Option<Coverage>,
	) -> Result<(), MipGenerationError> {
		// The CPU path has no queue, so only the GPU path reads the claim.
		#[cfg(not(feature = "gpu-mips"))]
		let _ = claim;
		// The GPU encodes the levels it filters before the CPU could scale them, so chains that keep coverage stay on the
		// CPU.
		#[cfg(feature = "gpu-mips")]
		if let (Self::Gpu(generator), Some(claim), None) = (self, &claim.gpu, coverage) {
			match generator
				.encode_mip_chain(claim, output_format, gamma, width, height, base_level, output)
				.await
			{
				Ok(()) => return Ok(()),
				// Requests outside the GPU path's formats are expected and take the CPU path quietly.
				Err(gpu::GPUMipError::UnsupportedRequest) => {}
				Err(error) => log::warn!(
					"GPU material mip generation failed; using the CPU fallback. The most likely cause is an unavailable or unsupported GPU path. Error: {error}"
				),
			}
		}

		// The CPU path filters and encodes on the calling thread.
		let expected = encoded_mip_chain_size(output_format, Extent::rectangle(width, height))
			.ok_or(MipGenerationError::UnsupportedFormat(output_format))?;
		if output.len() != expected {
			return Err(MipGenerationError::BufferSizeMismatch {
				expected,
				got: output.len(),
			});
		}

		let filtering_format = filtering_format(output_format);
		let (bytes_per_pixel, mut lower_levels) = generate_lower_levels(filtering_format, gamma, width, height, base_level)?;
		if let Some(coverage) = coverage {
			keep_coverage(filtering_format, width, height, base_level, &mut lower_levels, coverage)?;
		}
		encode_levels(
			output_format,
			width,
			height,
			bytes_per_pixel,
			base_level,
			&lower_levels,
			output,
		);
		Ok(())
	}

	/// Filters the base level at the start of `chain` down to one texel, writing each level into `chain` right after
	/// the one before it.
	///
	/// This is the copy-free form of [`Self::encode_mip_chain`] for formats that [`stores_filtering_texels`]: the
	/// stored base level is the filtering surface, so each level filters straight from the previous one in place, with
	/// no separate base or lower-level buffers. `chain` holds exactly [`encoded_mip_chain_size`] bytes, and only its
	/// base level needs to be written. `claim` comes from [`Self::claim`]. With `coverage`, each level below the base
	/// keeps the base level's coverage.
	pub(crate) async fn filter_mip_chain_in_place(
		&self,
		claim: &MipClaim,
		format: Formats,
		gamma: Gamma,
		width: u32,
		height: u32,
		chain: &mut [u8],
		coverage: Option<Coverage>,
	) -> Result<(), MipGenerationError> {
		if !stores_filtering_texels(format) {
			return Err(MipGenerationError::UnsupportedFormat(format));
		}
		let expected = encoded_mip_chain_size(format, Extent::rectangle(width, height))
			.ok_or(MipGenerationError::UnsupportedFormat(format))?;
		if chain.len() != expected {
			return Err(MipGenerationError::BufferSizeMismatch {
				expected,
				got: chain.len(),
			});
		}

		#[cfg(not(feature = "gpu-mips"))]
		let _ = claim;
		#[cfg(feature = "gpu-mips")]
		let filtered = match (self, &claim.gpu) {
			(Self::Gpu(generator), Some(claim)) => {
				match generator
					.filter_mip_chain_in_place(claim, format, gamma, width, height, chain)
					.await
				{
					Ok(()) => true,
					// Requests outside the GPU path's formats are expected and take the CPU path quietly.
					Err(gpu::GPUMipError::UnsupportedRequest) => false,
					Err(error) => {
						log::warn!(
							"GPU texture mip generation failed; using the CPU fallback. The most likely cause is an unavailable or unsupported GPU path. Error: {error}"
						);
						false
					}
				}
			}
			_ => false,
		};
		#[cfg(not(feature = "gpu-mips"))]
		let filtered = false;

		let bytes_per_pixel = format.texel_bytes().ok_or(MipGenerationError::UnsupportedFormat(format))?;
		let (base_level, lower_levels) = chain.split_at_mut(width as usize * height as usize * bytes_per_pixel);
		if !filtered {
			filter_lower_levels(format, gamma, width, height, base_level, lower_levels)?;
		}
		// Coverage scales levels that are already filtered, so it applies the same way to either filter's output.
		if let Some(coverage) = coverage {
			keep_coverage(format, width, height, base_level, lower_levels, coverage)?;
		}
		Ok(())
	}
}

/// Returns whether `format` stores the texels it is filtered in unchanged, so its mip chain can be filtered in the
/// stored buffer itself.
///
/// Block-compressed and packed formats encode their filtering surface into something else; every other storable
/// format is its own filtering surface.
pub fn stores_filtering_texels(format: Formats) -> bool {
	!matches!(
		format,
		Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB | Formats::RG8 | Formats::RG16
	)
}

/// Returns the levels below a `width` by `height` base level as stored back to back in `data`.
///
/// The CPU filter writes lower levels in this layout and the GPU path reads them back in it, with `bytes_per_pixel`
/// per texel.
pub(crate) fn packed_lower_levels(
	width: u32,
	height: u32,
	bytes_per_pixel: usize,
	data: &[u8],
) -> impl Iterator<Item = MipLevel<'_>> {
	mip_extents(width, height)
		.skip(1)
		.scan(0usize, move |offset, (width, height)| {
			let size = width as usize * height as usize * bytes_per_pixel;
			let level = MipLevel {
				width,
				height,
				data: &data[*offset..*offset + size],
			};
			*offset += size;
			Some(level)
		})
}

/// Returns the bytes the levels of [`packed_lower_levels`] take together.
pub(crate) fn packed_lower_levels_size(width: u32, height: u32, bytes_per_pixel: usize) -> usize {
	mip_extents(width, height)
		.skip(1)
		.map(|(width, height)| width as usize * height as usize * bytes_per_pixel)
		.sum()
}

/// Encodes a `width` by `height` `base_level` and the already filtered `lower_levels` below it, back to back into
/// `output` as `output_format`.
///
/// `lower_levels` holds texels of `bytes_per_pixel` bytes laid out as [`packed_lower_levels`] reads them, and `output`
/// holds at least the levels' encoded sizes. The GPU path shares this step for the levels it filtered.
pub(crate) fn encode_levels(
	output_format: Formats,
	width: u32,
	height: u32,
	bytes_per_pixel: usize,
	base_level: &[u8],
	lower_levels: &[u8],
	output: &mut [u8],
) {
	let base_level = MipLevel {
		width,
		height,
		data: base_level,
	};
	let mut remaining = output;
	for level in std::iter::once(base_level).chain(packed_lower_levels(width, height, bytes_per_pixel, lower_levels)) {
		let extent = Extent::rectangle(level.width, level.height);
		let size = encoded_mip_level_size(output_format, extent).expect("A storable chain has a stored size for every level");
		let (destination, rest) = std::mem::take(&mut remaining).split_at_mut(size);
		encode_level_in(output_format, extent, level.data, destination, Global);
		remaining = rest;
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MipGenerationError {
	ZeroDimensions,
	UnsupportedFormat(Formats),
	BufferSizeMismatch { expected: usize, got: usize },
	DimensionsTooLarge,
}

impl fmt::Display for MipGenerationError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			MipGenerationError::ZeroDimensions => write!(
				f,
				"Invalid image dimensions. The most likely cause is a width or height set to zero."
			),
			MipGenerationError::UnsupportedFormat(format) => write!(
				f,
				"Unsupported image format {:?}. The most likely cause is using a compressed format before mip generation.",
				format
			),
			MipGenerationError::BufferSizeMismatch { expected, got } => write!(
				f,
				"Invalid image buffer size: expected {}, got {}. The most likely cause is mismatched dimensions or format metadata.",
				expected, got
			),
			MipGenerationError::DimensionsTooLarge => write!(
				f,
				"Image dimensions are too large. The most likely cause is overflow while calculating buffer sizes."
			),
		}
	}
}

impl Error for MipGenerationError {}

/// Returns the number of mip levels needed to reach 1x1 from the provided base size.
pub fn mip_level_count(width: u32, height: u32) -> Result<u32, MipGenerationError> {
	if width == 0 || height == 0 {
		return Err(MipGenerationError::ZeroDimensions);
	}

	// Halving stops once the larger side reaches one texel, so the count is its bit length.
	Ok(u32::BITS - width.max(height).leading_zeros())
}

/// Returns each level extent from `width` by `height` down to 1x1, halving each side and flooring it at one texel.
pub(crate) fn mip_extents(width: u32, height: u32) -> impl Iterator<Item = (u32, u32)> {
	std::iter::successors(Some((width, height)), |&(width, height)| {
		(width > 1 || height > 1).then(|| ((width / 2).max(1), (height / 2).max(1)))
	})
}

/// Returns the format mip levels are filtered in before they are encoded as `output_format`.
///
/// Block-compressed outputs, sRGB RGBA8, and packed RG8 filter as RGBA8 texels, so the GPU path can filter them.
/// Packed RG16 filters as RGBA16. Every other format, including the half-float ones, filters in place. Convert the
/// base level to this format before a [`MipGenerator`] encodes its chain.
pub fn filtering_format(output_format: Formats) -> Formats {
	match output_format {
		Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB | Formats::RGBA8SRGB | Formats::RG8 => {
			Formats::RGBA8
		}
		Formats::RG16 => Formats::RGBA16,
		format => format,
	}
}

/// Returns the exact stored size of a complete mip chain encoded as `format`, or `None` for formats images can't be
/// stored in.
///
/// Size the buffer a [`MipGenerator`] encodes a chain into with it.
pub fn encoded_mip_chain_size(format: Formats, extent: Extent) -> Option<usize> {
	mip_extents(extent.width(), extent.height()).try_fold(0usize, |total, (width, height)| {
		total.checked_add(encoded_mip_level_size(format, Extent::rectangle(width, height))?)
	})
}

/// Returns the stored size of one level in a format the image processor can encode.
pub(crate) fn encoded_mip_level_size(format: Formats, extent: Extent) -> Option<usize> {
	match format {
		Formats::RGB8 | Formats::RGB16 => None,
		format => format.level_size(extent),
	}
}

/// Filters the levels below a base level into one allocation and returns them with their texel size.
///
/// The levels are laid out as [`packed_lower_levels`] reads them.
fn generate_lower_levels(
	format: Formats,
	gamma: Gamma,
	width: u32,
	height: u32,
	base_level: &[u8],
) -> Result<(usize, Vec<u8>), MipGenerationError> {
	if width == 0 || height == 0 {
		return Err(MipGenerationError::ZeroDimensions);
	}
	let bytes_per_pixel = format.texel_bytes().ok_or(MipGenerationError::UnsupportedFormat(format))?;
	let expected_base_size = (width as usize)
		.checked_mul(height as usize)
		.and_then(|texels| texels.checked_mul(bytes_per_pixel))
		.ok_or(MipGenerationError::DimensionsTooLarge)?;
	if base_level.len() != expected_base_size {
		return Err(MipGenerationError::BufferSizeMismatch {
			expected: expected_base_size,
			got: base_level.len(),
		});
	}
	// The lower levels together are smaller than the base level, so their sizes can't overflow.
	let mut data = vec![0_u8; packed_lower_levels_size(width, height, bytes_per_pixel)];
	filter_lower_levels(format, gamma, width, height, base_level, &mut data)?;

	Ok((bytes_per_pixel, data))
}

/// Filters `base_level` into the levels below it, laid out in `lower_levels` as [`packed_lower_levels`] reads them.
fn filter_lower_levels(
	format: Formats,
	gamma: Gamma,
	width: u32,
	height: u32,
	base_level: &[u8],
	lower_levels: &mut [u8],
) -> Result<(), MipGenerationError> {
	let bytes_per_pixel = format.texel_bytes().ok_or(MipGenerationError::UnsupportedFormat(format))?;
	// Each level filters the one before it, which the previous iteration just wrote.
	let (mut source, mut remaining) = (base_level, lower_levels);
	for ((source_width, source_height), (level_width, level_height)) in
		mip_extents(width, height).zip(mip_extents(width, height).skip(1))
	{
		let (destination, rest) =
			std::mem::take(&mut remaining).split_at_mut(level_width as usize * level_height as usize * bytes_per_pixel);
		downsample_level(format, gamma, source_width, source_height, source, destination)?;
		source = destination;
		remaining = rest;
	}
	Ok(())
}

/// Downsamples one level according to format and channel depth.
fn downsample_level(
	format: Formats,
	gamma: Gamma,
	source_width: u32,
	source_height: u32,
	source: &[u8],
	destination: &mut [u8],
) -> Result<(), MipGenerationError> {
	match format {
		Formats::RGB8 => downsample_unorm::<3, 1>(source_width, source_height, source, destination),
		Formats::RGBA8 | Formats::RGBA8SRGB if gamma == Gamma::SRGB => {
			downsample_rgba8_srgb(source_width, source_height, source, destination)
		}
		Formats::RGBA8 | Formats::RGBA8SRGB => downsample_unorm::<4, 1>(source_width, source_height, source, destination),
		Formats::RGB16 => downsample_unorm::<3, 2>(source_width, source_height, source, destination),
		Formats::RGBA16 => downsample_unorm::<4, 2>(source_width, source_height, source, destination),
		Formats::R16F => downsample_f16::<1>(source_width, source_height, source, destination),
		Formats::RGBA16F => downsample_f16::<4>(source_width, source_height, source, destination),
		// Packed RG formats filter as their RGBA filtering format and are truncated when each level is encoded.
		_ => return Err(MipGenerationError::UnsupportedFormat(format)),
	}

	Ok(())
}

/// Scales the opacity channel of every level in `lower_levels`, laid out as [`packed_lower_levels`] reads them, so each
/// level's coverage comes as close as its texels allow to `base_level`'s.
///
/// A level's coverage is the share of its texels whose opacity channel, as stored, is at or above the cutoff. Each
/// level is scaled on its own from its filtered values, so a level's scale never compounds into the next one; filter
/// the whole chain before calling this. Eight- and sixteen-bit unsigned formats are scaled; half-float levels keep
/// their values.
fn keep_coverage(
	format: Formats,
	width: u32,
	height: u32,
	base_level: &[u8],
	lower_levels: &mut [u8],
	coverage: Coverage,
) -> Result<(), MipGenerationError> {
	match format {
		Formats::RGBA8 | Formats::RGBA8SRGB => keep_unorm_coverage::<1>(width, height, base_level, lower_levels, coverage),
		Formats::RGBA16 => keep_unorm_coverage::<2>(width, height, base_level, lower_levels, coverage),
		Formats::R16F | Formats::RGBA16F => {}
		_ => return Err(MipGenerationError::UnsupportedFormat(format)),
	}
	Ok(())
}

/// Keeps coverage for RGBA levels of `BYTES`-wide unsigned-normalized samples. See [`keep_coverage`].
fn keep_unorm_coverage<const BYTES: usize>(
	width: u32,
	height: u32,
	base_level: &[u8],
	lower_levels: &mut [u8],
	coverage: Coverage,
) {
	let texel_bytes = 4 * BYTES;
	let max = (1_u32 << (8 * BYTES)) - 1;
	// The smallest stored value that passes: value / max >= cutoff.
	let cutoff = ((coverage.cutoff * max as f32).ceil() as u32).min(max);
	if cutoff == 0 {
		return;
	}
	// The opacity sample's bytes within a texel, little-endian.
	let opacity = coverage.channel * BYTES..(coverage.channel + 1) * BYTES;
	let sample = |texel: &[u8]| -> u32 {
		let mut bytes = [0; 4];
		bytes[..BYTES].copy_from_slice(&texel[opacity.clone()]);
		u32::from_le_bytes(bytes)
	};
	// Counts the texels of `level` whose opacity is at or above `threshold`.
	let passing = |level: &[u8], threshold: u32| -> usize {
		level
			.chunks_exact(texel_bytes)
			.filter(|texel| sample(texel) >= threshold)
			.count()
	};
	let base_texels = base_level.len() / texel_bytes;
	let share = passing(base_level, cutoff) as f64 / base_texels as f64;

	let mut remaining = lower_levels;
	for (level_width, level_height) in mip_extents(width, height).skip(1) {
		let (level, rest) =
			std::mem::take(&mut remaining).split_at_mut(level_width as usize * level_height as usize * texel_bytes);
		remaining = rest;
		let target = (share * (level.len() / texel_bytes) as f64).round() as usize;

		// The passing count falls as the threshold rises, so the smallest threshold whose count is at most the target
		// is found by bisection over 1 to max + 1. Zero is never a candidate: no factor makes a zero opacity pass.
		let (mut low, mut high) = (1, max + 1);
		while low < high {
			let middle = low + (high - low) / 2;
			if passing(level, middle) <= target {
				high = middle;
			} else {
				low = middle + 1;
			}
		}
		// The threshold below it passes more than the target; keep whichever count is closer.
		let short = target - passing(level, low);
		let threshold = if low > 1 && passing(level, low - 1) - target < short {
			low - 1
		} else {
			low
		};

		// Values at or above the threshold land at or above the cutoff, and the rest below it, whatever the rounding.
		let factor = cutoff as f32 / threshold as f32;
		for texel in level.chunks_exact_mut(texel_bytes) {
			let value = sample(texel);
			let scaled = ((value as f32 * factor).round() as u32).min(max);
			let scaled = if value >= threshold {
				scaled.max(cutoff)
			} else {
				scaled.min(cutoff - 1)
			};
			texel[opacity.clone()].copy_from_slice(&scaled.to_le_bytes()[..BYTES]);
		}
	}
}

// The kernels below box-filter each 2x2 footprint into one texel. A level halves each side and floors it, so a
// footprint's second column and row exist unless that side is one texel, where the only column or row repeats.
//
// They're written as plain loops over `#[inline(always)]` helpers on purpose: closures and iterator adapters such as
// `map(..).sum()` in these loops were left as out-of-line calls in optimized builds, which made filtering several
// times slower.

/// Returns the two source rows that destination row `y` filters.
#[inline(always)]
fn footprint_rows(source: &[u8], row_bytes: usize, source_height: usize, y: usize) -> (&[u8], &[u8]) {
	let y0 = y * 2;
	let y1 = (y0 + 1).min(source_height - 1);
	(&source[y0 * row_bytes..][..row_bytes], &source[y1 * row_bytes..][..row_bytes])
}

/// Downsamples sRGB RGB channels in linear light while keeping alpha linear.
fn downsample_rgba8_srgb(source_width: u32, source_height: u32, source: &[u8], destination: &mut [u8]) {
	let (source_width, source_height) = (source_width as usize, source_height as usize);
	let destination_width = (source_width / 2).max(1);
	// The column after a footprint's first one, or the same column in a one-texel-wide level.
	let next_column = usize::from(source_width > 1) * 4;
	let to_linear = &SRGB8_TO_LINEAR;
	let thresholds = &LINEAR_TO_SRGB8_THRESHOLDS;

	for (y, destination_row) in destination.chunks_exact_mut(destination_width * 4).enumerate() {
		let (row0, row1) = footprint_rows(source, source_width * 4, source_height, y);
		for (x, texel) in destination_row.chunks_exact_mut(4).enumerate() {
			let at = x * 8;
			let (a, b) = (&row0[at..at + 4], &row0[at + next_column..at + next_column + 4]);
			let (c, d) = (&row1[at..at + 4], &row1[at + next_column..at + next_column + 4]);
			for channel in 0..3 {
				// Summed left to right, as the reference filter does, so the averages are bit-identical to it.
				let sum = to_linear[usize::from(a[channel])]
					+ to_linear[usize::from(b[channel])]
					+ to_linear[usize::from(c[channel])]
					+ to_linear[usize::from(d[channel])];
				texel[channel] = encode_srgb8(sum * 0.25, thresholds);
			}
			let alpha_sum = u16::from(a[3]) + u16::from(b[3]) + u16::from(c[3]) + u16::from(d[3]);
			texel[3] = ((alpha_sum + 2) / 4) as u8;
		}
	}
}

/// Holds, at index `code`, the linear value [`srgb_to_linear`] decodes the 8-bit sRGB `code` to.
///
/// The values are generated, because `powf` can't run in a `const`; the
/// `srgb8_decode_table_matches_the_transfer_function` test checks each one bit for bit against [`srgb_to_linear`].
#[rustfmt::skip]
const SRGB8_TO_LINEAR: [f32; 256] = [
	0.0, 0.000303527, 0.000607054, 0.000910581, 0.001214108, 0.001517635, 0.001821162, 0.0021246888,
	0.002428216, 0.002731743, 0.00303527, 0.0033465356, 0.003676507, 0.004024717, 0.004391442, 0.0047769533,
	0.005181517, 0.0056053917, 0.0060488326, 0.006512091, 0.00699541, 0.0074990317, 0.008023192, 0.008568125,
	0.009134057, 0.009721218, 0.010329823, 0.010960094, 0.011612245, 0.012286487, 0.012983031, 0.013702081,
	0.014443844, 0.015208514, 0.015996292, 0.016807375, 0.017641952, 0.018500218, 0.019382361, 0.020288562,
	0.02121901, 0.022173883, 0.023153365, 0.02415763, 0.025186857, 0.026241222, 0.027320892, 0.028426038,
	0.029556833, 0.03071344, 0.03189603, 0.033104762, 0.034339808, 0.035601314, 0.036889445, 0.038204364,
	0.039546236, 0.0409152, 0.04231141, 0.043735027, 0.045186203, 0.046665084, 0.048171822, 0.049706563,
	0.051269468, 0.052860655, 0.05448028, 0.056128494, 0.057805434, 0.05951124, 0.06124607, 0.06301004,
	0.06480328, 0.06662595, 0.06847818, 0.07036011, 0.07227186, 0.07421358, 0.07618539, 0.07818743,
	0.08021983, 0.082282715, 0.084376216, 0.086500466, 0.088655606, 0.09084173, 0.09305898, 0.095307484,
	0.09758736, 0.09989874, 0.10224175, 0.10461649, 0.10702311, 0.10946172, 0.111932434, 0.11443538,
	0.11697067, 0.119538434, 0.1221388, 0.12477184, 0.1274377, 0.13013649, 0.13286833, 0.13563335,
	0.13843162, 0.1412633, 0.14412849, 0.14702728, 0.1499598, 0.15292616, 0.15592647, 0.15896086,
	0.1620294, 0.16513222, 0.1682694, 0.1714411, 0.17464739, 0.17788841, 0.18116423, 0.18447499,
	0.18782076, 0.19120167, 0.19461781, 0.1980693, 0.20155624, 0.2050787, 0.20863685, 0.21223073,
	0.21586053, 0.21952623, 0.22322798, 0.22696589, 0.23074007, 0.23455065, 0.23839766, 0.2422812,
	0.2462014, 0.25015837, 0.25415218, 0.2581829, 0.26225072, 0.26635566, 0.27049786, 0.27467737,
	0.27889434, 0.2831488, 0.2874409, 0.2917707, 0.29613832, 0.30054384, 0.30498737, 0.30946895,
	0.31398875, 0.31854683, 0.32314324, 0.32777813, 0.33245158, 0.33716366, 0.34191445, 0.3467041,
	0.3515327, 0.35640025, 0.36130688, 0.3662527, 0.37123778, 0.37626222, 0.3813261, 0.38642952,
	0.39157256, 0.3967553, 0.40197787, 0.4072403, 0.4125427, 0.41788515, 0.42326775, 0.42869055,
	0.4341537, 0.43965724, 0.44520125, 0.45078585, 0.4564111, 0.46207705, 0.46778384, 0.47353154,
	0.47932023, 0.48514998, 0.4910209, 0.49693304, 0.5028866, 0.50888145, 0.5149178, 0.5209957,
	0.5271152, 0.5332765, 0.5394796, 0.5457246, 0.5520115, 0.5583405, 0.56471163, 0.5711249,
	0.5775805, 0.5840785, 0.5906189, 0.5972019, 0.6038274, 0.6104956, 0.61720663, 0.62396044,
	0.6307572, 0.63759696, 0.64447975, 0.6514057, 0.65837485, 0.66538733, 0.6724432, 0.67954254,
	0.68668544, 0.6938719, 0.701102, 0.70837593, 0.71569365, 0.72305524, 0.7304609, 0.73791057,
	0.74540436, 0.7529423, 0.76052463, 0.7681513, 0.77582234, 0.7835379, 0.79129803, 0.79910284,
	0.80695236, 0.8148467, 0.82278585, 0.83076996, 0.8387991, 0.8468733, 0.8549927, 0.8631573,
	0.8713672, 0.87962234, 0.8879232, 0.8962694, 0.90466136, 0.9130987, 0.92158204, 0.9301109,
	0.9386859, 0.9473066, 0.9559735, 0.9646863, 0.9734455, 0.9822506, 0.9911022, 1.0,
];

/// Holds, at index `code - 1`, the smallest linear value that [`linear_to_srgb_u8`] encodes to `code` or above, and
/// infinity in the last slot, so [`encode_srgb8`] can encode by search instead of evaluating `powf`.
///
/// The values are generated by bisecting the bit patterns of non-negative `f32` values, which order like the values,
/// against the reference encoder itself. The `srgb8_search_encodes_exactly_like_the_reference` test checks that the
/// search returns exactly what the reference would.
#[rustfmt::skip]
const LINEAR_TO_SRGB8_THRESHOLDS: [f32; 256] = [
	0.0001517635, 0.0004552905, 0.0007588175, 0.0010623444, 0.0013658715, 0.0016693984, 0.0019729254, 0.0022764525,
	0.0025799794, 0.0028835065, 0.003188301, 0.0035092593, 0.0038483152, 0.0042057484, 0.004581833, 0.004976838,
	0.0053910245, 0.0058246506, 0.006277969, 0.006751227, 0.007244668, 0.00775853, 0.008293048, 0.008848452,
	0.009424971, 0.010022826, 0.010642237, 0.011283421, 0.011946592, 0.012631958, 0.01333973, 0.014070112,
	0.014823305, 0.015599506, 0.016398912, 0.017221719, 0.018068116, 0.018938296, 0.019832447, 0.020750748,
	0.021693386, 0.02266054, 0.023652393, 0.024669116, 0.025710888, 0.026777888, 0.027870275, 0.028988224,
	0.030131904, 0.031301484, 0.03249713, 0.033718996, 0.034967247, 0.036242045, 0.037543554, 0.038871925,
	0.04022733, 0.0416099, 0.043019794, 0.04445717, 0.045922183, 0.047414973, 0.048935693, 0.05048449,
	0.052061513, 0.0536669, 0.055300806, 0.05696336, 0.058654718, 0.060375027, 0.062124398, 0.06390299,
	0.06571093, 0.067548364, 0.06941543, 0.07131225, 0.07323897, 0.075195715, 0.07718262, 0.07919983,
	0.08124745, 0.083325624, 0.08543449, 0.08757418, 0.089744784, 0.09194645, 0.09417932, 0.09644349,
	0.0987391, 0.101066284, 0.103425145, 0.10581581, 0.10823841, 0.11069305, 0.11317987, 0.11569897,
	0.11825048, 0.120834544, 0.12345122, 0.12610066, 0.12878297, 0.13149828, 0.13424669, 0.13702832,
	0.13984329, 0.1426917, 0.14557368, 0.14848931, 0.15143874, 0.15442206, 0.1574394, 0.16049086,
	0.16357651, 0.16669649, 0.16985093, 0.17303991, 0.17626356, 0.179522, 0.18281527, 0.18614352,
	0.18950683, 0.19290534, 0.19633915, 0.19980834, 0.20331304, 0.20685333, 0.21042933, 0.21404113,
	0.21768883, 0.22137254, 0.22509237, 0.22884846, 0.2326408, 0.23646954, 0.2403348, 0.24423666,
	0.24817523, 0.2521506, 0.25616288, 0.26021215, 0.2642985, 0.26842207, 0.2725829, 0.2767811,
	0.28101683, 0.2852901, 0.28960103, 0.29394972, 0.2983363, 0.3027608, 0.30722335, 0.31172404,
	0.31626296, 0.32084018, 0.3254558, 0.33010998, 0.33480272, 0.33953416, 0.34430435, 0.3491135,
	0.35396153, 0.3588486, 0.36377484, 0.36874026, 0.373745, 0.37878916, 0.3838728, 0.38899603,
	0.3941589, 0.39936155, 0.40460402, 0.40988642, 0.41520885, 0.42057136, 0.42597404, 0.43141702,
	0.43690035, 0.44242412, 0.4479884, 0.4535933, 0.4592389, 0.46492526, 0.4706525, 0.47642067,
	0.4822299, 0.4880802, 0.49397182, 0.4999046, 0.50587875, 0.51189435, 0.5179515, 0.5240502,
	0.5301906, 0.5363728, 0.54259676, 0.5488627, 0.55517066, 0.5615207, 0.56791294, 0.5743474,
	0.58082414, 0.58734334, 0.59390503, 0.6005092, 0.6071561, 0.6138457, 0.6205781, 0.62735337,
	0.6341716, 0.6410329, 0.64793724, 0.6548848, 0.6618756, 0.6689098, 0.6759875, 0.6831085,
	0.69027317, 0.69748145, 0.70473343, 0.7120292, 0.7193689, 0.72675246, 0.7341801, 0.74165183,
	0.74916774, 0.7567279, 0.7643323, 0.7719812, 0.7796745, 0.7874123, 0.79519475, 0.8030219,
	0.81089383, 0.8188105, 0.8267722, 0.8347788, 0.8428305, 0.85092723, 0.8590692, 0.86725646,
	0.87548906, 0.8837672, 0.8920905, 0.90045965, 0.90887415, 0.9173346, 0.92584056, 0.93439263,
	0.9429903, 0.9516342, 0.9603239, 0.96906006, 0.97784203, 0.98667055, 0.99554527, f32::INFINITY,
];

/// Encodes a linear value in 0 to 1 as an 8-bit sRGB code, exactly as [`linear_to_srgb_u8`] does.
///
/// The code is the number of thresholds at or below `value`, found with eight branch-free halving steps.
#[inline(always)]
fn encode_srgb8(value: f32, thresholds: &[f32; 256]) -> u8 {
	let mut code = 0;
	for step in [128, 64, 32, 16, 8, 4, 2, 1] {
		code += step * usize::from(thresholds[code + step - 1] <= value);
	}
	code as u8
}

/// Encodes a linear value as an 8-bit sRGB code. It's the reference [`encode_srgb8`] matches.
#[cfg(test)]
fn linear_to_srgb_u8(value: f32) -> u8 {
	(linear_to_srgb(value).clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Reads one texel of `CHANNELS` little-endian samples, each `BYTES` wide, into SIMD lanes, leaving the rest zero.
#[inline(always)]
fn load_unorm<const CHANNELS: usize, const BYTES: usize>(texel: &[u8]) -> Simd<u32, 4> {
	let mut lanes = [0_u32; 4];
	for channel in 0..CHANNELS {
		lanes[channel] = if BYTES == 1 {
			u32::from(texel[channel])
		} else {
			u32::from(u16::from_le_bytes([texel[channel * 2], texel[channel * 2 + 1]]))
		};
	}
	Simd::from_array(lanes)
}

/// Downsamples a level of `CHANNELS` unsigned-normalized samples, each `BYTES` little-endian bytes wide, with SIMD
/// lane arithmetic for channel averaging.
fn downsample_unorm<const CHANNELS: usize, const BYTES: usize>(
	source_width: u32,
	source_height: u32,
	source: &[u8],
	destination: &mut [u8],
) {
	debug_assert!(CHANNELS > 0 && CHANNELS <= 4 && BYTES <= 2);

	let (source_width, source_height) = (source_width as usize, source_height as usize);
	let destination_width = (source_width / 2).max(1);
	let texel_bytes = CHANNELS * BYTES;
	// The column after a footprint's first one, or the same column in a one-texel-wide level.
	let next_column = usize::from(source_width > 1) * texel_bytes;

	for (y, destination_row) in destination.chunks_exact_mut(destination_width * texel_bytes).enumerate() {
		let (row0, row1) = footprint_rows(source, source_width * texel_bytes, source_height, y);
		for (x, texel) in destination_row.chunks_exact_mut(texel_bytes).enumerate() {
			let at = x * 2 * texel_bytes;
			let a = load_unorm::<CHANNELS, BYTES>(&row0[at..]);
			let b = load_unorm::<CHANNELS, BYTES>(&row0[at + next_column..]);
			let c = load_unorm::<CHANNELS, BYTES>(&row1[at..]);
			let d = load_unorm::<CHANNELS, BYTES>(&row1[at + next_column..]);
			let average = ((a + b + c + d + Simd::splat(2)) >> Simd::splat(2)).to_array();
			// Every average fits in `BYTES` bytes, so its low little-endian bytes are the stored sample.
			for channel in 0..CHANNELS {
				texel[channel * BYTES..][..BYTES].copy_from_slice(&average[channel].to_le_bytes()[..BYTES]);
			}
		}
	}
}

/// Reads one texel of `CHANNELS` little-endian half-float samples into `f32` lanes, leaving the rest zero.
#[inline(always)]
fn load_f16<const CHANNELS: usize>(texel: &[u8]) -> Simd<f32, 4> {
	let mut lanes = [0.0_f32; 4];
	for channel in 0..CHANNELS {
		lanes[channel] = f16::from_le_bytes([texel[channel * 2], texel[channel * 2 + 1]]).to_f32();
	}
	Simd::from_array(lanes)
}

/// Downsamples a level of `CHANNELS` little-endian half-float samples, averaging their linear values in `f32`.
///
/// Half-float images hold linear values, often outside 0 to 1, so they average without a transfer function or clamp.
fn downsample_f16<const CHANNELS: usize>(source_width: u32, source_height: u32, source: &[u8], destination: &mut [u8]) {
	let (source_width, source_height) = (source_width as usize, source_height as usize);
	let destination_width = (source_width / 2).max(1);
	let texel_bytes = CHANNELS * 2;
	let next_column = usize::from(source_width > 1) * texel_bytes;

	for (y, destination_row) in destination.chunks_exact_mut(destination_width * texel_bytes).enumerate() {
		let (row0, row1) = footprint_rows(source, source_width * texel_bytes, source_height, y);
		for (x, texel) in destination_row.chunks_exact_mut(texel_bytes).enumerate() {
			let at = x * 2 * texel_bytes;
			let a = load_f16::<CHANNELS>(&row0[at..]);
			let b = load_f16::<CHANNELS>(&row0[at + next_column..]);
			let c = load_f16::<CHANNELS>(&row1[at..]);
			let d = load_f16::<CHANNELS>(&row1[at + next_column..]);
			let average = ((a + b + c + d) * Simd::splat(0.25)).to_array();
			for channel in 0..CHANNELS {
				texel[channel * 2..][..2].copy_from_slice(&f16::from_f32(average[channel]).to_le_bytes());
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{
		LINEAR_TO_SRGB8_THRESHOLDS, MipGenerationError, SRGB8_TO_LINEAR, encode_srgb8, generate_lower_levels,
		linear_to_srgb_u8, mip_level_count, packed_lower_levels,
	};
	use crate::types::{Formats, Gamma};

	#[test]
	fn srgb8_tables_decode_and_encode_exactly_like_the_transfer_function() {
		for (code, &linear) in SRGB8_TO_LINEAR.iter().enumerate() {
			let expected = utils::color::srgb_to_linear(code as f32 / 255.0);
			assert_eq!(linear.to_bits(), expected.to_bits(), "sRGB code {code}");
		}

		let thresholds = &LINEAR_TO_SRGB8_THRESHOLDS;
		// Every code boundary and the value just below it, where an inexact table would disagree first.
		let boundaries = thresholds[..255]
			.iter()
			.flat_map(|&threshold| [threshold, f32::from_bits(threshold.to_bits().saturating_sub(1))]);
		let sweep = (0..=1 << 16).map(|step| step as f32 / (1 << 16) as f32);

		for value in boundaries.chain(sweep) {
			assert_eq!(
				encode_srgb8(value, thresholds),
				linear_to_srgb_u8(value),
				"linear value {value:e}"
			);
		}
	}

	#[test]
	fn generates_8_and_16_bit_chains_with_odd_extents_like_the_scalar_filter() {
		let data = create_rgba8_pattern(5, 3);
		let generated = generate_mip_chain(Formats::RGBA8, 5, 3, &data).expect("mips must generate");
		assert_eq!(generated, scalar_mip_chain::<4, 1>(5, 3, &data));
		assert_eq!(mip_level_count(17, 9), Ok(5));

		let data = create_rgba16_pattern(3, 4);
		let generated = generate_mip_chain(Formats::RGBA16, 3, 4, &data).expect("16-bit mips must generate");
		assert_eq!(generated, scalar_mip_chain::<4, 2>(3, 4, &data));
	}

	#[test]
	fn srgb_mips_average_rgb_in_linear_light_and_alpha_linearly() {
		let source = [0, 0, 0, 0, 255, 255, 255, 64, 0, 0, 0, 128, 255, 255, 255, 255];
		// A 2x2 level has one 1x1 level below it.
		let (_, srgb) =
			generate_lower_levels(Formats::RGBA8, Gamma::SRGB, 2, 2, &source).expect("sRGB mip generation should succeed");
		let (_, linear) =
			generate_lower_levels(Formats::RGBA8, Gamma::Linear, 2, 2, &source).expect("linear mip generation should succeed");

		assert_eq!(srgb, [188, 188, 188, 112]);
		assert_eq!(linear, [128, 128, 128, 112]);
	}

	/// Generates the complete linear chain, base level first, as (width, height, texels) the way consumers store it.
	fn generate_mip_chain(
		format: Formats,
		width: u32,
		height: u32,
		base_level: &[u8],
	) -> Result<Vec<(u32, u32, Vec<u8>)>, MipGenerationError> {
		let (bytes_per_pixel, lower) = generate_lower_levels(format, Gamma::Linear, width, height, base_level)?;

		Ok(std::iter::once((width, height, base_level.to_vec()))
			.chain(
				packed_lower_levels(width, height, bytes_per_pixel, &lower)
					.map(|level| (level.width, level.height, level.data.to_vec())),
			)
			.collect())
	}

	fn create_rgba8_pattern(width: u32, height: u32) -> Vec<u8> {
		let mut data = vec![0_u8; width as usize * height as usize * 4];

		for y in 0..height {
			for x in 0..width {
				let index = (y as usize * width as usize + x as usize) * 4;
				data[index] = ((x * 31 + y * 7 + 3) & 0xFF) as u8;
				data[index + 1] = ((x * 11 + y * 17 + 19) & 0xFF) as u8;
				data[index + 2] = ((x * 5 + y * 23 + 47) & 0xFF) as u8;
				data[index + 3] = ((x * 13 + y * 29 + 61) & 0xFF) as u8;
			}
		}

		data
	}

	fn create_rgba16_pattern(width: u32, height: u32) -> Vec<u8> {
		let mut data = vec![0_u8; width as usize * height as usize * 8];

		for y in 0..height {
			for x in 0..width {
				let pixel = y as usize * width as usize + x as usize;
				for channel in 0..4 {
					let value = (((x as usize + 1) * 997) + ((y as usize + 1) * 557) + ((channel + 1) * 313)) as u16;
					let offset = pixel * 8 + channel * 2;
					data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
				}
			}
		}

		data
	}

	/// Filters every level one texel and channel at a time, as the reference the SIMD filter must match.
	///
	/// Each texel holds `CHANNELS` little-endian samples of `BYTES` bytes.
	fn scalar_mip_chain<const CHANNELS: usize, const BYTES: usize>(
		width: u32,
		height: u32,
		base_level: &[u8],
	) -> Vec<(u32, u32, Vec<u8>)> {
		let sample = |data: &[u8], offset: usize| {
			let mut bytes = [0_u8; 4];
			bytes[..BYTES].copy_from_slice(&data[offset..offset + BYTES]);
			u32::from_le_bytes(bytes)
		};
		let mut levels = vec![(width, height, base_level.to_vec())];

		while let Some((current_width, current_height, current_data)) =
			levels.last().filter(|(width, height, _)| *width > 1 || *height > 1)
		{
			let (next_width, next_height) = ((current_width / 2).max(1), (current_height / 2).max(1));
			let (current_width, current_height) = (*current_width as usize, *current_height as usize);
			let mut next_data = vec![0_u8; next_width as usize * next_height as usize * CHANNELS * BYTES];

			for y in 0..next_height as usize {
				let y0 = (y * 2).min(current_height - 1);
				let y1 = (y0 + 1).min(current_height - 1);

				for x in 0..next_width as usize {
					let x0 = (x * 2).min(current_width - 1);
					let x1 = (x0 + 1).min(current_width - 1);
					let destination = (y * next_width as usize + x) * CHANNELS * BYTES;

					for channel in (0..CHANNELS * BYTES).step_by(BYTES) {
						let sum = [(y0, x0), (y0, x1), (y1, x0), (y1, x1)]
							.into_iter()
							.map(|(y, x)| sample(current_data, (y * current_width + x) * CHANNELS * BYTES + channel))
							.sum::<u32>();
						let value = (sum + 2) / 4;
						next_data[destination + channel..destination + channel + BYTES]
							.copy_from_slice(&value.to_le_bytes()[..BYTES]);
					}
				}
			}

			levels.push((next_width, next_height, next_data));
		}

		levels
	}
}
