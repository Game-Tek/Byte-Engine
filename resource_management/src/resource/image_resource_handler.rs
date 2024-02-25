use polodb_core::bson;
use serde::Deserialize;
use smol::{fs::File, future::FutureExt, io::AsyncReadExt};

use crate::{types::Image, GenericResourceResponse, GenericResourceSerialization, Resource, ResourceResponse, Stream, TypedResourceDocument};

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

	fn read<'a>(&'a self, mut resource: GenericResourceResponse<'a>, mut reader: Box<dyn ResourceReader>,) -> utils::BoxedFuture<'a, Option<ResourceResponse>> {
		Box::pin(async move {
			let image_resource = Image::deserialize(bson::Deserializer::new(resource.resource.clone().into())).ok()?;

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
			}

			Some(ResourceResponse::new(resource, image_resource))
		})
	}
}

#[cfg(test)]
mod tests {
	use crate::{asset::{asset_handler::AssetHandler, image_asset_handler::ImageAssetHandler, tests::{TestAssetResolver, TestStorageBackend},}, StorageBackend};

	use super::*;

	#[test]
	fn load_local_image() {
		// Create resource from asset

		let image_asset_handler = ImageAssetHandler::new();

		let url = "patterned_brick_floor_02_diff_2k.png";
		let doc = json::object! {
			"url": url,
		};

		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();

		smol::block_on(image_asset_handler.load(&asset_resolver, &storage_backend, url, &doc)).expect("Image asset handler did not handle asset").expect("Image asset handler failed to load asset");

		// Load resource from storage

		let image_resource_handler = ImageResourceHandler::new();

		let (resource, mut reader) = smol::block_on(storage_backend.read(url)).expect("Failed to read asset from storage");

		let resource = smol::block_on(image_resource_handler.read(resource, reader,)).expect("Failed to read image resource");

		let image = resource.resource.downcast_ref::<Image>().unwrap();

		assert_eq!(image.extent, [2048, 2048, 1]);
	}
}