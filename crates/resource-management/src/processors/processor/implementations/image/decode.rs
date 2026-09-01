//! Decode high-precision image files into linear RGBA16F pixels.

use std::{alloc::Allocator, error::Error, fmt, io::Cursor};

use exr::prelude::{ReadChannels as _, ReadImage as _, ReadLayers as _, f16};
use image::ImageDecoder as _;
use utils::Extent;

use super::{CanonicalImageData, ImageSource, SourceChannels, SourceEncoding, canonicalize_rgba16f_in};
use crate::types::Gamma;

/// The `DecodedImage` struct owns one decoded linear RGBA16F image.
pub(crate) struct DecodedImage<'a> {
	format: image::ImageFormat,
	extent: Extent,
	data: Box<[u8], &'a dyn Allocator>,
}

impl DecodedImage<'_> {
	/// Returns the encoded file format selected from the source content or extension hint.
	pub(crate) fn format(&self) -> image::ImageFormat {
		self.format
	}

	/// Returns the decoded two-dimensional extent.
	pub(crate) fn extent(&self) -> Extent {
		self.extent
	}

	/// Returns the tightly packed linear RGBA16F pixels.
	pub(crate) fn data(&self) -> &[u8] {
		&self.data
	}
}

/// The `ImageDecodeError` enum identifies failures before decoded pixels reach an image processor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageDecodeError {
	UnknownFormat,
	UnsupportedFormat(image::ImageFormat),
	UnsupportedColorType(image::ColorType),
	UnsupportedTransferFunction,
	InsufficientPrecision,
	ZeroDimensions,
	DimensionsTooLarge,
	AllocationFailed,
	InvalidData,
}

impl fmt::Display for ImageDecodeError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::UnknownFormat => formatter.write_str(
				"Image format could not be identified. The most likely cause is missing or unrecognized format bytes.",
			),
			Self::UnsupportedFormat(format) => write!(
				formatter,
				"Image format {format:?} is unsupported. The most likely cause is that its decoder is not enabled."
			),
			Self::UnsupportedColorType(color_type) => write!(
				formatter,
				"Image color type {color_type:?} is unsupported. The most likely cause is a channel layout that the image processor cannot represent."
			),
			Self::UnsupportedTransferFunction => formatter.write_str(
				"Image transfer function is unsupported. The most likely cause is explicit color metadata that is neither linear nor sRGB.",
			),
			Self::InsufficientPrecision => formatter.write_str(
				"Image precision is insufficient. The most likely cause is an 8-bit integer source; use 16-bit integer or floating-point samples.",
			),
			Self::ZeroDimensions => formatter.write_str(
				"Image dimensions are invalid. The most likely cause is a source with zero width or height.",
			),
			Self::DimensionsTooLarge => formatter.write_str(
				"Image dimensions are too large. The most likely cause is decoded storage that exceeds this platform's address space.",
			),
			Self::AllocationFailed => formatter.write_str(
				"Image decode allocation failed. The most likely cause is insufficient asset-baking memory.",
			),
			Self::InvalidData => formatter.write_str(
				"Image data could not be decoded. The most likely cause is malformed data or a file extension that does not match its contents.",
			),
		}
	}
}

impl Error for ImageDecodeError {}

/// Decodes one high-precision image to linear RGBA16F after identifying its encoded contents.
pub(crate) fn decode_rgba16f_in<'a>(
	encoded: &[u8],
	allocator: &'a dyn Allocator,
) -> Result<DecodedImage<'a>, ImageDecodeError> {
	let format = image::guess_format(encoded).map_err(|_| ImageDecodeError::UnknownFormat)?;

	if format == image::ImageFormat::OpenExr {
		return decode_exr_in(encoded, allocator);
	}

	decode_image_rs_in(encoded, format, allocator)
}

