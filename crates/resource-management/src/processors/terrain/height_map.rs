//! Decode height map images into one normalized value per sample, at the source's full precision.
//!
//! The image decoders the rest of the engine uses convert to color formats, such as RGBA16F, that keep fewer bits than
//! a 16-bit height map holds, so height maps get their own decoder.

use std::{alloc::Allocator, fmt, io::Cursor};

use exr::prelude::{ReadChannels as _, ReadImage as _, ReadLayers as _};

/// The `HeightMap` struct holds a height map's samples while a terrain bake turns them into heights.
///
/// Decode one with [`decode_height_map_in`], then pass it to
/// [`HeightField::from_height_map`](super::HeightField::from_height_map).
pub struct HeightMap<'a> {
	/// Samples along X, then along Z.
	pub(super) extent: [u32; 2],
	/// One value per sample, row by row from the image's top row, where 0 and 1 span the source's integer range.
	pub(super) values: Vec<f32, &'a dyn Allocator>,
}

impl HeightMap<'_> {
	/// Returns the samples along X, then along Z.
	pub fn extent(&self) -> [u32; 2] {
		self.extent
	}
}

/// The `HeightMapError` enum says why a height map can't become a terrain, so the bake can report the fix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeightMapError {
	/// The bytes are neither PNG nor EXR.
	UnsupportedFormat,
	/// The PNG uses a palette or a bit depth other than 8 or 16.
	UnsupportedPng,
	/// The image has fewer than two samples on a side.
	TooSmall,
	/// An EXR sample is infinite or not a number.
	NonFiniteSample,
	/// The file is malformed or its extension doesn't match its contents.
	InvalidData,
}

impl fmt::Display for HeightMapError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(match self {
			Self::UnsupportedFormat => {
				"The height map is neither PNG nor EXR. The most likely cause is an image exported in another format. Export it as a 16-bit grayscale PNG or an EXR."
			}
			Self::UnsupportedPng => {
				"The height map PNG uses a palette or a bit depth other than 8 or 16. The most likely cause is an export with color reduction. Export it as a 16-bit grayscale PNG."
			}
			Self::TooSmall => {
				"The height map has fewer than two samples on a side. The most likely cause is a wrong or empty export. Export a height map of at least 2 by 2 samples."
			}
			Self::NonFiniteSample => {
				"The height map EXR holds a sample that isn't a finite number. The most likely cause is an unfilled region in the source. Fill every sample before exporting it."
			}
			Self::InvalidData => {
				"The height map couldn't be decoded. The most likely cause is a damaged file or an extension that doesn't match its contents. Export the height map again."
			}
		})
	}
}

impl std::error::Error for HeightMapError {}

/// Decodes a PNG or EXR height map into one value per sample, chosen by the file's contents.
pub fn decode_height_map_in<'a>(encoded: &[u8], allocator: &'a dyn Allocator) -> Result<HeightMap<'a>, HeightMapError> {
	let map = match image::guess_format(encoded) {
		Ok(image::ImageFormat::Png) => decode_png_in(encoded, allocator)?,
		Ok(image::ImageFormat::OpenExr) => decode_exr_in(encoded, allocator)?,
		_ => return Err(HeightMapError::UnsupportedFormat),
	};
	if map.extent.iter().any(|&side| side < 2) {
		return Err(HeightMapError::TooSmall);
	}
	Ok(map)
}

/// Reads the first channel of every PNG pixel as a fraction of the bit depth's largest integer.
fn decode_png_in<'a>(encoded: &[u8], allocator: &'a dyn Allocator) -> Result<HeightMap<'a>, HeightMapError> {
	// No transformations: a 16-bit map must keep all 16 bits, and the decoder would otherwise expand palettes.
	let mut reader = png::Decoder::new(Cursor::new(encoded))
		.read_info()
		.map_err(|_| HeightMapError::InvalidData)?;
	let info = reader.info();
	let channels = match info.color_type {
		png::ColorType::Grayscale => 1,
		png::ColorType::GrayscaleAlpha => 2,
		png::ColorType::Rgb => 3,
		png::ColorType::Rgba => 4,
		png::ColorType::Indexed => return Err(HeightMapError::UnsupportedPng),
	};
	let bytes_per_channel = match info.bit_depth {
		png::BitDepth::Eight => 1,
		png::BitDepth::Sixteen => 2,
		_ => return Err(HeightMapError::UnsupportedPng),
	};
	let extent = [info.width, info.height];
	let mut pixels = vec![0; reader.output_buffer_size().ok_or(HeightMapError::InvalidData)?];
	let frame = reader.next_frame(&mut pixels).map_err(|_| HeightMapError::InvalidData)?;
	let pixel_size = channels * bytes_per_channel;

	let mut values = Vec::new_in(allocator);
	values.reserve_exact(extent[0] as usize * extent[1] as usize);
	for row in pixels[..frame.buffer_size()].chunks_exact(frame.line_size) {
		// Rows can carry padding past their last pixel, so only whole pixels are read.
		let row = &row[..extent[0] as usize * pixel_size];
		values.extend(row.chunks_exact(pixel_size).map(|pixel| match bytes_per_channel {
			1 => f32::from(pixel[0]) / f32::from(u8::MAX),
			// PNG stores 16-bit samples big-endian.
			_ => f32::from(u16::from_be_bytes([pixel[0], pixel[1]])) / f32::from(u16::MAX),
		}));
	}
	Ok(HeightMap { extent, values })
}

