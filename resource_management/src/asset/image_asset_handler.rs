
use std::any::Any;

use maths_rs::vec::Vec2;
use utils::Extent;

use crate::{types::{CompressionSchemes, Formats, Gamma, Image}, Description, GenericResourceSerialization, Resource, StorageBackend};

use super::{asset_handler::AssetHandler, asset_manager::AssetManager, AssetResolver};

pub struct ImageAssetHandler {
}

impl ImageAssetHandler {
	pub fn new() -> ImageAssetHandler {
		ImageAssetHandler {}
	}
}

impl AssetHandler for ImageAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "png" || r#type == "Image" || r#type == "image/png"
	}

	fn load<'a>(&'a self, _: &'a AssetManager, asset_resolver: &'a dyn AssetResolver, storage_backend: &'a dyn StorageBackend, url: &'a str, _: Option<&'a json::JsonValue>) -> utils::SendSyncBoxedFuture<'a, Result<Option<GenericResourceSerialization>, String>> {
		Box::pin(async move {
			if let Some(dt) = asset_resolver.get_type(url) {
				if dt != "png" { return Err("Not my type".to_string()); }
			}

			let (data, dt) = asset_resolver.resolve(url).await.ok_or("Failed to resolve asset".to_string())?;

			let extent;
			let format;
			let mut buffer;
			let gamma;

			match dt.as_str() {
				"png" | "image/png" => {
					let mut decoder = png::Decoder::new(data.as_slice());
					if true { // TODO: make this a setting
						// decoder.set_transformations(png::Transformations::normalize_to_color8());
					}
					let mut reader = decoder.read_info().unwrap();
					buffer = vec![0; reader.output_buffer_size()];
					let info = reader.next_frame(&mut buffer).unwrap();
		
					extent = Extent::rectangle(info.width, info.height);

					gamma = reader.info().gama_chunk.map(|gamma| {
						if gamma.into_scaled() == 45455 {
							Gamma::SRGB
						} else {
							Gamma::Linear
						}
					});

					match info.bit_depth {
						png::BitDepth::Sixteen => {
							for i in 0..buffer.len() / 2 {
								buffer.swap(i * 2, i * 2 + 1);
							}							
						}
						_ => { panic!("Unsupported bit depth"); }
					}

					format = match info.color_type {
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
				}
				_ => { return Err("Not my type".to_string()); }
			
			}

			assert_eq!(extent.depth(), 1); // TODO: support 3D textures

			let semantic = guess_semantic_from_name(url);

			let gamma = gamma.unwrap_or(if semantic == Semantic::Albedo { Gamma::SRGB } else { Gamma::Linear });

			let (image, data) = self.produce(&ImageDescription {
				format,
				extent,
				semantic,
				gamma,
			}, &buffer);

			let resource_document = GenericResourceSerialization::new(url, image);

			storage_backend.store(&resource_document, &data).await;

			Ok(Some(resource_document))
		})
	}

	fn produce<'a>(&'a self, id: &'a str, description: &'a dyn Description, data: &'a [u8]) -> utils::SendSyncBoxedFuture<'a, Result<(GenericResourceSerialization, Box<[u8]>), String>> {
		Box::pin(async move {
			if let Some(description) = (description as &dyn Any).downcast_ref::<ImageDescription>() {
				let (resource, buffer) = self.produce(description, data);
				Ok((GenericResourceSerialization::new(id, resource), buffer))
			} else {
				Err("Invalid description".to_string())
			}
		})
	}
}

pub fn guess_semantic_from_name(name: &str) -> Semantic {
	if name.contains("Base_color") || name.contains("Albedo") || name.contains("Diffuse") { Semantic::Albedo }
	else if name.contains("Normal") { Semantic::Normal }
	else if name.contains("Metallic") { Semantic::Metallic }
	else if name.contains("Roughness") { Semantic::Roughness }
	else if name.contains("Emissive") { Semantic::Emissive }
	else if name.contains("Height") { Semantic::Height }
	else if name.contains("Opacity") { Semantic::Opacity }
	else if name.contains("Displacement") { Semantic::Displacement }
	else if name.contains("AO") { Semantic::AO }
	else { Semantic::Other }
}