/// Decodes formats provided by `image` directly into the bake allocator.
fn decode_image_rs_in<'a>(
	encoded: &[u8],
	format: image::ImageFormat,
	allocator: &'a dyn Allocator,
) -> Result<DecodedImage<'a>, ImageDecodeError> {
	if !format.reading_enabled() {
		return Err(ImageDecodeError::UnsupportedFormat(format));
	}

	let gamma = source_gamma(encoded, format)?;
	let decoder = image::ImageReader::with_format(Cursor::new(encoded), format)
		.into_decoder()
		.map_err(|_| ImageDecodeError::InvalidData)?;
	let (width, height) = decoder.dimensions();

	if width == 0 || height == 0 {
		return Err(ImageDecodeError::ZeroDimensions);
	}

	let (channels, encoding) = source_layout(decoder.color_type())?;

	if encoding == SourceEncoding::U8 {
		return Err(ImageDecodeError::InsufficientPrecision);
	}

	let byte_len = usize::try_from(decoder.total_bytes()).map_err(|_| ImageDecodeError::DimensionsTooLarge)?;
	let mut data = Vec::new_in(allocator);

	data.try_reserve_exact(byte_len)
		.map_err(|_| ImageDecodeError::AllocationFailed)?;
	data.resize(byte_len, 0);

	decoder.read_image(&mut data).map_err(|_| ImageDecodeError::InvalidData)?;
	let source = ImageSource::new(Extent::rectangle(width, height), channels, encoding, &data);
	let converted = canonicalize_rgba16f_in(source, gamma, allocator).ok_or(ImageDecodeError::InvalidData)?;
	let CanonicalImageData::Owned(data) = converted else {
		unreachable!("image-rs does not expose borrowed RGBA16F decoder output")
	};

	Ok(DecodedImage {
		format,
		extent: Extent::rectangle(width, height),
		data,
	})
}

/// Selects the transfer function that environment processing must remove from decoded RGB samples.
fn source_gamma(encoded: &[u8], format: image::ImageFormat) -> Result<Gamma, ImageDecodeError> {
	match format {
		image::ImageFormat::Hdr => Ok(Gamma::Linear),
		image::ImageFormat::Png => png_gamma(encoded),
		// Farbfeld stores 16-bit samples but specifies sRGB RGB values for interoperability.
		image::ImageFormat::Farbfeld => Ok(Gamma::SRGB),
		// The decoders do not expose a reliable transfer contract for TIFF or PNM. Environment sources use their
		// high-precision samples as linear radiance instead of guessing and irreversibly applying a display transfer.
		image::ImageFormat::Tiff | image::ImageFormat::Pnm => Ok(Gamma::Linear),
		_ => Err(ImageDecodeError::UnsupportedFormat(format)),
	}
}

/// Reads PNG transfer metadata before the image decoder consumes the pixel stream.
fn png_gamma(encoded: &[u8]) -> Result<Gamma, ImageDecodeError> {
	let reader = png::Decoder::new(Cursor::new(encoded))
		.read_info()
		.map_err(|_| ImageDecodeError::InvalidData)?;
	let info = reader.info();

	if info.srgb.is_some() {
		return Ok(Gamma::SRGB);
	}

	// ICC profiles can carry arbitrary parametric or sampled transfer curves. Do not silently treat one as sRGB.
	if info.icc_profile.is_some() {
		return Err(ImageDecodeError::UnsupportedTransferFunction);
	}

	if let Some(cicp) = info.coding_independent_code_points {
		return match cicp.transfer_function {
			8 => Ok(Gamma::Linear),
			13 => Ok(Gamma::SRGB),
			_ => Err(ImageDecodeError::UnsupportedTransferFunction),
		};
	}

	let Some(gamma) = info.gama_chunk.map(png::ScaledFloat::into_scaled) else {
		// PNGs without color metadata conventionally contain display-referred color. This matches standalone PNG assets.
		return Ok(Gamma::SRGB);
	};

	match gamma {
		40_000..=50_000 => Ok(Gamma::SRGB),
		95_000..=105_000 => Ok(Gamma::Linear),
		_ => Err(ImageDecodeError::UnsupportedTransferFunction),
	}
}

