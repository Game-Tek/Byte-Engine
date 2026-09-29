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
struct MipLevel<'a> {
	width: u32,
	height: u32,
	data: &'a [u8],
}

const MAX_MIP_LEVELS: usize = u32::BITS as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OwnedMipRange {
	width: u32,
	height: u32,
	offset: usize,
	size: usize,
}

impl OwnedMipRange {
	const EMPTY: Self = Self {
		width: 0,
		height: 0,
		offset: 0,
		size: 0,
	};
}

/// The `OwnedMipChain` struct holds the filtered levels below a base level until the CPU backend encodes them.
///
/// The chain uses one allocation for all texels and keeps its bounded level metadata inline.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedMipChain {
	levels: [OwnedMipRange; MAX_MIP_LEVELS],
	level_count: usize,
	data: Box<[u8]>,
}

impl OwnedMipChain {
	/// Returns each generated level in order without allocating view metadata.
	fn levels(&self) -> impl ExactSizeIterator<Item = MipLevel<'_>> + '_ {
		self.levels[..self.level_count].iter().map(|level| MipLevel {
			width: level.width,
			height: level.height,
			data: &self.data[level.offset..level.offset + level.size],
		})
	}
}

/// The `MipGenerationBackend` trait lets material importers move mip filtering and block compression to an accelerator.
///
/// [`crate::processors::processor::implementations::image::process_image_with_mip_backend_in`] allocates the stored
/// chain and calls [`Self::encode_mip_chain`] once per material texture. [`CPUMipGenerationBackend`] does this work on
/// the CPU; the `gpu` module's `MaterialMipGenerator` does it on the GPU.
pub trait MipGenerationBackend: Send + Sync {
	/// Filters `base_level` down to one texel and writes every level, base level first, into `output` encoded as
	/// `output_format`.
	///
	/// `base_level` holds `width` by `height` texels in the [`filtering_format`] of `output_format`. `output` holds
	/// exactly [`encoded_mip_chain_size`] bytes, and each level directly follows the one before it.
	fn encode_mip_chain(
		&self,
		output_format: Formats,
		gamma: Gamma,
		width: u32,
		height: u32,
		base_level: &[u8],
		output: &mut [u8],
	) -> Result<(), MipGenerationError>;
}

/// The `CPUMipGenerationBackend` struct keeps material textures bakeable where the GPU backend can't run.
///
/// Keep it even though material importers normally use the GPU backend: it filters and encodes the chain on the CPU
/// whenever GPU setup or a GPU request fails, it's the backend that bakes and tests use on machines without a
/// compatible GPU, and its `intel_tex_2` encoder is the reference the GPU BC7 encoder's quality is measured against.
pub struct CPUMipGenerationBackend;

