/// The `ImageAssetHandler` struct lets an asset manager bake standalone PNG and JPEG files into image resources.
///
/// The file's type selects the decoder. PNG palette and low-bit-depth images are expanded to whole channels while
/// decoding. The shared [`ImageProcessor`] does the rest, the same way for every format.
#[derive(Default)]
pub struct ImageAssetHandler;

impl ImageAssetHandler {
	pub fn new() -> ImageAssetHandler {
		ImageAssetHandler
	}
}

impl AssetHandler for ImageAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		matches!(
			r#type,
			"png" | "jpg" | "jpeg" | "Image" | "image/png" | "image/jpeg"
		)
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (data, dt) = context.resolve(url).await?;

		let allocator = context.allocator();

		let semantic = guess_semantic_from_name(url.get_base());

		let spec = context.load_sidecar(url).await?;
		let settings = image_settings(&context, url, spec.as_ref(), IMAGE_SETTINGS_KEY, semantic)?;

		let (decoder, header) = SourceDecoder::open(&dt, &data, semantic).inspect_err(|_| {
			log::error!(
				"Image '{}' could not be decoded. The most likely cause is malformed data or a file extension that doesn't match its contents. Re-export the image as PNG or JPEG with the matching extension.",
				url.as_ref()
			);
		})?;

		// The decode buffer reserves the whole stored chain when the processor can keep the decoded pixels as the
		// stored image, so they're never copied.
		let capacity =
			ImageProcessor::in_place_capacity(semantic, header.gamma, settings, header.extent, header.channels, header.encoding)
				.unwrap_or(0);
		let mut buffer = Vec::with_capacity_in(header.size.max(capacity), allocator);
		buffer.resize(header.size, 0);
		let written = decoder.read(&mut buffer)?;
		buffer.truncate(written);

		let source = OwnedImageSource {
			extent: header.extent,
			channels: header.channels,
			encoding: header.encoding,
			data: buffer,
		};
		let image = context
			.image_processor()
			.process_owned_in(semantic, header.gamma, settings, source, allocator)
			.await?;
		let (asset, data) = image.into_asset(url);

		context.store_primary(asset, &data).await
	}
}

/// The `SourceHeader` struct describes the pixels a [`SourceDecoder`] writes, so the handler can size the buffer
/// before decoding.
struct SourceHeader {
	extent: Extent,
	channels: SourceChannels,
	encoding: SourceEncoding,
	gamma: Gamma,
	/// The number of bytes the decoder needs in its output buffer.
	size: usize,
}

