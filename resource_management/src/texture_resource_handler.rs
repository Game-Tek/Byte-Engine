use serde::{Serialize, Deserialize};
use smol::{fs::File, future::FutureExt, io::AsyncReadExt};
use utils::Extent;

use crate::{CreateInfo, CreateResource, GenericResourceSerialization};

use super::{Resource, ProcessedResources, resource_manager::ResourceManager, resource_handler::ResourceHandler, Stream};

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
			"Texture" => true,
			_ => false
		}
	}

	fn process<'a>(&'a self, resource_manager: &'a ResourceManager, asset_url: &'a str,) -> utils::BoxedFuture<Result<Vec<ProcessedResources>, String>> {
		Box::pin(async move {
			let (bytes, _) = resource_manager.read_asset_from_source(asset_url).await.unwrap();
			let mut decoder = png::Decoder::new(bytes.as_slice());
			decoder.set_transformations(png::Transformations::normalize_to_color8());
			let mut reader = decoder.read_info().unwrap();
			let mut buffer = vec![0; reader.output_buffer_size()];
			let info = reader.next_frame(&mut buffer).unwrap();
	
			let extent = Extent::rectangle(info.width, info.height,);
	
			assert_eq!(info.color_type, png::ColorType::Rgb);
			assert_eq!(info.bit_depth, png::BitDepth::Eight);
	
			let mut buf: Vec<u8> = Vec::with_capacity(extent.width() as usize * extent.height() as usize * 4);
	
			// convert rgb to rgba
			for y in 0..extent.height() {
				for x in 0..extent.width() {
					let index = ((x + y * extent.width()) * 3) as usize;
					buf.push(buffer[index]);
					buf.push(buffer[index + 1]);
					buf.push(buffer[index + 2]);
					buf.push(255);
				}
			}
	
			assert_eq!(extent.depth(), 1); // TODO: support 3D textures
	
			let rgba_surface = intel_tex_2::RgbaSurface {
				data: &buf,
				width: extent.width(),
				height: extent.height(),
				stride: extent.width() * 4,
			};
	
			let settings = intel_tex_2::bc7::opaque_ultra_fast_settings();
	
			let resource_document = GenericResourceSerialization::new(asset_url.to_string(), Texture{
				format: Formats::RGB8,
				extent: extent.as_array(),
				compression: Some(CompressionSchemes::BC7),
			});
	
			Ok(vec![ProcessedResources::Generated((resource_document, intel_tex_2::bc7::compress_blocks(&settings, &rgba_surface)))])
		})
	}

	fn read<'a>(&'a self, _resource: &'a dyn Resource, file: &'a mut File, buffers: &'a mut [Stream<'a>]) -> utils::BoxedFuture<'_, ()> {
		Box::pin(async move {
			file.read_exact(buffers[0].buffer).await.unwrap()
		})
	}

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn Resource> + Send>)> {
		vec![("Texture", Box::new(|document| {
			let texture = Texture::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap();
			Box::new(texture)
		}))]
	}

	fn create_resource<'a>(&'a self, resource: &'a CreateInfo, resource_manager: &'a crate::resource_manager::ResourceManager) -> utils::BoxedFuture<Result<Vec<ProcessedResources>, String>> {
		async move {
			let image_info = resource.info.downcast_ref::<CreateImage>().ok_or("Invalid resource info")?;

			let bytes = resource.data;
			let asset_url = resource.name;

			let extent = Extent::rectangle(image_info.extent[0], image_info.extent[1]);

			assert_eq!(extent.depth(), 1); // TODO: support 3D textures
			
			let (data, compression) = match image_info.format {
				Formats::RGB8 => {
					let mut buf: Vec<u8> = Vec::with_capacity(extent.width() as usize * extent.height() as usize * 4);

					for y in 0..extent.height() {
						for x in 0..extent.width() {
							let index = ((x + y * extent.width()) * 3) as usize;
							buf.push(bytes[index + 0]);
							buf.push(bytes[index + 1]);
							buf.push(bytes[index + 2]);
							buf.push(255);
						}
					}

					let rgba_surface = intel_tex_2::RgbaSurface {
						data: &buf,
						width: extent.width(),
						height: extent.height(),
						stride: extent.width() * 4,
					};
		
					let settings = intel_tex_2::bc7::opaque_ultra_fast_settings();

					(intel_tex_2::bc7::compress_blocks(&settings, &rgba_surface), Some(CompressionSchemes::BC7))
				}
				Formats::RGB16 => {
					let mut buf: Vec<u8> = Vec::with_capacity(extent.width() as usize * extent.height() as usize * 8);

					for y in 0..extent.height() {
						for x in 0..extent.width() {
							let index = ((x + y * extent.width()) * 6) as usize;
							buf.push(bytes[index + 0]); buf.push(bytes[index + 1]);
							buf.push(bytes[index + 2]); buf.push(bytes[index + 3]);
							buf.push(bytes[index + 4]); buf.push(bytes[index + 5]);
							buf.push(255); buf.push(255);
						}
					}

					let rgba_surface = intel_tex_2::RgbaSurface {
						data: &buf,
						width: extent.width(),
						height: extent.height(),
						stride: extent.width() * 8,
					};
		
					let settings = intel_tex_2::bc7::opaque_ultra_fast_settings();

					(intel_tex_2::bc7::compress_blocks(&settings, &rgba_surface), Some(CompressionSchemes::BC7))
				}
				_ => {
					(bytes.to_vec(), None)
				}
			};

			let resource_document = GenericResourceSerialization::new(asset_url.to_string(), Texture{
				format: image_info.format,
				extent: extent.as_array(),
				compression,
			});

			Ok(vec![ProcessedResources::Generated((resource_document, data))])
		}.boxed()
	}
}

pub struct CreateImage {
	pub format: Formats,
	pub extent: [u32; 3],
}

impl CreateResource for CreateImage {}

#[derive(Debug, Serialize, Deserialize)]
pub enum CompressionSchemes {
	BC7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Formats {
	RGB8,
	RGBA8,
	RGB16,
	RGBA16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Texture {
	pub compression: Option<CompressionSchemes>,
	pub format: Formats,
	pub extent: [u32; 3],
}

impl Resource for Texture {
	fn get_class(&self) -> &'static str { "Texture"	}
}

#[cfg(test)]
mod tests {
	use std::any::Any;

	use crate::resource_manager::ResourceManager;

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

		let texture_info = resource.downcast_ref::<Texture>().unwrap();

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

		let texture_info = resource.downcast_ref::<Texture>().unwrap();

		assert_eq!(texture_info.extent, [2048, 2048, 1]);
	}
}