impl MipGenerationBackend for CPUMipGenerationBackend {
	fn encode_mip_chain(
		&self,
		output_format: Formats,
		gamma: Gamma,
		width: u32,
		height: u32,
		base_level: &[u8],
		output: &mut [u8],
	) -> Result<(), MipGenerationError> {
		let expected = encoded_mip_chain_size(output_format, Extent::rectangle(width, height))
			.ok_or(MipGenerationError::UnsupportedFormat(output_format))?;
		if output.len() != expected {
			return Err(MipGenerationError::BufferSizeMismatch {
				expected,
				got: output.len(),
			});
		}

		let lower_levels = generate_owned_lower_mip_chain(filtering_format(output_format), gamma, width, height, base_level)?;
		let base_level = MipLevel {
			width,
			height,
			data: base_level,
		};
		let mut remaining = output;
		for level in std::iter::once(base_level).chain(lower_levels.levels()) {
			let extent = Extent::rectangle(level.width, level.height);
			let size =
				encoded_mip_level_size(output_format, extent).expect("A storable chain has a stored size for every level");
			let (destination, rest) = remaining.split_at_mut(size);
			encode_level_in(output_format, extent, level.data, destination, Global);
			remaining = rest;
		}
		Ok(())
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
/// Block-compressed outputs and sRGB RGBA8 filter as RGBA8 texels. Every other format filters in place. Convert the
/// base level to this format before passing it to [`MipGenerationBackend::encode_mip_chain`].
pub fn filtering_format(output_format: Formats) -> Formats {
	match output_format {
		Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB | Formats::RGBA8SRGB => Formats::RGBA8,
		format => format,
	}
}

/// Returns the exact stored size of a complete mip chain encoded as `format`, or `None` for formats images can't be
/// stored in.
///
/// Size the output of [`MipGenerationBackend::encode_mip_chain`] with it.
pub fn encoded_mip_chain_size(format: Formats, extent: Extent) -> Option<usize> {
	mip_extents(extent.width(), extent.height()).try_fold(0usize, |total, (width, height)| {
		total.checked_add(encoded_mip_level_size(format, Extent::rectangle(width, height))?)
	})
}

/// Returns the stored size of one level in a format the image processor can encode.
pub(crate) fn encoded_mip_level_size(format: Formats, extent: Extent) -> Option<usize> {
	match format {
		Formats::BC5
		| Formats::BC5SNORM
		| Formats::BC7
		| Formats::BC7SRGB
		| Formats::RGBA8
		| Formats::RGBA8SRGB
		| Formats::R16F
		| Formats::RGBA16
		| Formats::RGBA16F => format.level_size(extent),
		_ => None,
	}
}

/// Lays out every level below the base level back to back and returns the ranges, their count, and the total size.
fn lower_level_layout(
	width: u32,
	height: u32,
	bytes_per_pixel: usize,
) -> Result<([OwnedMipRange; MAX_MIP_LEVELS], usize, usize), MipGenerationError> {
	let mut levels = [OwnedMipRange::EMPTY; MAX_MIP_LEVELS];
	let mut level_count = 0usize;
	let mut offset = 0usize;
	for (level_width, level_height) in mip_extents(width, height).skip(1) {
		let size = expected_size(level_width, level_height, bytes_per_pixel).ok_or(MipGenerationError::DimensionsTooLarge)?;
		levels[level_count] = OwnedMipRange {
			width: level_width,
			height: level_height,
			offset,
			size,
		};
		offset = offset.checked_add(size).ok_or(MipGenerationError::DimensionsTooLarge)?;
		level_count += 1;
	}
	Ok((levels, level_count, offset))
}

/// Generates packed lower mip levels using one output allocation.
fn generate_owned_lower_mip_chain(
	format: Formats,
	gamma: Gamma,
	width: u32,
	height: u32,
	base_level: &[u8],
) -> Result<OwnedMipChain, MipGenerationError> {
	if width == 0 || height == 0 {
		return Err(MipGenerationError::ZeroDimensions);
	}
	let bytes_per_pixel = bytes_per_pixel(format).ok_or(MipGenerationError::UnsupportedFormat(format))?;
	let expected_base_size = expected_size(width, height, bytes_per_pixel).ok_or(MipGenerationError::DimensionsTooLarge)?;
	if base_level.len() != expected_base_size {
		return Err(MipGenerationError::BufferSizeMismatch {
			expected: expected_base_size,
			got: base_level.len(),
		});
	}
	let (levels, level_count, total_size) = lower_level_layout(width, height, bytes_per_pixel)?;

	let mut data = vec![0_u8; total_size];
	let (mut source_width, mut source_height) = (width, height);
	let mut source = None;
	for level in &levels[..level_count] {
		let (completed, destination) = data.split_at_mut(level.offset);
		// Each level filters the one before it, which the previous iteration just wrote.
		let source_bytes = source
			.map(|range: std::ops::Range<usize>| &completed[range])
			.unwrap_or(base_level);
		downsample_level(
			format,
			gamma,
			source_width,
			source_height,
			source_bytes,
			&mut destination[..level.size],
		)?;
		source = Some(level.offset..level.offset + level.size);
		(source_width, source_height) = (level.width, level.height);
	}

	Ok(OwnedMipChain {
		levels,
		level_count,
		data: data.into_boxed_slice(),
	})
}

/// Returns the texel size of a format the CPU downsampler supports.
fn bytes_per_pixel(format: Formats) -> Option<usize> {
	match format {
		Formats::R16F | Formats::RGBA16F => None,
		format => format.texel_bytes(),
	}
}

fn expected_size(width: u32, height: u32, bytes_per_pixel: usize) -> Option<usize> {
	(width as usize).checked_mul(height as usize)?.checked_mul(bytes_per_pixel)
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
		Formats::RG8 => downsample_u8::<2>(source_width, source_height, source, destination),
		Formats::RGB8 => downsample_u8::<3>(source_width, source_height, source, destination),
		Formats::RGBA8 | Formats::RGBA8SRGB if gamma == Gamma::SRGB => {
			downsample_rgba8_srgb(source_width, source_height, source, destination)
		}
		Formats::RGBA8 | Formats::RGBA8SRGB => downsample_u8::<4>(source_width, source_height, source, destination),
		Formats::RGB16 => downsample_u16::<3>(source_width, source_height, source, destination),
		Formats::RGBA16 => downsample_u16::<4>(source_width, source_height, source, destination),
		Formats::R16F | Formats::RGBA16F | Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB => {
			return Err(MipGenerationError::UnsupportedFormat(format));
		}
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

/// Downsamples an 8-bit format level with SIMD lane arithmetic for channel averaging.
fn downsample_u8<const CHANNELS: usize>(source_width: u32, source_height: u32, source: &[u8], destination: &mut [u8]) {
	debug_assert!(CHANNELS > 0 && CHANNELS <= 4);

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

			let top_left = (y0 * source_width + x0) * CHANNELS;
			let top_right = (y0 * source_width + x1) * CHANNELS;
			let bottom_left = (y1 * source_width + x0) * CHANNELS;
			let bottom_right = (y1 * source_width + x1) * CHANNELS;
			let destination_pixel = (y * destination_width + x) * CHANNELS;

			let a = load_u8_pixel::<CHANNELS>(source, top_left);
			let b = load_u8_pixel::<CHANNELS>(source, top_right);
			let c = load_u8_pixel::<CHANNELS>(source, bottom_left);
			let d = load_u8_pixel::<CHANNELS>(source, bottom_right);
			let average = (a + b + c + d + Simd::splat(2)) / Simd::splat(4);
			let lanes = average.to_array();

			for channel in 0..CHANNELS {
				destination[destination_pixel + channel] = lanes[channel] as u8;
			}
		}
	}
}

/// Downsamples a 16-bit format level with SIMD lane arithmetic for channel averaging.
fn downsample_u16<const CHANNELS: usize>(source_width: u32, source_height: u32, source: &[u8], destination: &mut [u8]) {
	debug_assert!(CHANNELS > 0 && CHANNELS <= 4);

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

			let top_left = (y0 * source_width + x0) * CHANNELS * 2;
			let top_right = (y0 * source_width + x1) * CHANNELS * 2;
			let bottom_left = (y1 * source_width + x0) * CHANNELS * 2;
			let bottom_right = (y1 * source_width + x1) * CHANNELS * 2;
			let destination_pixel = (y * destination_width + x) * CHANNELS * 2;

			let a = load_u16_pixel::<CHANNELS>(source, top_left);
			let b = load_u16_pixel::<CHANNELS>(source, top_right);
			let c = load_u16_pixel::<CHANNELS>(source, bottom_left);
			let d = load_u16_pixel::<CHANNELS>(source, bottom_right);
			let average = (a + b + c + d + Simd::splat(2)) / Simd::splat(4);
			let lanes = average.to_array();

			for channel in 0..CHANNELS {
				let value = lanes[channel] as u16;
				let offset = destination_pixel + channel * 2;
				destination[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
			}
		}
	}
}