/// Maps decoder output into the byte layout consumed by the common image source.
fn source_layout(color_type: image::ColorType) -> Result<(SourceChannels, SourceEncoding), ImageDecodeError> {
	let layout = match color_type {
		image::ColorType::L8 => (SourceChannels::Luminance, SourceEncoding::U8),
		image::ColorType::La8 => (SourceChannels::LuminanceAlpha, SourceEncoding::U8),
		image::ColorType::Rgb8 => (SourceChannels::RGB, SourceEncoding::U8),
		image::ColorType::Rgba8 => (SourceChannels::RGBA, SourceEncoding::U8),
		image::ColorType::L16 => (SourceChannels::Luminance, SourceEncoding::U16NativeEndian),
		image::ColorType::La16 => (SourceChannels::LuminanceAlpha, SourceEncoding::U16NativeEndian),
		image::ColorType::Rgb16 => (SourceChannels::RGB, SourceEncoding::U16NativeEndian),
		image::ColorType::Rgba16 => (SourceChannels::RGBA, SourceEncoding::U16NativeEndian),
		image::ColorType::Rgb32F => (SourceChannels::RGB, SourceEncoding::F32NativeEndian),
		image::ColorType::Rgba32F => (SourceChannels::RGBA, SourceEncoding::F32NativeEndian),
		_ => return Err(ImageDecodeError::UnsupportedColorType(color_type)),
	};

	Ok(layout)
}

/// The `DecodedExr` struct accumulates allocator-backed RGBA16F pixels while EXR blocks are visited.
struct DecodedExr<'a> {
	data: Vec<u8, &'a dyn Allocator>,
	extent: Option<Extent>,
	width: usize,
	valid: bool,
}

impl<'a> DecodedExr<'a> {
	/// Allocates the exact half-float RGBA surface required by the decoded EXR layer.
	fn new(resolution: exr::prelude::Vec2<usize>, allocator: &'a dyn Allocator) -> Self {
		let extent = u32::try_from(resolution.width())
			.ok()
			.zip(u32::try_from(resolution.height()).ok())
			.map(|(width, height)| Extent::rectangle(width, height));
		let byte_len = resolution
			.width()
			.checked_mul(resolution.height())
			.and_then(|pixel_count| pixel_count.checked_mul(4 * std::mem::size_of::<f16>()));
		let mut data = Vec::new_in(allocator);
		let valid = extent.is_some()
			&& byte_len
				.map(|byte_len| {
					if data.try_reserve_exact(byte_len).is_err() {
						return false;
					}

					data.resize(byte_len, 0);

					true
				})
				.unwrap_or(false);

		Self {
			data,
			extent,
			width: resolution.width(),
			valid,
		}
	}

	/// Writes one decoded pixel into its tightly packed little-endian RGBA16F location.
	fn set_pixel(&mut self, position: exr::prelude::Vec2<usize>, channels: (f16, f16, f16, f16)) {
		let Some(offset) = position
			.y()
			.checked_mul(self.width)
			.and_then(|row| row.checked_add(position.x()))
			.and_then(|pixel| pixel.checked_mul(4 * std::mem::size_of::<f16>()))
		else {
			self.valid = false;

			return;
		};
		let Some(end) = offset.checked_add(4 * std::mem::size_of::<f16>()) else {
			self.valid = false;

			return;
		};
		let Some(pixel) = self.data.get_mut(offset..end) else {
			self.valid = false;

			return;
		};

		for (destination, channel) in pixel
			.chunks_exact_mut(std::mem::size_of::<f16>())
			.zip([channels.0, channels.1, channels.2, channels.3])
		{
			destination.copy_from_slice(&channel.to_le_bytes());
		}
	}
}

/// Decodes EXR directly to half floats so highlights do not require an intermediate f32 surface.
fn decode_exr_in<'a>(encoded: &[u8], allocator: &'a dyn Allocator) -> Result<DecodedImage<'a>, ImageDecodeError> {
	let image = exr::prelude::read()
		.no_deep_data()
		.largest_resolution_level()
		.rgba_channels(
			|resolution, _| DecodedExr::new(resolution, allocator),
			|pixels, position, channels: (f16, f16, f16, f16)| pixels.set_pixel(position, channels),
		)
		.first_valid_layer()
		.all_attributes()
		.from_buffered(Cursor::new(encoded))
		.map_err(|_| ImageDecodeError::InvalidData)?;
	let decoded = image.layer_data.channel_data.pixels;
	let extent = decoded
		.extent
		.filter(|_| decoded.valid)
		.ok_or(ImageDecodeError::InvalidData)?;

	if extent.width() == 0 || extent.height() == 0 {
		return Err(ImageDecodeError::ZeroDimensions);
	}

	Ok(DecodedImage {
		format: image::ImageFormat::OpenExr,
		extent,
		data: decoded.data.into_boxed_slice(),
	})
}