impl ImageAssetHandler {
	fn produce(&self, description: &ImageDescription, buffer: &[u8]) -> (Image, Box<[u8]>) {
		let ImageDescription { format, extent, semantic, gamma } = description;

		let compress = match semantic {
			Semantic::Albedo => true,
			Semantic::Normal => true,
			_ => false,
		};

		let (data, format, compression) = match format {
			Formats::RGB8 => {
				let mut buf: Vec<u8> = Vec::with_capacity(extent.width() as usize * extent.height() as usize * 4);

				for y in 0..extent.height() {
					for x in 0..extent.width() {
						let index = ((x + y * extent.width()) * 3) as usize;
						buf.push(buffer[index + 0]);
						buf.push(buffer[index + 1]);
						buf.push(buffer[index + 2]);
						buf.push(0xFF);
					}
				}

				if compress { // TODO: make this a setting
					let rgba_surface = intel_tex_2::RgbaSurface {
						data: &buf,
						width: extent.width(),
						height: extent.height(),
						stride: extent.width() * 4,
					};
		
					let settings = intel_tex_2::bc7::opaque_ultra_fast_settings();

					(intel_tex_2::bc7::compress_blocks(&settings, &rgba_surface), Formats::RGBA8, Some(CompressionSchemes::BC7))
				} else {
					(buf, Formats::RGBA8, None)
				}
			}
			Formats::RGB16 => {
				if compress {
					match semantic {
						Semantic::Normal => {
							assert_eq!(buffer.len(), extent.width() as usize * extent.height() as usize * 6);

							let mut buf: Vec<u8> = Vec::with_capacity(extent.width() as usize * extent.height() as usize * 4);

							for y in 0..extent.height() {
								for x in 0..extent.width() {
									let index = ((x + y * extent.width()) * 6) as usize;
									let x = u16::from_le_bytes([buffer[index + 0], buffer[index + 1]]);
									let y = u16::from_le_bytes([buffer[index + 2], buffer[index + 3]]);
									let x: u8 = (x / 256) as u8;
									let y: u8 = (y / 256) as u8;
									buf.push(x); buf.push(y); buf.push(0x00); buf.push(0xFF);
								}
							}

							let rgba_surface = intel_tex_2::RgbaSurface {
								data: &buf,
								width: extent.width(),
								height: extent.height(),
								stride: extent.width() * 4,
							};

							(intel_tex_2::bc5::compress_blocks(&rgba_surface), Formats::RG8, Some(CompressionSchemes::BC5))
						}
						_ => {
							let mut buf: Vec<u8> = Vec::with_capacity(extent.width() as usize * extent.height() as usize * 8);

							for y in 0..extent.height() {
								for x in 0..extent.width() {
									let index = ((x + y * extent.width()) * 6) as usize;
									buf.push(buffer[index + 0]); buf.push(buffer[index + 1]);
									buf.push(buffer[index + 2]); buf.push(buffer[index + 3]);
									buf.push(buffer[index + 4]); buf.push(buffer[index + 5]);
									buf.push(0xFF); buf.push(0xFF);
								}
							}

							let rgba_surface = intel_tex_2::RgbaSurface {
								data: &buf,
								width: extent.width(),
								height: extent.height(),
								stride: extent.width() * 8,
							};

							(intel_tex_2::bc7::compress_blocks(&intel_tex_2::bc7::opaque_ultra_fast_settings(), &rgba_surface), Formats::RGBA8, Some(CompressionSchemes::BC7))
						}
					}
				} else {
					let mut buf: Vec<u8> = Vec::with_capacity(extent.width() as usize * extent.height() as usize * 8);

					for y in 0..extent.height() {
						for x in 0..extent.width() {
							let index = ((x + y * extent.width()) * 6) as usize;
							buf.push(buffer[index + 0]); buf.push(buffer[index + 1]);
							buf.push(buffer[index + 2]); buf.push(buffer[index + 3]);
							buf.push(buffer[index + 4]); buf.push(buffer[index + 5]);
							buf.push(0xFF); buf.push(0xFF);
						}
					}

					(buf, Formats::RGBA16, None)
				}
			}
			_ => {
				panic!("Unsupported format")
			}
		};

		(Image {
			format,
			extent: extent.as_array(),
			compression,
			gamma: *gamma,
		},
		data.into())
	}
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Semantic {
	Albedo,
	Normal,
	Metallic,
	Roughness,
	Emissive,
	Height,
	Opacity,
	Displacement,
	AO,
	Other,
}

pub struct ImageDescription {
	pub format: Formats,
	pub extent: Extent,
	pub gamma: Gamma,
	pub semantic: Semantic,
}

impl Description for ImageDescription {
	// type Resource = Image;
	fn get_resource_class() -> &'static str {
		"Image"
	}
}

#[cfg(test)]
mod tests {
	use super::ImageAssetHandler;
	use crate::asset::{asset_handler::AssetHandler, asset_manager::AssetManager, tests::{TestAssetResolver, TestStorageBackend}};

	#[test]
	fn load_image() {
		let asset_manager = AssetManager::new_with_path_and_storage_backend("../assets".into(), TestStorageBackend::new());
		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();
		let asset_handler = ImageAssetHandler::new();

		let url = "patterned_brick_floor_02_diff_2k.png";

		let _ = smol::block_on(asset_handler.load(&asset_manager, &asset_resolver, &storage_backend, &url, None)).unwrap().expect("Image asset handler did not handle asset");

		let generated_resources = storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 1);

		let resource = &generated_resources[0];

		assert_eq!(resource.id, "patterned_brick_floor_02_diff_2k.png");
		assert_eq!(resource.class, "Image");
	}

	#[test]
	fn load_16_bit_normal_image() {
		let asset_manager = AssetManager::new_with_path_and_storage_backend("../assets".into(), TestStorageBackend::new());
		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();
		let asset_handler = ImageAssetHandler::new();

		let url = "Revolver_Normal.png";

		let _ = smol::block_on(asset_handler.load(&asset_manager, &asset_resolver, &storage_backend, &url, None)).unwrap().expect("Image asset handler did not handle asset");

		let generated_resources = storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 1);

		let resource = &generated_resources[0];

		assert_eq!(resource.id, url);
		assert_eq!(resource.class, "Image");
	}
}