/// Reads the height channel of an EXR's first layer: `Y`, else `R`, else the first channel in name order.
fn decode_exr_in<'a>(encoded: &[u8], allocator: &'a dyn Allocator) -> Result<HeightMap<'a>, HeightMapError> {
	// Bakes already run across the asset workers, and exr's thread pool can't be joined, so decode on this thread.
	let image = exr::prelude::read()
		.no_deep_data()
		.largest_resolution_level()
		.all_channels()
		.first_valid_layer()
		.all_attributes()
		.non_parallel()
		.from_buffered(Cursor::new(encoded))
		.map_err(|_| HeightMapError::InvalidData)?;
	let layer = image.layer_data;
	// The format keeps channels sorted by name, so the first one is the first in name order.
	let channels = &layer.channel_data.list;
	let channel = ["Y", "R"]
		.into_iter()
		.find_map(|name| channels.iter().find(|channel| channel.name.eq(name)))
		.or_else(|| channels.first())
		.ok_or(HeightMapError::InvalidData)?;
	let extent = [
		u32::try_from(layer.size.width()).map_err(|_| HeightMapError::InvalidData)?,
		u32::try_from(layer.size.height()).map_err(|_| HeightMapError::InvalidData)?,
	];

	let mut values = Vec::new_in(allocator);
	values.reserve_exact(layer.size.area());
	for value in channel.sample_data.values_as_f32() {
		if !value.is_finite() {
			return Err(HeightMapError::NonFiniteSample);
		}
		values.push(value);
	}
	if values.len() != layer.size.area() {
		return Err(HeightMapError::InvalidData);
	}
	Ok(HeightMap { extent, values })
}

#[cfg(test)]
pub(crate) mod tests {
	use std::{alloc::Global, io::Cursor};

	use exr::prelude::WritableImage as _;

	use super::{HeightMapError, decode_height_map_in};

	/// Encodes a grayscale PNG with one sample per value, row by row.
	pub(crate) fn grayscale_png(width: u32, height: u32, depth: png::BitDepth, samples: &[u16]) -> Vec<u8> {
		let mut bytes = Vec::new();
		{
			let mut encoder = png::Encoder::new(&mut bytes, width, height);
			encoder.set_color(png::ColorType::Grayscale);
			encoder.set_depth(depth);
			let mut writer = encoder.write_header().expect("the PNG header must encode");
			let data = match depth {
				png::BitDepth::Sixteen => samples.iter().flat_map(|sample| sample.to_be_bytes()).collect::<Vec<_>>(),
				_ => samples.iter().map(|&sample| sample as u8).collect(),
			};
			writer.write_image_data(&data).expect("the PNG data must encode");
		}
		bytes
	}

	#[test]
	fn sixteen_bit_png_samples_keep_every_bit() {
		let png = grayscale_png(2, 2, png::BitDepth::Sixteen, &[0, 1, 65_534, 65_535]);

		let map = decode_height_map_in(&png, &Global).unwrap();

		assert_eq!(map.extent(), [2, 2]);
		assert_eq!(*map.values, [0.0, 1.0 / 65_535.0, 65_534.0 / 65_535.0, 1.0]);
	}

	#[test]
	fn eight_bit_png_samples_span_zero_to_one() {
		let png = grayscale_png(2, 2, png::BitDepth::Eight, &[0, 51, 204, 255]);

		let map = decode_height_map_in(&png, &Global).unwrap();

		assert_eq!(*map.values, [0.0, 0.2, 0.8, 1.0]);
	}

	#[test]
	fn rgb_png_samples_read_the_first_channel() {
		let mut bytes = Vec::new();
		{
			let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
			encoder.set_color(png::ColorType::Rgb);
			encoder.set_depth(png::BitDepth::Eight);
			let mut writer = encoder.write_header().unwrap();
			writer
				.write_image_data(&[255, 0, 0, 0, 255, 255, 0, 0, 0, 255, 255, 255])
				.unwrap();
		}

		let map = decode_height_map_in(&bytes, &Global).unwrap();

		assert_eq!(*map.values, [1.0, 0.0, 0.0, 1.0]);
	}

	#[test]
	fn height_maps_need_two_samples_on_each_side() {
		let png = grayscale_png(4, 1, png::BitDepth::Sixteen, &[0; 4]);

		assert_eq!(decode_height_map_in(&png, &Global).err(), Some(HeightMapError::TooSmall));
	}

	#[test]
	fn exr_samples_read_the_luminance_channel_as_is() {
		let channel =
			|name: &str, values: Vec<f32>| exr::prelude::AnyChannel::new(name, exr::prelude::FlatSamples::F32(values));
		let channels = exr::prelude::AnyChannels::sort(exr::prelude::SmallVec::from_vec(vec![
			channel("A", vec![9.0; 4]),
			channel("Y", vec![-0.5, 0.25, 1.0, 2.0]),
		]));
		let image = exr::prelude::Image::from_channels((2, 2), channels);
		let mut bytes = Vec::new();
		image.write().non_parallel().to_buffered(Cursor::new(&mut bytes)).unwrap();

		let map = decode_height_map_in(&bytes, &Global).unwrap();

		assert_eq!(map.extent(), [2, 2]);
		assert_eq!(*map.values, [-0.5, 0.25, 1.0, 2.0]);
	}

	#[test]
	fn palette_pngs_and_other_formats_are_rejected() {
		let mut bytes = Vec::new();
		{
			let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
			encoder.set_color(png::ColorType::Indexed);
			encoder.set_depth(png::BitDepth::Eight);
			encoder.set_palette(vec![0, 0, 0]);
			let mut writer = encoder.write_header().unwrap();
			writer.write_image_data(&[0; 4]).unwrap();
		}

		assert_eq!(
			decode_height_map_in(&bytes, &Global).err(),
			Some(HeightMapError::UnsupportedPng)
		);
		assert_eq!(
			decode_height_map_in(b"not an image", &Global).err(),
			Some(HeightMapError::UnsupportedFormat)
		);
	}
}
