/// The `PNGAssetHandler` struct lets an asset manager bake PNG files into image resources.
///
/// Palette and low-bit-depth images are expanded to whole channels while decoding. The shared
/// [`ImageProcessor`] does the rest.
#[derive(Default)]
pub struct PNGAssetHandler;

impl PNGAssetHandler {
	pub fn new() -> PNGAssetHandler {
		PNGAssetHandler
	}
}

impl AssetHandler for PNGAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "png" || r#type == "Image" || r#type == "image/png"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (data, dt) = context.resolve(url).await?;

		let allocator = context.allocator();

		let semantic = guess_semantic_from_name(url.get_base());

		if !matches!(dt.as_str(), "png" | "image/png") {
			return Err(LoadErrors::UnsupportedType);
		}

		let spec = context.load_sidecar(url).await?;
		let settings = image_settings(&context, url, spec.as_ref(), IMAGE_SETTINGS_KEY, semantic)?;

		let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
		decoder.set_transformations(png::Transformations::EXPAND);
		let mut reader = decoder.read_info().map_err(|_| LoadErrors::FailedToProcess)?;
		let extent = Extent::rectangle(reader.info().width, reader.info().height);
		let gamma = png_gamma(reader.info(), semantic);
		let (color_type, bit_depth) = reader.output_color_type();
		let (channels, encoding) = png_source_layout(color_type, bit_depth)?;

		// The decode buffer reserves the whole stored chain when the processor can keep the decoded pixels as the
		// stored image, so they're never copied.
		let size = reader.output_buffer_size().ok_or(LoadErrors::FailedToProcess)?;
		let capacity = ImageProcessor::in_place_capacity(semantic, gamma, settings, extent, channels, encoding).unwrap_or(0);
		let mut buffer = Vec::with_capacity_in(size.max(capacity), allocator);
		buffer.resize(size, 0);
		let info = reader.next_frame(&mut buffer).map_err(|_| LoadErrors::FailedToProcess)?;
		buffer.truncate(info.buffer_size());

		let source = OwnedImageSource {
			extent,
			channels,
			encoding,
			data: buffer,
		};
		let image = context
			.image_processor()
			.process_owned_in(semantic, gamma, settings, source, allocator)
			.await?;
		let (asset, data) = image.into_asset(url);

		context.store_primary(asset, &data).await
	}
}

/// Determines the image gamma from its semantic and the transfer functions the engine can represent.
fn png_gamma(info: &png::Info<'_>, semantic: crate::processors::image::Semantic) -> Gamma {
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
			self, ResourceId, handler::AssetHandler, handler::implementations::png::PNGAssetHandler, manager::AssetManager,
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

		asset_manager.add_asset_handler(PNGAssetHandler::new());

		for (id, _, expected_gamma, expected_format) in cases {
			asset_manager.bake(id).await.expect("generated 8-bit PNG should bake");

			let resource = resource_storage_backend
				.get_resource(ResourceId::new(id))
				.expect("baked PNG resource should be stored");
			let image: Image = crate::from_slice(&resource.resource).expect("baked PNG metadata should deserialize");

			assert_eq!((image.gamma, image.format), (expected_gamma, expected_format), "asset: {id}");
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

		asset_manager.add_asset_handler(PNGAssetHandler::new());

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

use utils::Extent;

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	asset::{IMAGE_SETTINGS_KEY, image_settings},
	processors::image::{
		ImageProcessor, OwnedImageSource, SourceChannels, SourceEncoding, gamma_from_semantic, guess_semantic_from_name,
		png_declared_gamma,
	},
	types::Gamma,
};
