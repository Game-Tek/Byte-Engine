use std::{alloc::Global, error::Error, fmt, simd::Simd};

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

/// The `MipGenerator` enum selects where material importers filter and block-compress texture mip chains.
///
/// Material importers share one generator and call [`Self::encode_mip_chain`] once per material texture, after
/// [`crate::processors::image::process_image_in`] sized the stored chain.
/// `Cpu` filters and encodes on the calling thread. `Gpu` submits to the offline GPU worker and falls back to the CPU
/// path for requests it can't serve or whose GPU work fails.
pub enum MipGenerator {
	/// Filters and encodes every level on the CPU.
	///
	/// Keep this path even though material importers normally use the GPU: it bakes wherever GPU setup or a GPU request
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
/// Packed RG16 filters as RGBA16. Every other format filters in place. Convert the base level to this format before
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
	let bytes_per_pixel = bytes_per_pixel(format).ok_or(MipGenerationError::UnsupportedFormat(format))?;
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
	// Each level filters the one before it, which the previous iteration just wrote.
	let (mut source, mut remaining) = (base_level, data.as_mut_slice());
	for ((source_width, source_height), (level_width, level_height)) in
		mip_extents(width, height).zip(mip_extents(width, height).skip(1))
	{
		let (destination, rest) =
			std::mem::take(&mut remaining).split_at_mut(level_width as usize * level_height as usize * bytes_per_pixel);
		downsample_level(format, gamma, source_width, source_height, source, destination)?;
		source = destination;
		remaining = rest;
	}

	Ok((bytes_per_pixel, data))
}

/// Returns the texel size of a format the CPU downsampler supports.
fn bytes_per_pixel(format: Formats) -> Option<usize> {
	match format {
		Formats::R16F | Formats::RGBA16F => None,
		format => format.texel_bytes(),
	}
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
		// Packed RG formats filter as their RGBA filtering format and are truncated when each level is encoded.
		_ => return Err(MipGenerationError::UnsupportedFormat(format)),
	}

	Ok(())
}

/// Downsamples sRGB RGB channels in linear light while keeping alpha linear.
fn downsample_rgba8_srgb(source_width: u32, source_height: u32, source: &[u8], destination: &mut [u8]) {
	let source_width = source_width as usize;
	let source_height = source_height as usize;
	let destination_width = (source_width / 2).max(1);
	let destination_height = (source_height / 2).max(1);

	for y in 0..destination_height {
		let y0 = (y * 2).min(source_height - 1);
		let y1 = (y0 + 1).min(source_height - 1);
		for x in 0..destination_width {
			let x0 = (x * 2).min(source_width - 1);
			let x1 = (x0 + 1).min(source_width - 1);
			let sources = [
				(y0 * source_width + x0) * 4,
				(y0 * source_width + x1) * 4,
				(y1 * source_width + x0) * 4,
				(y1 * source_width + x1) * 4,
			];
			let destination_pixel = (y * destination_width + x) * 4;
			for channel in 0..3 {
				let linear_average = sources
					.iter()
					.map(|source_pixel| srgb_u8_to_linear(source[*source_pixel + channel]))
					.sum::<f32>() * 0.25;
				destination[destination_pixel + channel] = linear_to_srgb_u8(linear_average);
			}
			let alpha_sum = sources
				.iter()
				.map(|source_pixel| u16::from(source[*source_pixel + 3]))
				.sum::<u16>();
			destination[destination_pixel + 3] = ((alpha_sum + 2) / 4) as u8;
		}
	}
}

fn srgb_u8_to_linear(value: u8) -> f32 {
	srgb_to_linear(f32::from(value) / 255.0)
}

fn linear_to_srgb_u8(value: f32) -> u8 {
	(linear_to_srgb(value).clamp(0.0, 1.0) * 255.0).round() as u8
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

	let source_width = source_width as usize;
	let source_height = source_height as usize;
	let destination_width = (source_width / 2).max(1);
	let destination_height = (source_height / 2).max(1);
	let texel_bytes = CHANNELS * BYTES;
	// Reads one texel's samples into SIMD lanes, leaving the unused lanes zero.
	let load = |texel: usize| {
		Simd::<u32, 4>::from_array(std::array::from_fn(|channel| {
			let at = texel * texel_bytes + channel * BYTES;
			match (channel < CHANNELS, BYTES) {
				(false, _) => 0,
				(true, 1) => u32::from(source[at]),
				(true, _) => u32::from(u16::from_le_bytes([source[at], source[at + 1]])),
			}
		}))
	};

	for y in 0..destination_height {
		let y0 = (y * 2).min(source_height - 1);
		let y1 = (y0 + 1).min(source_height - 1);

		for x in 0..destination_width {
			let x0 = (x * 2).min(source_width - 1);
			let x1 = (x0 + 1).min(source_width - 1);
			let (a, b) = (load(y0 * source_width + x0), load(y0 * source_width + x1));
			let (c, d) = (load(y1 * source_width + x0), load(y1 * source_width + x1));
			let average = (a + b + c + d + Simd::splat(2)) / Simd::splat(4);
			let destination_pixel = (y * destination_width + x) * texel_bytes;
			// Every average fits in `BYTES` bytes, so its low little-endian bytes are the stored sample.
			for (channel, value) in average.to_array()[..CHANNELS].iter().enumerate() {
				destination[destination_pixel + channel * BYTES..][..BYTES].copy_from_slice(&value.to_le_bytes()[..BYTES]);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{MipGenerationError, generate_lower_levels, mip_level_count, packed_lower_levels};
	use crate::types::{Formats, Gamma};

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