fn load_u8_pixel<const CHANNELS: usize>(source: &[u8], offset: usize) -> Simd<u16, 4> {
	let mut lanes = [0_u16; 4];

	for channel in 0..CHANNELS {
		lanes[channel] = source[offset + channel] as u16;
	}

	Simd::from_array(lanes)
}

fn load_u16_pixel<const CHANNELS: usize>(source: &[u8], offset: usize) -> Simd<u32, 4> {
	let mut lanes = [0_u32; 4];

	for channel in 0..CHANNELS {
		let channel_offset = offset + channel * 2;
		let value = u16::from_le_bytes([source[channel_offset], source[channel_offset + 1]]);
		lanes[channel] = value as u32;
	}

	Simd::from_array(lanes)
}

#[cfg(test)]
mod tests {
	use super::{MipGenerationError, generate_owned_lower_mip_chain, mip_level_count};
	use crate::types::{Formats, Gamma};

	#[derive(Debug, Clone, PartialEq, Eq)]
	struct ExpectedMipLevel {
		width: u32,
		height: u32,
		data: Vec<u8>,
	}

	#[test]
	fn generates_rgba8_chain_with_odd_extent() {
		let width = 5_u32;
		let height = 3_u32;
		let data = create_rgba8_pattern(width, height);

		let generated = generate_mip_chain(Formats::RGBA8, width, height, &data).expect("mips must generate");
		let expected = scalar_mip_chain_u8::<4>(width, height, &data);

		assert_chain_matches(&generated, &expected);
	}

