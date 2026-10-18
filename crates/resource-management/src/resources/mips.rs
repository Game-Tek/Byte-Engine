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

/// The `MipGenerator` enum selects where the image processor filters and block-compresses texture mip chains.
///
/// An asset manager's [`crate::processors::image::ImageProcessor`] owns one generator and calls
/// [`Self::encode_mip_chain`] once per image whose [`crate::processors::image::ImageProcessorSettings::mips`] is set,
/// after sizing the stored chain. `Cpu` filters and encodes on the calling thread. `Gpu` submits to the offline GPU worker and falls back to the CPU
/// path for requests it can't serve or whose GPU work fails.
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

impl MipGenerator {
	/// Filters `base_level` down to one texel and writes every level, base level first, into `output` encoded as
	/// `output_format`.
	///
	/// `base_level` holds `width` by `height` texels in the [`filtering_format`] of `output_format`. `output` holds
	/// exactly [`encoded_mip_chain_size`] bytes, and each level directly follows the one before it. The GPU path awaits
	/// its worker without blocking the calling runtime thread.
	pub async fn encode_mip_chain(
		&self,
		output_format: Formats,
		gamma: Gamma,
		width: u32,
		height: u32,
		base_level: &[u8],
		output: &mut [u8],
	) -> Result<(), MipGenerationError> {
		#[cfg(feature = "gpu-mips")]
		if let Self::Gpu(generator) = self {
			match generator
				.encode_mip_chain(output_format, gamma, width, height, base_level, output)
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

		let (bytes_per_pixel, lower_levels) =
			generate_lower_levels(filtering_format(output_format), gamma, width, height, base_level)?;
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
	/// base level needs to be written.
	pub async fn filter_mip_chain_in_place(
		&self,
		format: Formats,
		gamma: Gamma,
		width: u32,
		height: u32,
		chain: &mut [u8],
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

		#[cfg(feature = "gpu-mips")]
		if let Self::Gpu(generator) = self {
			match generator.filter_mip_chain_in_place(format, gamma, width, height, chain).await {
				Ok(()) => return Ok(()),
				// Requests outside the GPU path's formats are expected and take the CPU path quietly.
				Err(gpu::GPUMipError::UnsupportedRequest) => {}
				Err(error) => log::warn!(
					"GPU texture mip generation failed; using the CPU fallback. The most likely cause is an unavailable or unsupported GPU path. Error: {error}"
				),
			}
		}

		let bytes_per_pixel = format.texel_bytes().ok_or(MipGenerationError::UnsupportedFormat(format))?;
		let (base_level, lower_levels) = chain.split_at_mut(width as usize * height as usize * bytes_per_pixel);
		filter_lower_levels(format, gamma, width, height, base_level, lower_levels)
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
/// Packed RG16 filters as RGBA16. Every other format, including the half-float ones, filters in place. Convert the base level to this format before
/// passing it to [`MipGenerator::encode_mip_chain`].
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
/// Size the output of [`MipGenerator::encode_mip_chain`] with it.
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
	let to_linear = &*SRGB8_TO_LINEAR;
	let thresholds = &*LINEAR_TO_SRGB8_THRESHOLDS;

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

/// Each 8-bit sRGB code always decodes to the same linear value, so the 256 codes are decoded once per process.
static SRGB8_TO_LINEAR: std::sync::LazyLock<[f32; 256]> =
	std::sync::LazyLock::new(|| std::array::from_fn(|code| srgb_to_linear(code as f32 / 255.0)));

/// Holds, at index `code - 1`, the smallest linear value that [`linear_to_srgb_u8`] encodes to `code` or above, and
/// infinity in the last slot, so [`encode_srgb8`] can encode by search instead of evaluating `powf`.
///
/// Each threshold is found by bisecting the bit patterns of non-negative `f32` values, which order like the values,
/// against the reference encoder itself, so the search returns exactly what the reference would.
static LINEAR_TO_SRGB8_THRESHOLDS: std::sync::LazyLock<[f32; 256]> = std::sync::LazyLock::new(|| {
	std::array::from_fn(|index| {
		let code = index + 1;
		if code > 255 {
			return f32::INFINITY;
		}
		let (mut low, mut high) = (0_u32, 1.0_f32.to_bits());
		while low < high {
			let middle = low + (high - low) / 2;
			if usize::from(linear_to_srgb_u8(f32::from_bits(middle))) >= code {
				high = middle;
			} else {
				low = middle + 1;
			}
		}
		f32::from_bits(low)
	})
});

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
		LINEAR_TO_SRGB8_THRESHOLDS, MipGenerationError, encode_srgb8, generate_lower_levels, linear_to_srgb_u8,
		mip_level_count, packed_lower_levels,
	};
	use crate::types::{Formats, Gamma};

	#[test]
	fn srgb8_search_encodes_exactly_like_the_reference() {
		let thresholds = &*LINEAR_TO_SRGB8_THRESHOLDS;
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
	fn generates_rgba8_chain_with_odd_extent() {
		let width = 5_u32;
		let height = 3_u32;
		let data = create_rgba8_pattern(width, height);

		let generated = generate_mip_chain(Formats::RGBA8, width, height, &data).expect("mips must generate");

		assert_eq!(generated, scalar_mip_chain::<4, 1>(width, height, &data));
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

	#[test]
	fn generates_rgba16_chain() {
		let width = 3_u32;
		let height = 4_u32;
		let data = create_rgba16_pattern(width, height);

		let generated = generate_mip_chain(Formats::RGBA16, width, height, &data).expect("16-bit mips must generate");

		assert_eq!(generated, scalar_mip_chain::<4, 2>(width, height, &data));
	}

	#[test]
	fn counts_mip_levels() {
		let count = mip_level_count(17, 9).expect("valid size");

		assert_eq!(count, 5);
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
