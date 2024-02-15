use serde::Deserialize;
use smol::{fs::File, future::FutureExt, io::AsyncReadExt};

use crate::{types::Image, Resource, Stream};

use super::{resource_handler::ResourceHandler,};

pub struct ImageResourceHandler {

}

impl ImageResourceHandler {
	pub fn new() -> Self {
		Self {}
	}
}

impl ResourceHandler for ImageResourceHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool {
		match resource_type {
			"image/png" => true,
			"png" => true,
			"Image" => true,
			_ => false
		}
	}

	fn read<'a>(&'a self, _resource: &'a dyn Resource, file: &'a mut File, buffers: &'a mut [Stream<'a>]) -> utils::BoxedFuture<'_, ()> {
		Box::pin(async move {
			file.read_exact(buffers[0].buffer).await.unwrap()
		})
	}

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn Resource> + Send>)> {
		vec![("Image", Box::new(|document| {
			let texture = Image::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap();
			Box::new(texture)
		}))]
	}
}

#[cfg(test)]
mod tests {
	use crate::{resource::resource_manager::ResourceManager, types::Image};

use super::*;

	#[test]
	fn load_net_image() {
		let mut resource_manager = ResourceManager::new();

		resource_manager.add_resource_handler(ImageResourceHandler::new());

		// smol::block_on(async {
		// 	Timer::after(std::time::Duration::from_secs(60 * 3)).await;
		// });

		let (response, _) = smol::block_on(resource_manager.get("https://camo.githubusercontent.com/a49890a2fa4559f38b13e6427defe7579aee065a9a3f7ee37cf7cb86295bab79/68747470733a2f2f692e696d6775722e636f6d2f56525261434f702e706e67")).expect("Failed to load image");

		assert_eq!(response.resources.len(), 1);

		let resource_container = &response.resources[0];
		let resource = &resource_container.resource;

		let texture_info = resource.downcast_ref::<Image>().unwrap();

		assert_eq!(texture_info.extent, [4096, 1024, 1]);
	}

	#[test]
	fn load_local_image() {
		let mut resource_manager = ResourceManager::new();

		resource_manager.add_resource_handler(ImageResourceHandler::new());

		let (response, _) = smol::block_on(resource_manager.get("patterned_brick_floor_02_diff_2k")).expect("Failed to load image");

		assert_eq!(response.resources.len(), 1);

		let resource_container = &response.resources[0];
		let resource = &resource_container.resource;

		let texture_info = resource.downcast_ref::<Image>().unwrap();

		assert_eq!(texture_info.extent, [2048, 2048, 1]);
	}
}