/// The `SourceDecoder` enum holds a decoder that has read an image's header and is ready to write its pixels.
enum SourceDecoder<'d> {
	Png(png::Reader<Cursor<&'d [u8]>>),
	Jpeg(image::codecs::jpeg::JpegDecoder<Cursor<&'d [u8]>>),
}

impl<'d> SourceDecoder<'d> {
	/// Reads the header of `data`, decoded as the format the asset type `dt` names.
	fn open(dt: &str, data: &'d [u8], semantic: Semantic) -> Result<(Self, SourceHeader), LoadErrors> {
		match dt {
			"png" | "image/png" => {
				let mut decoder = png::Decoder::new(Cursor::new(data));
				decoder.set_transformations(png::Transformations::EXPAND);
				let reader = decoder.read_info().map_err(|_| LoadErrors::FailedToProcess)?;
				let extent = Extent::rectangle(reader.info().width, reader.info().height);
				let gamma = png_gamma(reader.info(), semantic);
				let (color_type, bit_depth) = reader.output_color_type();
				let (channels, encoding) = png_source_layout(color_type, bit_depth)?;
				let size = reader.output_buffer_size().ok_or(LoadErrors::FailedToProcess)?;

				Ok((
					Self::Png(reader),
					SourceHeader {
						extent,
						channels,
						encoding,
						gamma,
						size,
					},
				))
			}
			"jpg" | "jpeg" | "image/jpeg" => {
				let decoder =
					image::codecs::jpeg::JpegDecoder::new(Cursor::new(data)).map_err(|_| LoadErrors::FailedToProcess)?;
				let (width, height) = decoder.dimensions();
				// The decoder converts CMYK to RGB, so only 8-bit luminance and RGB reach the processor.
				let channels = match decoder.color_type() {
					image::ColorType::L8 => SourceChannels::Luminance,
					image::ColorType::Rgb8 => SourceChannels::RGB,
					_ => return Err(LoadErrors::FailedToProcess),
				};
				let size = usize::try_from(decoder.total_bytes()).map_err(|_| LoadErrors::FailedToProcess)?;

				// JPEG files carry no transfer metadata the engine reads, so the semantic decides.
				Ok((
					Self::Jpeg(decoder),
					SourceHeader {
						extent: Extent::rectangle(width, height),
						channels,
						encoding: SourceEncoding::U8,
						gamma: gamma_from_semantic(semantic),
						size,
					},
				))
			}
			_ => Err(LoadErrors::UnsupportedType),
		}
	}

	/// Writes the decoded pixels into `buffer`, which holds at least the header's size, and returns the bytes written.
	fn read(self, buffer: &mut [u8]) -> Result<usize, LoadErrors> {
		match self {
			Self::Png(mut reader) => reader
				.next_frame(buffer)
				.map(|info| info.buffer_size())
				.map_err(|_| LoadErrors::FailedToProcess),
			Self::Jpeg(decoder) => {
				let size = buffer.len();
				decoder.read_image(buffer).map_err(|_| LoadErrors::FailedToProcess)?;
				Ok(size)
			}
		}
	}
}

/// Determines the image gamma from its semantic and the transfer functions the engine can represent.
fn png_gamma(info: &png::Info<'_>, semantic: Semantic) -> Gamma {
	let semantic_gamma = gamma_from_semantic(semantic);

	// Color-profile metadata is frequently attached to every exported PNG. Data textures must keep their numeric samples
	// linear even when an editor added an sRGB chunk during export.
	if semantic_gamma == Gamma::Linear {
		return Gamma::Linear;
	}

	// Unrepresentable or missing metadata falls back to the semantic's transfer function.
	png_declared_gamma(info).and_then(Result::ok).unwrap_or(semantic_gamma)
}

/// Maps PNG decoder output into the source layout normalized by the common image processor.
fn png_source_layout(
	color_type: png::ColorType,
	bit_depth: png::BitDepth,
) -> Result<(SourceChannels, SourceEncoding), LoadErrors> {
	match (color_type, bit_depth) {
		(png::ColorType::Grayscale, png::BitDepth::Eight) => Ok((SourceChannels::Luminance, SourceEncoding::U8)),
		(png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => Ok((SourceChannels::LuminanceAlpha, SourceEncoding::U8)),
		(png::ColorType::Rgb, png::BitDepth::Eight) => Ok((SourceChannels::RGB, SourceEncoding::U8)),
		(png::ColorType::Rgba, png::BitDepth::Eight) => Ok((SourceChannels::RGBA, SourceEncoding::U8)),
		(png::ColorType::Grayscale, png::BitDepth::Sixteen) => Ok((SourceChannels::Luminance, SourceEncoding::U16BigEndian)),
		(png::ColorType::GrayscaleAlpha, png::BitDepth::Sixteen) => {
			Ok((SourceChannels::LuminanceAlpha, SourceEncoding::U16BigEndian))
		}
		(png::ColorType::Rgb, png::BitDepth::Sixteen) => Ok((SourceChannels::RGB, SourceEncoding::U16BigEndian)),
		(png::ColorType::Rgba, png::BitDepth::Sixteen) => Ok((SourceChannels::RGBA, SourceEncoding::U16BigEndian)),
		_ => Err(LoadErrors::FailedToProcess),
	}
}

#[cfg(test)]
mod tests {
	use crate::{
		asset::{
			self, ResourceId, handler::AssetHandler, handler::implementations::image::ImageAssetHandler, manager::AssetManager,
		},
		r#async, resource,
		resources::image::Image,
		types::{Formats, Gamma},
	};

	#[derive(Clone, Copy)]
	enum GammaMetadata {
		Unspecified,
		Srgb,
		Gamma(u32),
	}

	/// Encodes a small RGBA8 image with the requested authored gamma metadata.
	fn generated_rgba8_png(metadata: GammaMetadata) -> Vec<u8> {
		let mut png = Vec::new();

		{
			let mut encoder = png::Encoder::new(&mut png, 4, 4);

			encoder.set_color(png::ColorType::Rgba);

			encoder.set_depth(png::BitDepth::Eight);

			match metadata {
				GammaMetadata::Unspecified => {}
				GammaMetadata::Srgb => encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual),
				GammaMetadata::Gamma(gamma) => encoder.set_source_gamma(png::ScaledFloat::from_scaled(gamma)),
			}

			let mut writer = encoder.write_header().expect("generated PNG header should encode");

			let pixels = [0x20, 0x80, 0xe0, 0xff].repeat(16);

			writer.write_image_data(&pixels).expect("generated PNG pixels should encode");
		}

		png
	}

	#[r#async::test]
	async fn standalone_png_infers_gamma_from_semantics_and_supported_metadata() {
		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();
		let resource_storage_backend = resource::storage_backend::MemoryStorageBackend::new();
		let cases = [
			(
				"textures/default.png",
				GammaMetadata::Unspecified,
				Gamma::SRGB,
				Formats::RGBA8SRGB,
			),
			(
				"textures/authored-srgb.png",
				GammaMetadata::Srgb,
				Gamma::SRGB,
				Formats::RGBA8SRGB,
			),
			(
				"textures/approximate-srgb.png",
				GammaMetadata::Gamma(45_000),
				Gamma::SRGB,
				Formats::RGBA8SRGB,
			),
			(
				"textures/authored-linear.png",
				GammaMetadata::Gamma(100_000),
				Gamma::Linear,
				Formats::RGBA8,
			),
			(
				"textures/unsupported-gamma.png",
				GammaMetadata::Gamma(70_000),
				Gamma::SRGB,
				Formats::RGBA8SRGB,
			),
			(
				"textures/default-albedo.png",
				GammaMetadata::Unspecified,
				Gamma::SRGB,
				Formats::BC7SRGB,
			),
			(
				"textures/albedo.png",
				GammaMetadata::Gamma(100_000),
				Gamma::Linear,
				Formats::BC7,
			),
			("textures/normal.png", GammaMetadata::Srgb, Gamma::Linear, Formats::BC5),
		];

		for (id, metadata, ..) in cases {
			asset_storage_backend.add_file(id, &generated_rgba8_png(metadata));
		}

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(ImageAssetHandler::new());

		for (id, _, expected_gamma, expected_format) in cases {
			asset_manager.bake(id).await.expect("generated 8-bit PNG should bake");

			let resource = resource_storage_backend
				.get_resource(ResourceId::new(id))
				.expect("baked PNG resource should be stored");
			let image: Image = crate::from_slice(&resource.resource).expect("baked PNG metadata should deserialize");

			assert_eq!((image.gamma, image.format), (expected_gamma, expected_format), "asset: {id}");
		}
	}

	/// Encodes a small 8-bit RGB JPEG.
	fn generated_rgb8_jpeg() -> Vec<u8> {
		let mut jpeg = Vec::new();
		let pixels = [0x20, 0x80, 0xe0].repeat(16);

		::image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
			.encode(&pixels, 4, 4, ::image::ExtendedColorType::Rgb8)
			.expect("generated JPEG should encode");

		jpeg
	}

	#[r#async::test]
	async fn standalone_jpeg_bakes_like_png_by_semantic_and_bead_file() {
		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();
		let resource_storage_backend = resource::storage_backend::MemoryStorageBackend::new();
		let jpeg = generated_rgb8_jpeg();
		let cases = [
			("textures/default.jpg", Gamma::SRGB, Formats::RGBA8SRGB, 3),
			("textures/albedo.jpeg", Gamma::SRGB, Formats::BC7SRGB, 3),
			("textures/normal.jpg", Gamma::Linear, Formats::BC5, 3),
			("textures/ui_albedo.jpg", Gamma::SRGB, Formats::RGBA8SRGB, 1),
		];

		for (id, ..) in cases {
			asset_storage_backend.add_file(id, &jpeg);
		}
		asset_storage_backend.add_file(
			"textures/ui_albedo.jpg.bead",
			b"{ image: { mips: false, compression: false } }",
		);

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(ImageAssetHandler::new());

		for (id, expected_gamma, expected_format, expected_mips) in cases {
			asset_manager.bake(id).await.expect("generated JPEG should bake");

			let resource = resource_storage_backend
				.get_resource(ResourceId::new(id))
				.expect("baked JPEG resource should be stored");
			let image: Image = crate::from_slice(&resource.resource).expect("baked JPEG metadata should deserialize");

			assert_eq!(resource.class, "Image", "asset: {id}");
			assert_eq!(
				(image.gamma, image.format, image.mip_count),
				(expected_gamma, expected_format, expected_mips),
				"asset: {id}"
			);
		}
	}

	#[r#async::test]
	async fn standalone_png_takes_mip_and_compression_settings_from_its_bead_file() {
		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();
		let resource_storage_backend = resource::storage_backend::MemoryStorageBackend::new();
		let png = generated_rgba8_png(GammaMetadata::Unspecified);

		asset_storage_backend.add_file("textures/ui_albedo.png", &png);
		asset_storage_backend.add_file(
			"textures/ui_albedo.png.bead",
			b"{ image: { mips: false, compression: false } }",
		);
		asset_storage_backend.add_file("textures/brick_albedo.png", &png);
		asset_storage_backend.add_file("textures/broken_albedo.png", &png);
		asset_storage_backend.add_file("textures/broken_albedo.png.bead", b"{ image: { mipmaps: false } }");

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(ImageAssetHandler::new());

		// An albedo without a BEAD file keeps the semantic's BC7 mip chain; the BEAD file turns both steps off.
		for (id, expected_format, expected_mips) in [
			("textures/ui_albedo.png", Formats::RGBA8SRGB, 1),
			("textures/brick_albedo.png", Formats::BC7SRGB, 3),
		] {
			asset_manager.bake(id).await.expect("generated PNG should bake");

			let resource = resource_storage_backend
				.get_resource(ResourceId::new(id))
				.expect("baked PNG resource should be stored");
			let image: Image = crate::from_slice(&resource.resource).expect("baked PNG metadata should deserialize");

			assert_eq!(
				(image.format, image.mip_count),
				(expected_format, expected_mips),
				"asset: {id}"
			);
		}

		assert!(
			asset_manager.bake("textures/broken_albedo.png").await.is_err(),
			"an unknown image setting must fail the bake"
		);
	}
}

use std::io::Cursor;

use image::ImageDecoder as _;
use utils::Extent;

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	asset::{IMAGE_SETTINGS_KEY, image_settings},
	processors::image::{
		ImageProcessor, OwnedImageSource, Semantic, SourceChannels, SourceEncoding, gamma_from_semantic,
		guess_semantic_from_name, png_declared_gamma,
	},
	types::Gamma,
};
