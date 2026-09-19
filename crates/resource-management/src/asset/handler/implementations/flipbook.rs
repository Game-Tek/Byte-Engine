//! Flipbook asset baking.

/// The `FlipbookAssetHandler` struct exists to bake `.flipbook` declarations together with the images they play.
pub struct FlipbookAssetHandler;

/// The `FlipbookSource` struct is the authored form of a flipbook, naming its images by asset ID.
#[derive(serde::Deserialize)]
struct FlipbookSource {
	frames_per_second: u32,
	images: Vec<String>,
}

impl AssetHandler for FlipbookAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "flipbook"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (source, format) = context.resolve(id).await?;

		if format != "flipbook" {
			return Err(LoadErrors::UnsupportedType);
		}

		let source: FlipbookSource = serde_json::from_slice(&source).map_err(|error| {
			log::error!(
				"Flipbook asset could not be parsed for '{}': {error}. The most likely cause is invalid flipbook JSON.",
				id.as_ref()
			);

			LoadErrors::FailedToProcess
		})?;

		if source.frames_per_second == 0 || source.images.is_empty() {
			log::error!(
				"Flipbook asset '{}' cannot be played. The most likely cause is a zero `frames_per_second` or an empty `images` list.",
				id.as_ref()
			);

			return Err(LoadErrors::FailedToProcess);
		}

		let images = context.bake_dependencies::<Image>(&source.images, 8).await?;

		let flipbook = FlipbookModel {
			frames_per_second: source.frames_per_second,
			images,
		};

		context.store_primary(ProcessedAsset::new(id, flipbook), &[]).await
	}
}

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	ProcessedAsset,
	resources::{flipbook::FlipbookModel, image::Image},
};

#[cfg(test)]
mod tests {
	use super::FlipbookAssetHandler;
	use crate::{
		asset::{self, handler::implementations::png::PNGAssetHandler, manager::AssetManager},
		r#async, resource,
		resource::resource_manager::ResourceManager,
		resources::flipbook::Flipbook,
	};

	/// Encodes a small RGBA8 image to stand in for one animation frame.
	fn generated_png() -> Vec<u8> {
		let mut png = Vec::new();
		{
			let mut encoder = png::Encoder::new(&mut png, 4, 4);
			encoder.set_color(png::ColorType::Rgba);
			encoder.set_depth(png::BitDepth::Eight);
			let mut writer = encoder.write_header().expect("generated PNG header should encode");
			writer
				.write_image_data(&[0x20, 0x80, 0xe0, 0xff].repeat(16))
				.expect("generated PNG pixels should encode");
		}
		png
	}

	fn asset_manager(
		assets: asset::storage_backend::tests::TestStorageBackend,
		resources: resource::storage_backend::tests::TestStorageBackend,
	) -> AssetManager {
		let mut asset_manager = AssetManager::new(assets, resources);
		asset_manager.add_asset_handler(PNGAssetHandler::new());
		asset_manager.add_asset_handler(FlipbookAssetHandler);
		asset_manager
	}

	#[r#async::test]
	async fn baked_flipbook_is_requested_with_its_rate_and_ordered_images() {
		let assets = asset::storage_backend::tests::TestStorageBackend::new();
		let resources = resource::storage_backend::tests::TestStorageBackend::new();
		assets.add_file("char/run1.png", &generated_png());
		assets.add_file("char/run0.png", &generated_png());
		assets.add_file(
			"char/run.flipbook",
			br#"{"frames_per_second": 12, "images": ["char/run0.png", "char/run1.png"]}"#,
		);

		asset_manager(assets, resources.clone())
			.bake("char/run.flipbook")
			.await
			.expect("flipbook and its images should bake");

		let flipbook = ResourceManager::new(resources)
			.request::<Flipbook>("char/run.flipbook")
			.await
			.expect("baked flipbook should be requested");
		assert_eq!(flipbook.resource().frames_per_second, 12);
		let images = flipbook.resource().images.iter().map(|image| image.id()).collect::<Vec<_>>();
		assert_eq!(images, ["char/run0.png", "char/run1.png"]);
	}

	#[r#async::test]
	async fn flipbooks_that_cannot_play_are_rejected() {
		for source in [
			&br#"{"frames_per_second": 0, "images": ["char/run0.png"]}"#[..],
			&br#"{"frames_per_second": 12, "images": []}"#[..],
		] {
			let assets = asset::storage_backend::tests::TestStorageBackend::new();
			assets.add_file("char/run0.png", &generated_png());
			assets.add_file("char/run.flipbook", source);
			let resources = resource::storage_backend::tests::TestStorageBackend::new();

			assert!(asset_manager(assets, resources).bake("char/run.flipbook").await.is_err());
		}
	}
}
