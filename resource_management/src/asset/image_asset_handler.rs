use utils::Extent;

use crate::{types::{CompressionSchemes, Formats, Image}, GenericResourceSerialization};

use super::{asset_handler::AssetHandler, read_asset_from_source};

struct ImageAssetHandler {
}

impl ImageAssetHandler {
	fn new() -> ImageAssetHandler {
		ImageAssetHandler {}
	}
}

impl AssetHandler for ImageAssetHandler {
	fn load(&self, url: &str, json: &json::JsonValue) -> utils::BoxedFuture<Option<Result<(), String>>> {
		async move {
			let (data, dt) = read_asset_from_source(url, None).await.unwrap();

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

			// Ok(vec![ProcessedResources::Generated((resource_document, data))])

			Ok(())
		}.boxed()
	}
}