	#[test]
	fn srgb_mips_average_rgb_in_linear_light_and_alpha_linearly() {
		let source = [0, 0, 0, 0, 255, 255, 255, 64, 0, 0, 0, 128, 255, 255, 255, 255];
		let srgb = generate_owned_lower_mip_chain(Formats::RGBA8, Gamma::SRGB, 2, 2, &source)
			.expect("sRGB mip generation should succeed");
		let linear = generate_owned_lower_mip_chain(Formats::RGBA8, Gamma::Linear, 2, 2, &source)
			.expect("linear mip generation should succeed");

		assert_eq!(
			srgb.levels().next().expect("1x1 sRGB mip should exist").data,
			[188, 188, 188, 112]
		);
		assert_eq!(
			linear.levels().next().expect("1x1 linear mip should exist").data,
			[128, 128, 128, 112]
		);
	}

	#[test]
	fn generates_rgba16_chain() {
		let width = 3_u32;
		let height = 4_u32;
		let data = create_rgba16_pattern(width, height);

		let generated = generate_mip_chain(Formats::RGBA16, width, height, &data).expect("16-bit mips must generate");
		let expected = scalar_mip_chain_u16::<4>(width, height, &data);

		assert_chain_matches(&generated, &expected);
	}

	#[test]
	fn counts_mip_levels() {
		let count = mip_level_count(17, 9).expect("valid size");

		assert_eq!(count, 5);
	}

	/// Generates the complete linear chain, base level first, the way consumers store it.
	fn generate_mip_chain(
		format: Formats,
		width: u32,
		height: u32,
		base_level: &[u8],
	) -> Result<Vec<ExpectedMipLevel>, MipGenerationError> {
		let lower = generate_owned_lower_mip_chain(format, Gamma::Linear, width, height, base_level)?;
		let base = ExpectedMipLevel {
			width,
			height,
			data: base_level.to_vec(),
		};

		Ok(std::iter::once(base)
			.chain(lower.levels().map(|level| ExpectedMipLevel {
				width: level.width,
				height: level.height,
				data: level.data.to_vec(),
			}))
			.collect())
	}

