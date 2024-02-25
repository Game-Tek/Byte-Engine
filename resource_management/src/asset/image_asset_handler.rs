use smol::future::FutureExt;
use utils::Extent;

use crate::{types::{CompressionSchemes, Formats, Image}, GenericResourceSerialization, StorageBackend};

use super::{asset_handler::AssetHandler, AssetResolver,};

pub struct ImageAssetHandler {
}

impl ImageAssetHandler {
	pub fn new() -> ImageAssetHandler {
		ImageAssetHandler {}
	}
}

impl AssetHandler for ImageAssetHandler {
	fn load<'a>(&'a self, asset_resolver: &'a dyn AssetResolver, storage_backend: &'a dyn StorageBackend, url: &'a str, json: &'a json::JsonValue) -> utils::BoxedFuture<'a, Option<Result<(), String>>> {
		async move {
			if let Some(dt) = asset_resolver.get_type(url) {
				if dt != "png" { return None; }
			}

			let (data, dt) = asset_resolver.resolve(url).await?;

			if dt != "png" { return None; }

			let mut decoder = png::Decoder::new(data.as_slice());
			decoder.set_transformations(png::Transformations::normalize_to_color8());
			let mut reader = decoder.read_info().unwrap();
			let mut buffer = vec![0; reader.output_buffer_size()];
			let info = reader.next_frame(&mut buffer).unwrap();

			let extent = Extent::rectangle(info.width, info.height);

			assert_eq!(extent.depth(), 1); // TODO: support 3D textures

			let format = match info.color_type {
				png::ColorType::Rgb => {
					match info.bit_depth {
						png::BitDepth::Eight => Formats::RGB8,
						png::BitDepth::Sixteen => Formats::RGB16,
						_ => { panic!("Unsupported bit depth") }
					}
				}
				png::ColorType::Rgba => {
					match info.bit_depth {
						png::BitDepth::Eight => Formats::RGBA8,
						png::BitDepth::Sixteen => Formats::RGBA16,
						_ => { panic!("Unsupported bit depth") }
					}
				}
				_ => { panic!("Unsupported color type") }
			};
			
			let (data, compression) = match format {
				Formats::RGB8 => {
					let mut buf: Vec<u8> = Vec::with_capacity(extent.width() as usize * extent.height() as usize * 4);

					for y in 0..extent.height() {
						for x in 0..extent.width() {
							let index = ((x + y * extent.width()) * 3) as usize;
							buf.push(data[index + 0]);
							buf.push(data[index + 1]);
							buf.push(data[index + 2]);
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
							buf.push(data[index + 0]); buf.push(data[index + 1]);
							buf.push(data[index + 2]); buf.push(data[index + 3]);
							buf.push(data[index + 4]); buf.push(data[index + 5]);
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
					(data, None)
				}
			};

			let resource_document = GenericResourceSerialization::new(url.to_string(), Image {
				format,
				extent: extent.as_array(),
				compression,
			});

			storage_backend.store(resource_document, &buffer);

			Some(Ok(()))
		}.boxed()
	}
}

#[cfg(test)]
mod tests {
	use super::{ImageAssetHandler};
	use crate::asset::{asset_handler::AssetHandler, tests::{TestAssetResolver, TestStorageBackend}};

	#[test]
	fn load_image() {
		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();
		let asset_handler = ImageAssetHandler::new();

		let url = "patterned_brick_floor_02_diff_2k.png";
		let doc = json::object! {
			"url": url,
		};

		let result = smol::block_on(asset_handler.load(&asset_resolver, &storage_backend, &url, &doc)).expect("Image asset handler did not handle asset");

		let generated_resources = storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 1);

		let resource = &generated_resources[0];

		assert_eq!(resource.url, "patterned_brick_floor_02_diff_2k.png");
		assert_eq!(resource.class, "Image");
	}
}