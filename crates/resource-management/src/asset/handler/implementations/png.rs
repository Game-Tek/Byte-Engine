/// The `PNGAssetHandler` struct configures PNG decoding for image assets.
pub struct PNGAssetHandler {
	transformations: png::Transformations,
}

impl PNGAssetHandler {
	pub fn new() -> PNGAssetHandler {
		PNGAssetHandler {
			transformations: png::Transformations::EXPAND,
		}
	}

	/// Creates a PNG asset handler with explicit decoder transformations.
	pub fn with_transformations(transformations: png::Transformations) -> PNGAssetHandler {
		PNGAssetHandler { transformations }
	}
}

impl AssetHandler for PNGAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "png" || r#type == "Image" || r#type == "image/png"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<(), LoadErrors> {
		if let Some(dt) = context.resource_type(url)
			&& !self.can_handle(dt)
		{
			return Err(LoadErrors::UnsupportedType);
		}

		let (data, _, dt) = context.resolve(url).await?;

		let allocator = context.allocator();

		let semantic = guess_semantic_from_name(url.get_base());

		let transformations = self.transformations;

		if !matches!(dt.as_str(), "png" | "image/png") {
			return Err(LoadErrors::UnsupportedType);
		}

		let cursor = std::io::Cursor::new(data);
		let mut decoder = png::Decoder::new(cursor);
		decoder.set_transformations(transformations);
		let mut reader = decoder.read_info().map_err(|_| LoadErrors::FailedToProcess)?;
		let Some(size) = reader.output_buffer_size() else {
			return Err(LoadErrors::FailedToProcess);
		};
		let mut buffer = Vec::with_capacity_in(size, allocator);
		buffer.resize(size, 0);
		let info = reader.next_frame(&mut buffer).map_err(|_| LoadErrors::FailedToProcess)?;
		buffer.truncate(info.buffer_size());

		let extent = Extent::rectangle(info.width, info.height);
		let gamma = png_gamma(reader.info(), semantic);
		let (channels, encoding) = png_source_layout(info.color_type, info.bit_depth)?;
		let description = ImageDescription {
			semantic,
			gamma,
			generate_mipmaps: false,
		};
		let source = ImageSource::new(extent, channels, encoding, &buffer);
		let (asset, data) = process_image_in(url, description, source, allocator).map_err(|_| LoadErrors::FailedToProcess)?;

		context.store_primary(asset, &data).await
	}
}

impl Default for PNGAssetHandler {
	fn default() -> Self {
		Self::new()
	}
}

/// Determines the image gamma from PNG metadata before falling back to the asset semantic.
fn png_gamma(info: &png::Info<'_>, semantic: crate::processors::processor::implementations::image::Semantic) -> Gamma {
	info.source_gamma
		.map(|g| {
			if g.into_scaled() == 45455 {
				Gamma::SRGB
			} else {
				Gamma::Linear
			}
		})
		.unwrap_or(gamma_from_semantic(semantic))
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

	/// Encodes a small RGBA8 albedo image so the PNG decoder sees real 8-bit color data.
	fn generated_rgba8_albedo_png() -> Vec<u8> {
		let mut png = Vec::new();

		{
			let mut encoder = png::Encoder::new(&mut png, 4, 4);

			encoder.set_color(png::ColorType::Rgba);

			encoder.set_depth(png::BitDepth::Eight);

			let mut writer = encoder.write_header().expect("generated PNG header should encode");

			let pixels = [0x20, 0x80, 0xe0, 0xff].repeat(16);

			writer.write_image_data(&pixels).expect("generated PNG pixels should encode");
		}

		png
	}

	#[r#async::test]
	async fn asset_manager_bakes_generated_rgba8_albedo_png() {
		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();

		let resource_storage_backend = resource::storage_backend::tests::TestStorageBackend::new();

		asset_storage_backend.add_file("generated_albedo.png", &generated_rgba8_albedo_png());

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(PNGAssetHandler::new());

		asset_manager
			.bake("generated_albedo.png")
			.await
			.expect("generated 8-bit PNG should bake");

		let resource = resource_storage_backend
			.get_resource(ResourceId::new("generated_albedo.png"))
			.expect("baked PNG resource should be stored");

		let image: Image = crate::from_slice(&resource.resource).expect("baked PNG metadata should deserialize");

		assert_eq!(resource.class, "Image");
		assert_eq!(image.extent, [4, 4, 0]);
		assert_eq!(image.gamma, Gamma::SRGB);
		assert_eq!(image.format, Formats::BC7SRGB);
		assert_eq!(
			resource_storage_backend
				.get_resource_data_by_name(ResourceId::new("generated_albedo.png"))
				.expect("baked PNG data should be stored")
				.len(),
			16
		);
	}

	/// Encodes a small RGB16 normal map so the PNG decoder sees real 16-bit file data.
	fn generated_rgb16_normal_png() -> Vec<u8> {
		let mut png = Vec::new();

		{
			let mut encoder = png::Encoder::new(&mut png, 4, 4);

			encoder.set_color(png::ColorType::Rgb);

			encoder.set_depth(png::BitDepth::Sixteen);

			let mut writer = encoder.write_header().expect("generated PNG header should encode");

			let normal = [0x80, 0x00, 0x80, 0x00, 0xff, 0xff];

			let pixels = normal.repeat(16);

			writer.write_image_data(&pixels).expect("generated PNG pixels should encode");
		}

		png
	}

	#[r#async::test]
	async fn asset_manager_bakes_generated_16_bit_normal_png() {
		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();

		let resource_storage_backend = resource::storage_backend::tests::TestStorageBackend::new();

		asset_storage_backend.add_file("generated_normal.png", &generated_rgb16_normal_png());

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend.clone());

		asset_manager.add_asset_handler(PNGAssetHandler::new());

		asset_manager
			.bake("generated_normal.png")
			.await
			.expect("generated 16-bit PNG should bake");

		let resource = resource_storage_backend
			.get_resource(ResourceId::new("generated_normal.png"))
			.expect("baked PNG resource should be stored");

		let image: Image = crate::from_slice(&resource.resource).expect("baked PNG metadata should deserialize");

		assert_eq!(resource.class, "Image");
		assert_eq!(image.extent, [4, 4, 0]);
		assert_eq!(image.format, Formats::BC5);
		assert_eq!(
			resource_storage_backend
				.get_resource_data_by_name(ResourceId::new("generated_normal.png"))
				.expect("baked PNG data should be stored")
				.len(),
			16
		);
	}
}

use utils::Extent;

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	processors::processor::implementations::image::{
		ImageDescription, ImageSource, SourceChannels, SourceEncoding, gamma_from_semantic, guess_semantic_from_name,
		process_image_in,
	},
	types::Gamma,
};
