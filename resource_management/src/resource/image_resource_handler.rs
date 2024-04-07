use polodb_core::bson;
use serde::Deserialize;


use crate::{types::Image, GenericResourceResponse, ResourceResponse, StorageBackend};

use super::resource_handler::{ReadTargets, ResourceHandler, ResourceReader};

pub struct ImageResourceHandler {

}

impl ImageResourceHandler {
	pub fn new() -> Self {
		Self {}
	}
}

impl ResourceHandler for ImageResourceHandler {
	fn get_handled_resource_classes<'a>(&self,) -> &'a [&'a str] {
		&["Image"]
	}

	fn read<'s, 'a, 'b>(&'s self, mut resource: GenericResourceResponse<'a>, reader: Option<Box<dyn ResourceReader>>, _: &'b dyn StorageBackend) -> utils::BoxedFuture<'b, Option<ResourceResponse<'a>>> where 'a: 'b {
		Box::pin(async move {
			let image_resource = Image::deserialize(bson::Deserializer::new(resource.resource.clone().into())).ok()?;

			if let Some(mut reader) = reader {
				if let Some(read_target) = &mut resource.read_target {
					match read_target {
						ReadTargets::Buffer(buffer) => {
							reader.read_into(0, buffer).await?;
						},
						_ => {
							return None;
						}
						
					}
				} else {
					let mut buffer = Vec::with_capacity(resource.size);
					unsafe {
						buffer.set_len(resource.size);
					}
					reader.read_into(0, &mut buffer).await?;
					resource.set_box_buffer(buffer.into());
				}
			}

			Some(ResourceResponse::new(resource, image_resource))
		})
	}
}

#[cfg(test)]
mod tests {
	use crate::asset::{asset_handler::AssetHandler, asset_manager::AssetManager, image_asset_handler::ImageAssetHandler, tests::{TestAssetResolver, TestStorageBackend}};

	use super::*;

	#[test]
	fn load_local_image() {
		// Create resource from asset

		let image_asset_handler = ImageAssetHandler::new();

		let url = "patterned_brick_floor_02_diff_2k.png";

		let asset_manager = AssetManager::new();
		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();

		smol::block_on(image_asset_handler.load(&asset_manager, &asset_resolver, &storage_backend, url, None)).expect("Image asset handler did not handle asset").expect("Image asset handler failed to load asset");

		// Load resource from storage

		let image_resource_handler = ImageResourceHandler::new();

		let (resource, reader) = smol::block_on(storage_backend.read(url)).expect("Failed to read asset from storage");

		let resource = smol::block_on(image_resource_handler.read(resource, Some(reader), &storage_backend)).expect("Failed to read image resource");

		assert!(resource.get_buffer().is_some());

		let image = resource.resource.downcast_ref::<Image>().unwrap();

		assert_eq!(image.extent, [2048, 2048, 1]);
	}
}