	fn assert_chain_matches(chain: &[ExpectedMipLevel], expected: &[ExpectedMipLevel]) {
		assert_eq!(chain, expected);
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

	fn scalar_mip_chain_u8<const CHANNELS: usize>(width: u32, height: u32, base_level: &[u8]) -> Vec<ExpectedMipLevel> {
		let mut levels = Vec::new();
		let mut current_width = width;
		let mut current_height = height;
		let mut current_data = base_level.to_vec();

		loop {
			levels.push(ExpectedMipLevel {
				width: current_width,
				height: current_height,
				data: current_data.clone(),
			});

			if current_width == 1 && current_height == 1 {
				break;
			}

			let next_width = (current_width / 2).max(1);
			let next_height = (current_height / 2).max(1);
			let mut next_data = vec![0_u8; next_width as usize * next_height as usize * CHANNELS];

			for y in 0..next_height as usize {
				let y0 = (y * 2).min(current_height as usize - 1);
				let y1 = (y0 + 1).min(current_height as usize - 1);

				for x in 0..next_width as usize {
					let x0 = (x * 2).min(current_width as usize - 1);
					let x1 = (x0 + 1).min(current_width as usize - 1);

					let p00 = (y0 * current_width as usize + x0) * CHANNELS;
					let p10 = (y0 * current_width as usize + x1) * CHANNELS;
					let p01 = (y1 * current_width as usize + x0) * CHANNELS;
					let p11 = (y1 * current_width as usize + x1) * CHANNELS;
					let destination = (y * next_width as usize + x) * CHANNELS;

					for channel in 0..CHANNELS {
						let sum = current_data[p00 + channel] as u16
							+ current_data[p10 + channel] as u16
							+ current_data[p01 + channel] as u16
							+ current_data[p11 + channel] as u16;
						next_data[destination + channel] = ((sum + 2) / 4) as u8;
					}
				}
			}

			current_width = next_width;
			current_height = next_height;
			current_data = next_data;
		}

		levels
	}

	fn scalar_mip_chain_u16<const CHANNELS: usize>(width: u32, height: u32, base_level: &[u8]) -> Vec<ExpectedMipLevel> {
		let mut levels = Vec::new();
		let mut current_width = width;
		let mut current_height = height;
		let mut current_data = base_level.to_vec();

		loop {
			levels.push(ExpectedMipLevel {
				width: current_width,
				height: current_height,
				data: current_data.clone(),
			});

			if current_width == 1 && current_height == 1 {
				break;
			}

			let next_width = (current_width / 2).max(1);
			let next_height = (current_height / 2).max(1);
			let mut next_data = vec![0_u8; next_width as usize * next_height as usize * CHANNELS * 2];

			for y in 0..next_height as usize {
				let y0 = (y * 2).min(current_height as usize - 1);
				let y1 = (y0 + 1).min(current_height as usize - 1);

				for x in 0..next_width as usize {
					let x0 = (x * 2).min(current_width as usize - 1);
					let x1 = (x0 + 1).min(current_width as usize - 1);

					let p00 = (y0 * current_width as usize + x0) * CHANNELS * 2;
					let p10 = (y0 * current_width as usize + x1) * CHANNELS * 2;
					let p01 = (y1 * current_width as usize + x0) * CHANNELS * 2;
					let p11 = (y1 * current_width as usize + x1) * CHANNELS * 2;
					let destination = (y * next_width as usize + x) * CHANNELS * 2;

					for channel in 0..CHANNELS {
						let c = channel * 2;
						let s00 = u16::from_le_bytes([current_data[p00 + c], current_data[p00 + c + 1]]) as u32;
						let s10 = u16::from_le_bytes([current_data[p10 + c], current_data[p10 + c + 1]]) as u32;
						let s01 = u16::from_le_bytes([current_data[p01 + c], current_data[p01 + c + 1]]) as u32;
						let s11 = u16::from_le_bytes([current_data[p11 + c], current_data[p11 + c + 1]]) as u32;
						let value = ((s00 + s10 + s01 + s11 + 2) / 4) as u16;
						next_data[destination + c..destination + c + 2].copy_from_slice(&value.to_le_bytes());
					}
				}
			}

			current_width = next_width;
			current_height = next_height;
			current_data = next_data;
		}

		levels
	}
}
