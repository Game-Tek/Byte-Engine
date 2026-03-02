use std::{borrow::Cow, error::Error, fmt, simd::Simd};

use crate::types::Formats;

/// The `MipLevel` struct exposes one mip level so upload code can consume borrowed image data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MipLevel<'a> {
	pub width: u32,
	pub height: u32,
	pub data: &'a [u8],
}

/// The `MipChain` struct owns generated levels while exposing each level as a borrowed slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MipChain<'a> {
	levels: Vec<StoredMipLevel<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredMipLevel<'a> {
	width: u32,
	height: u32,
	data: Cow<'a, [u8]>,
}

impl<'a> StoredMipLevel<'a> {
	fn as_borrowed(&self) -> MipLevel<'_> {
		MipLevel {
			width: self.width,
			height: self.height,
			data: self.data.as_ref(),
		}
	}
}

impl<'a> MipChain<'a> {
	pub fn len(&self) -> usize {
		self.levels.len()
	}

	pub fn is_empty(&self) -> bool {
		self.levels.is_empty()
	}

	pub fn level(&self, index: usize) -> Option<MipLevel<'_>> {
		self.levels.get(index).map(StoredMipLevel::as_borrowed)
	}

	pub fn levels(&self) -> impl ExactSizeIterator<Item = MipLevel<'_>> + '_ {
		self.levels.iter().map(StoredMipLevel::as_borrowed)
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

	let mut levels = 1_u32;
	let mut current_width = width;
	let mut current_height = height;

	while current_width > 1 || current_height > 1 {
		current_width = (current_width / 2).max(1);
		current_height = (current_height / 2).max(1);
		levels += 1;
	}

	Ok(levels)
}

/// Generates a full mip chain, including the base level, using a 2x2 box filter.
pub fn generate_mip_chain<'a>(
	format: Formats,
	width: u32,
	height: u32,
	base_level: &'a [u8],
) -> Result<MipChain<'a>, MipGenerationError> {
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

	let levels_count = mip_level_count(width, height)?;
	let mut levels = Vec::with_capacity(levels_count as usize);

	let mut current_level = StoredMipLevel {
		width,
		height,
		data: Cow::Borrowed(base_level),
	};

	loop {
		let current_width = current_level.width;
		let current_height = current_level.height;

		if current_width == 1 && current_height == 1 {
			levels.push(current_level);
			break;
		}

		let next_width = (current_width / 2).max(1);
		let next_height = (current_height / 2).max(1);
		let next_size =
			expected_size(next_width, next_height, bytes_per_pixel).ok_or(MipGenerationError::DimensionsTooLarge)?;

		let mut next_data = vec![0_u8; next_size];
		downsample_level(
			format,
			current_width,
			current_height,
			current_level.data.as_ref(),
			&mut next_data,
		)?;

		levels.push(current_level);
		current_level = StoredMipLevel {
			width: next_width,
			height: next_height,
			data: Cow::Owned(next_data),
		};
	}

	Ok(MipChain { levels })
}

fn bytes_per_pixel(format: Formats) -> Option<usize> {
	match format {
		Formats::RG8 => Some(2),
		Formats::RGB8 => Some(3),
		Formats::RGBA8 => Some(4),
		Formats::RGB16 => Some(6),
		Formats::RGBA16 => Some(8),
		Formats::BC5 | Formats::BC7 => None,
	}
}

fn expected_size(width: u32, height: u32, bytes_per_pixel: usize) -> Option<usize> {
	(width as usize).checked_mul(height as usize)?.checked_mul(bytes_per_pixel)
}

/// Downsamples one level according to format and channel depth.
fn downsample_level(
	format: Formats,
	source_width: u32,
	source_height: u32,
	source: &[u8],
	destination: &mut [u8],
) -> Result<(), MipGenerationError> {
	match format {
		Formats::RG8 => downsample_u8::<2>(source_width, source_height, source, destination),
		Formats::RGB8 => downsample_u8::<3>(source_width, source_height, source, destination),
		Formats::RGBA8 => downsample_u8::<4>(source_width, source_height, source, destination),
		Formats::RGB16 => downsample_u16::<3>(source_width, source_height, source, destination),
		Formats::RGBA16 => downsample_u16::<4>(source_width, source_height, source, destination),
		Formats::BC5 | Formats::BC7 => {
			return Err(MipGenerationError::UnsupportedFormat(format));
		}
	}

	Ok(())
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
	use crate::types::Formats;

	use super::{generate_mip_chain, mip_level_count, MipChain, MipGenerationError};

	#[derive(Debug, Clone, PartialEq, Eq)]
	struct ExpectedMipLevel {
		width: u32,
		height: u32,
		data: Vec<u8>,
	}

	#[test]
	fn generates_rgba8_chain_with_even_extent() {
		let width = 4_u32;
		let height = 4_u32;
		let data = create_rgba8_pattern(width, height);

		let generated = generate_mip_chain(Formats::RGBA8, width, height, &data).expect("mips must generate");
		let expected = scalar_mip_chain_u8::<4>(width, height, &data);

		assert_chain_matches(&generated, &expected);
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
	fn generates_rgba16_chain() {
		let width = 3_u32;
		let height = 4_u32;
		let data = create_rgba16_pattern(width, height);

		let generated = generate_mip_chain(Formats::RGBA16, width, height, &data).expect("16-bit mips must generate");
		let expected = scalar_mip_chain_u16::<4>(width, height, &data);

		assert_chain_matches(&generated, &expected);
	}

	#[test]
	fn rejects_invalid_buffer_size() {
		let error = generate_mip_chain(Formats::RGBA8, 2, 2, &[0_u8; 3]).expect_err("must fail");

		assert_eq!(error, MipGenerationError::BufferSizeMismatch { expected: 16, got: 3 });
	}

	#[test]
	fn rejects_unsupported_format() {
		let error = generate_mip_chain(Formats::BC7, 1, 1, &[]).expect_err("must fail");

		assert_eq!(error, MipGenerationError::UnsupportedFormat(Formats::BC7));
	}

	#[test]
	fn counts_mip_levels() {
		let count = mip_level_count(17, 9).expect("valid size");
		assert_eq!(count, 5);
	}

	fn assert_chain_matches(chain: &MipChain<'_>, expected: &[ExpectedMipLevel]) {
		assert_eq!(chain.len(), expected.len());

		for (generated, expected_level) in chain.levels().zip(expected.iter()) {
			assert_eq!(generated.width, expected_level.width);
			assert_eq!(generated.height, expected_level.height);
			assert_eq!(generated.data, expected_level.data.as_slice());
		}
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