#[cfg(test)]
mod tests {
	use std::{alloc::Global, io::Cursor};

	use exr::prelude::{SpecificChannels, WritableImage as _};
	use image::{ExtendedColorType, ImageEncoder as _, codecs::hdr::HdrEncoder};

	use super::{ImageDecodeError, decode_rgba16f_in};

	#[derive(Clone, Copy)]
	enum PngTransfer {
		Unspecified,
		Srgb,
		Gamma(u32),
	}

	/// Encodes a tiny EXR with values outside normalized image range.
	fn exr_fixture() -> Vec<u8> {
		let channels = SpecificChannels::rgb(|position: exr::prelude::Vec2<usize>| match position.x() {
			0 => (4.0_f32, 0.5_f32, -0.25_f32),
			_ => (16.0_f32, 2.0_f32, 8.0_f32),
		});
		let image = exr::prelude::Image::from_channels((2, 1), channels);
		let mut bytes = Vec::new();

		image
			.write()
			.non_parallel()
			.to_buffered(Cursor::new(&mut bytes))
			.expect("the EXR fixture must encode");

		bytes
	}

	/// Encodes a tiny Radiance image that has no standalone engine asset handler.
	fn hdr_fixture() -> Vec<u8> {
		let mut bytes = Vec::new();
		let pixels = [image::Rgb([4.0, 0.5, 0.25]), image::Rgb([16.0, 2.0, 8.0])];

		HdrEncoder::new(&mut bytes)
			.encode(&pixels, 2, 1)
			.expect("the HDR fixture must encode");

		bytes
	}

	/// Encodes one RGBA PNG at the requested sample precision.
	fn png_fixture(bit_depth: png::BitDepth, transfer: PngTransfer) -> Vec<u8> {
		let mut bytes = Vec::new();

		{
			let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
			encoder.set_color(png::ColorType::Rgba);
			encoder.set_depth(bit_depth);
			match transfer {
				PngTransfer::Unspecified => {}
				PngTransfer::Srgb => encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual),
				PngTransfer::Gamma(gamma) => encoder.set_source_gamma(png::ScaledFloat::from_scaled(gamma)),
			}
			let mut writer = encoder.write_header().expect("the PNG header must encode");
			let pixels: &[u8] = match bit_depth {
				png::BitDepth::Eight => &[0x20, 0x80, 0xe0, 0xff],
				png::BitDepth::Sixteen => &[0x20, 0x00, 0x80, 0x00, 0xe0, 0x00, 0x40, 0x00],
				_ => unreachable!("the fixture only uses 8-bit and 16-bit PNGs"),
			};

			writer.write_image_data(pixels).expect("the PNG pixels must encode");
		}

