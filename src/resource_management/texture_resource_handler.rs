use std::io::Read;

use serde::{Serialize, Deserialize};

use crate::resource_management::GenericResourceSerialization;

use super::{Resource, ProcessedResources, resource_manager::ResourceManager, resource_handler::ResourceHandler};

pub(crate) struct ImageResourceHandler {

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
			_ => false
		}
	}

	fn process(&self, resource_manager: &ResourceManager, asset_url: &str,) -> Result<Vec<ProcessedResources>, String> {
		let (bytes, _) = resource_manager.read_asset_from_source(asset_url).unwrap();
		let mut decoder = png::Decoder::new(bytes.as_slice());
		decoder.set_transformations(png::Transformations::normalize_to_color8());
		let mut reader = decoder.read_info().unwrap();
		let mut buffer = vec![0; reader.output_buffer_size()];
		let info = reader.next_frame(&mut buffer).unwrap();

		let extent = crate::Extent { width: info.width, height: info.height, depth: 1, };

		assert_eq!(info.color_type, png::ColorType::Rgb);
		assert_eq!(info.bit_depth, png::BitDepth::Eight);

		let mut buf: Vec<u8> = Vec::with_capacity(extent.width as usize * extent.height as usize * 4);

		// convert rgb to rgba
		for y in 0..extent.height {
			for x in 0..extent.width {
				let index = ((x + y * extent.width) * 3) as usize;
				buf.push(buffer[index]);
				buf.push(buffer[index + 1]);
				buf.push(buffer[index + 2]);
				buf.push(255);
			}
		}

		assert_eq!(extent.depth, 1); // TODO: support 3D textures

		let rgba_surface = intel_tex_2::RgbaSurface {
			data: &buf,
			width: extent.width,
			height: extent.height,
			stride: extent.width * 4,
		};

		let settings = intel_tex_2::bc7::opaque_ultra_fast_settings();

		let resource_document = GenericResourceSerialization::new(asset_url.to_string(), Texture{
			extent: crate::Extent{ width: extent.width, height: extent.height, depth: extent.depth },
			compression: Some(CompressionSchemes::BC7),
		});

		Ok(vec![ProcessedResources::Generated((resource_document, intel_tex_2::bc7::compress_blocks(&settings, &rgba_surface)))])
	}

	fn read(&self, _resource: &Box<dyn Resource>, file: &mut std::fs::File, buffers: &mut [super::Stream]) {
		file.read_exact(buffers[0].buffer).unwrap();
	}

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn Resource> + Send>)> {
		vec![("Texture", Box::new(|document| {
			let texture = Texture::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap();
			Box::new(texture)
		}))]
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub enum CompressionSchemes {
	BC7,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Texture {
	pub compression: Option<CompressionSchemes>,
	pub extent: crate::Extent,
}

impl Resource for Texture {
	fn get_class(&self) -> &'static str { "Texture"	}
}

#[cfg(test)]
mod tests {
	use crate::resource_management::resource_manager::ResourceManager;

	use super::*;

	#[test]
	fn load_net_image() {
		let mut resource_manager = ResourceManager::new();

		let (response, _) = resource_manager.get("https://camo.githubusercontent.com/dca6cdb597abc9c7ff4a0e066e6c35eb70b187683fbff2208d0440b4ef6c5a30/68747470733a2f2f692e696d6775722e636f6d2f56525261434f702e706e67").expect("Failed to load image");

		assert_eq!(response.resources.len(), 1);

		let resource_container = &response.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Texture>());

		let texture_info = resource.downcast_ref::<Texture>().unwrap();

		assert_eq!(texture_info.extent, crate::Extent{ width: 4096, height: 1024, depth: 1 });
	}

	#[test]
	fn load_local_image() {
		let mut resource_manager = ResourceManager::new();

		let (response, _) = resource_manager.get("patterned_brick_floor_02_diff_2k").expect("Failed to load image");

		assert_eq!(response.resources.len(), 1);

		let resource_container = &response.resources[0];
		let resource = &resource_container.resource;

		assert_eq!(resource.type_id(), std::any::TypeId::of::<Texture>());

		let texture_info = resource.downcast_ref::<Texture>().unwrap();

		assert!(texture_info.extent.width == 2048 && texture_info.extent.height == 2048 && texture_info.extent.depth == 1);
	}
}