		bytes
	}

	fn farbfeld_fixture() -> Vec<u8> {
		let mut bytes = b"farbfeld".to_vec();
		bytes.extend_from_slice(&1_u32.to_be_bytes());
		bytes.extend_from_slice(&1_u32.to_be_bytes());
		for channel in [u16::MAX / 2, u16::MAX / 4, u16::MAX, u16::MAX / 8] {
			bytes.extend_from_slice(&channel.to_be_bytes());
		}
		bytes
	}

	fn tiff_fixture() -> Vec<u8> {
		let samples = [u16::MAX / 2, u16::MAX / 4, u16::MAX]
			.into_iter()
			.flat_map(u16::to_ne_bytes)
			.collect::<Vec<_>>();
		let mut cursor = Cursor::new(Vec::new());

		image::codecs::tiff::TiffEncoder::new(&mut cursor)
			.write_image(&samples, 1, 1, ExtendedColorType::Rgb16)
			.expect("the TIFF fixture must encode");

		cursor.into_inner()
	}

	fn pnm_fixture() -> Vec<u8> {
		let mut bytes = Vec::new();
		let samples = [u16::MAX / 2, u16::MAX / 4, u16::MAX];

		image::codecs::pnm::PnmEncoder::new(&mut bytes)
			.encode(&samples[..], 1, 1, ExtendedColorType::Rgb16)
			.expect("the PNM fixture must encode");

		bytes
	}

	fn rgba16f_values(data: &[u8]) -> Vec<f32> {
		data.chunks_exact(2)
			.map(|bytes| exr::prelude::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32())
			.collect()
	}

	#[test]
	fn content_sniffing_keeps_exr_hdr_values_and_layouts() {
		let exr = decode_rgba16f_in(&exr_fixture(), &Global).expect("EXR content must decode");

		assert_eq!(exr.format(), image::ImageFormat::OpenExr);
		assert_eq!(exr.extent().as_array(), [2, 1, 0]);
		assert_eq!(rgba16f_values(exr.data()), vec![4.0, 0.5, -0.25, 1.0, 16.0, 2.0, 8.0, 1.0]);

		let hdr = decode_rgba16f_in(&hdr_fixture(), &Global).expect("Radiance HDR content must decode");

		assert_eq!(hdr.format(), image::ImageFormat::Hdr);
		assert_eq!(hdr.extent().as_array(), [2, 1, 0]);
		assert_eq!(rgba16f_values(hdr.data()), vec![4.0, 0.5, 0.25, 1.0, 16.0, 2.0, 8.0, 1.0]);
	}

	#[test]
	fn environment_decode_rejects_8_bit_but_accepts_16_bit_pngs() {
		let eight_bit = decode_rgba16f_in(&png_fixture(png::BitDepth::Eight, PngTransfer::Unspecified), &Global);
		let sixteen_bit = decode_rgba16f_in(&png_fixture(png::BitDepth::Sixteen, PngTransfer::Unspecified), &Global);

		assert!(matches!(eight_bit, Err(ImageDecodeError::InsufficientPrecision)));
		assert!(sixteen_bit.is_ok());
	}

	#[test]
	fn supported_png_gamma_metadata_selects_the_linearization() {
		for (gamma, expected_red) in [(45_000, 0.01435), (100_000, 0.125)] {
			let decoded = decode_rgba16f_in(&png_fixture(png::BitDepth::Sixteen, PngTransfer::Gamma(gamma)), &Global)
				.expect("supported PNG gamma must decode");
			let red = rgba16f_values(decoded.data())[0];

			assert!((red - expected_red).abs() < 0.001);
		}
	}

	#[test]
	fn high_precision_format_transfer_defaults_are_explicit() {
		for (encoded, expected_format, expected_red) in [
			(farbfeld_fixture(), image::ImageFormat::Farbfeld, 0.214),
			(tiff_fixture(), image::ImageFormat::Tiff, 0.5),
			(pnm_fixture(), image::ImageFormat::Pnm, 0.5),
		] {
			let decoded = decode_rgba16f_in(&encoded, &Global).expect("high-precision fixture must decode");
			let red = rgba16f_values(decoded.data())[0];

			assert_eq!(decoded.format(), expected_format);
			assert!((red - expected_red).abs() < 0.001);
		}
	}

	#[test]
	fn sixteen_bit_srgb_png_is_linearized_without_changing_alpha() {
		let decoded = decode_rgba16f_in(&png_fixture(png::BitDepth::Sixteen, PngTransfer::Srgb), &Global)
			.expect("16-bit sRGB PNG must decode");
		let values = rgba16f_values(decoded.data());

		assert!((values[0] - 0.01435).abs() < 0.001);
		assert!((values[1] - 0.214).abs() < 0.001);
		assert!((values[2] - 0.738).abs() < 0.002);
		assert!((values[3] - 0.25).abs() < 0.001);
	}

	#[test]
	fn unsupported_explicit_png_transfer_is_rejected() {
		let error = decode_rgba16f_in(&png_fixture(png::BitDepth::Sixteen, PngTransfer::Gamma(70_000)), &Global)
			.err()
			.expect("unsupported explicit PNG gamma must fail");

		assert_eq!(error, ImageDecodeError::UnsupportedTransferFunction);
	}

	#[test]
	fn unknown_and_malformed_images_report_decode_errors() {
		assert!(decode_rgba16f_in(b"not an image", &Global).is_err());
	}
}
