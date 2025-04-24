use std::any::Any;

use utils::Extent;

use crate::{resources::image::Image, resource, asset, types::{Formats, Gamma}, Description, ProcessedAsset};

use super::{asset_handler::{Asset, AssetHandler, LoadErrors}, asset_manager::AssetManager, resource_id::ResourceIdBase, ResourceId};

pub struct ImageAsset {
    id: String,
    data: Box<[u8]>,
    extent: Extent,
    format: Formats,
    gamma: Gamma,
}

impl Asset for ImageAsset {
    fn requested_assets(&self) -> Vec<String> { vec![] }

    fn load<'a>(&'a self, _: &'a AssetManager, storage_backend: &'a dyn resource::StorageBackend, _: &'a dyn asset::StorageBackend, url: ResourceId<'a>) -> Result<(), String> {
		let semantic = guess_semantic_from_name(url.get_base());

		let format = self.format;
		let extent = self.extent;
		let gamma = self.gamma;

		let buffer = self.data.clone();

		let (image, data) = ImageAssetHandler::produce(&ImageDescription {
			format,
			extent,
			semantic,
			gamma,
		}, buffer);

		let resource_document = ProcessedAsset::new(url, image);

		storage_backend.store(&resource_document, &data).map_err(|_| format!("Failed to store resource"))?;

		Ok(())
    }
}

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

	fn load<'a>(&'a self, _: &'a AssetManager, storage_backend: &'a dyn resource::StorageBackend, asset_storage_backend: &'a dyn asset::StorageBackend, url: ResourceId<'a>,) -> Result<Box<dyn Asset>, LoadErrors> {
		if let Some(dt) = storage_backend.get_type(url) {
			if dt != "png" { return Err(LoadErrors::UnsupportedType); }
		}

		let (data, _, dt) = asset_storage_backend.resolve(url).or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

		let semantic = guess_semantic_from_name(url.get_base());

		let mut buffer;
		let extent;
		let gamma;
		let format;

		match dt.as_str() {
			"png" | "image/png" => {
				let decoder = png::Decoder::new(data.as_ref());
				if true { // TODO: make this a setting
					// decoder.set_transformations(png::Transformations::normalize_to_color8());
				}
				let mut reader = decoder.read_info().unwrap();
				buffer = vec![0u8; reader.output_buffer_size()];
				let info = reader.next_frame(&mut buffer).unwrap();

				extent = Extent::rectangle(info.width, info.height);

				gamma = reader.info().gama_chunk.map(|gamma| {
					if gamma.into_scaled() == 45455 {
						Gamma::SRGB
					} else {
						Gamma::Linear
					}
				}).unwrap_or(gamma_from_semantic(semantic));

				match info.bit_depth {
					png::BitDepth::Eight => {}
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
			_ => { return Err(LoadErrors::UnsupportedType); }
		}

		Ok(Box::new(ImageAsset {
		    id: url.to_string(),
			data: buffer.into(),
			gamma,
			format,
			extent,
		}) as Box<dyn Asset>)
	}

	fn produce<'a>(&'a self, id: ResourceId<'a>, description: &'a dyn Description, data: Box<[u8]>) -> Result<(ProcessedAsset, Box<[u8]>), String> {
		if let Some(description) = (description as &dyn Any).downcast_ref::<ImageDescription>() {
		    let description = description.clone();
			let (resource, buffer) = Self::produce(&description, data);
			Ok((ProcessedAsset::new(id, resource), buffer))
		} else {
			Err("Invalid description".to_string())
		}
	}
}

pub fn guess_semantic_from_name(name: ResourceIdBase) -> Semantic {
    let name = name.as_ref();
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

pub fn gamma_from_semantic(semantic: Semantic) -> Gamma {
	match semantic {
		Semantic::Albedo | Semantic::Other => Gamma::SRGB,
		Semantic::Normal | Semantic::Metallic | Semantic::Roughness | Semantic::Emissive | Semantic::Height | Semantic::Opacity | Semantic::Displacement | Semantic::AO => Gamma::Linear,
	}
}

impl ImageAssetHandler {
	fn produce(description: &ImageDescription, buffer: Box<[u8]>) -> (Image, Box<[u8]>) {
		let ImageDescription { format, extent, semantic, gamma } = description;

		let compress = match semantic {
			Semantic::Albedo | Semantic::Normal => true,
			_ => false,
		};

		let (data, format) = match format {
			Formats::RGB8 => {
				let mut buf: Box<[u8]> = vec![0_u8; extent.width() as usize * extent.height() as usize * 4].into();

				for y in 0..extent.height() {
					let source_row = &buffer[(y * extent.width() * 3) as usize..][..(extent.width() * 3) as usize];
					let dest_row = &mut buf[(y * extent.width() * 4) as usize..][..(extent.width() * 4) as usize];

					for x in 0..extent.width() {
						let source_pixel = &source_row[(x * 3) as usize..][..3];
						let dest_pixel = &mut dest_row[(x * 4) as usize..][..4];
						dest_pixel[..3].copy_from_slice(source_pixel);
						dest_pixel[3] = 0xFF;
					}
				}

				match (compress, semantic) {
					(true, Semantic::Normal) => {
						(buf, Formats::BC5)
					}
					(true, _) => {
						(buf, Formats::BC7)
					}
					(false, _) => {
						(buf, Formats::RGBA8)
					}
				}
			}
			Formats::RGBA8 => {
				match (compress, semantic) {
					(true, Semantic::Normal) => {
						let mut buf: Box<[u8]> = vec![0_u8; extent.width() as usize * extent.height() as usize * 4].into();

						for y in 0..extent.height() {
							let source_row = &buffer[(y * extent.width() * 4) as usize..][..(extent.width() * 4) as usize];
							let dest_row = &mut buf[(y * extent.width() * 4) as usize..][..(extent.width() * 4) as usize];
							for x in 0..extent.width() {
								let source_pixel = &source_row[(x * 4) as usize..][..4];
								let dest_pixel = &mut dest_row[(x * 4) as usize..][..4];
								dest_pixel[..3].copy_from_slice(&source_pixel[..3]);
								dest_pixel[3] = 0xFF;
							}
						}

						(buf, Formats::BC5)
					}
					(compress, _) => {
						if compress {
							(buffer, Formats::BC7)
						} else {
							(buffer, Formats::RGBA8)
						}
					}
				}
			}
			Formats::RGB16 => {
				match (compress, semantic) {
					(true, Semantic::Normal) => {
						let mut buf: Box<[u8]> = vec![0_u8; extent.width() as usize * extent.height() as usize * 4].into();

						for y in 0..extent.height() {
							let source_row = &buffer[(y * extent.width() * 6) as usize..][..(extent.width() * 6) as usize];
							let dest_row = &mut buf[(y * extent.width() * 4) as usize..][..(extent.width() * 4) as usize];
							for x in 0..extent.width() {
								let source_pixel = &source_row[(x * 6) as usize..][..6];
								let dest_pixel = &mut dest_row[(x * 4) as usize..][..4];
								let x = u16::from_le_bytes([source_pixel[0], source_pixel[1]]);
								let y = u16::from_le_bytes([source_pixel[2], source_pixel[3]]);
								let x: u8 = (x / 256) as u8;
								let y: u8 = (y / 256) as u8;
								dest_pixel[0] = x; dest_pixel[1] = y;
								dest_pixel[2] = 0x00; dest_pixel[3] = 0xFF;
							}
						}

						(buf, Formats::BC5)
					}
					(compress, _) => {
						let mut buf: Box<[u8]> = vec![0_u8; extent.width() as usize * extent.height() as usize * 8].into();

						for y in 0..extent.height() {
							let source_row = &buffer[(y * extent.width() * 6) as usize..][..(extent.width() * 6) as usize];
							let dest_row = &mut buf[(y * extent.width() * 8) as usize..][..(extent.width() * 8) as usize];
							for x in 0..extent.width() {
								let source_pixel = &source_row[(x * 6) as usize..][..6];
								let dest_pixel = &mut dest_row[(x * 8) as usize..][..8];
								dest_pixel[..6].copy_from_slice(&source_pixel);
								dest_pixel[6] = 0xFF; dest_pixel[7] = 0xFF;
							}
						}

						if compress {
							(buf, Formats::BC7)
						} else {
							(buf, Formats::RGBA16)
						}
					}
				}
			}
			_ => {
				panic!("Unsupported format: {:#?}", format);
			}
		};

		let data = match format {
			Formats::BC5 => {
				let rgba_surface = intel_tex_2::RgSurface {
					data: &data,
					width: extent.width(),
					height: extent.height(),
					stride: extent.width() * 4,
				};

				intel_tex_2::bc5::compress_blocks(&rgba_surface).into()
			}
			Formats::RGB8 | Formats::RGBA8 => {
				data
			}
			Formats::BC7 => {
				let rgba_surface = intel_tex_2::RgbaSurface {
					data: &data,
					width: extent.width(),
					height: extent.height(),
					stride: extent.width() * 4,
				};

				let settings = intel_tex_2::bc7::opaque_ultra_fast_settings();

				intel_tex_2::bc7::compress_blocks(&settings, &rgba_surface).into()
			}
			Formats::RGB16 | Formats::RGBA16 => {
				data
			}
			_ => {
				panic!("Unsupported format")
			}
		};

		(Image {
			format,
			extent: extent.as_array(),
			gamma: *gamma,
		},
		data)
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

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
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
	use crate::{asset::{self, asset_handler::AssetHandler, asset_manager::AssetManager, ResourceId}, resource::storage_backend::tests::TestStorageBackend, tests::ASSETS_PATH};

	#[test]
	fn load_image() {
		let asset_handler = ImageAssetHandler::new();

		let asset_storage_backend = asset::FileStorageBackend::new(ASSETS_PATH.into());
		let resource_storage_backend = TestStorageBackend::new();
		let asset_manager = AssetManager::new_with_storage_backends(asset_storage_backend, resource_storage_backend.clone());
		let asset_storage_backend = asset::FileStorageBackend::new(ASSETS_PATH.into());

		let url = ResourceId::new("patterned_brick_floor_02_diff_2k.png");

		let asset = asset_handler.load(&asset_manager, &resource_storage_backend, &asset_storage_backend, url,).expect("Image asset handler did not handle asset");

		let _ = asset.load(&asset_manager, &resource_storage_backend, &asset_storage_backend, url,).expect("Image asset did not load");

		let generated_resources = resource_storage_backend.get_resources();

		assert_eq!(generated_resources.len(), 1);

		let resource = &generated_resources[0];

		assert_eq!(resource.id, "patterned_brick_floor_02_diff_2k.png");
		assert_eq!(resource.class, "Image");
	}

	// #[test]
	// #[ignore]
	// fn load_16_bit_normal_image() {
	// 	let asset_manager = AssetManager::new("../assets".into(),);
	// 	let asset_handler = ImageAssetHandler::new();

	// 	let url = "Revolver_Normal.png";

	// 	let storage_backend = asset_manager.get_test_storage_backend();

	// 	let _ = smol::block_on(asset_handler.load(&asset_manager, storage_backend, &url,)).expect("Image asset handler did not handle asset");

	// 	let generated_resources = storage_backend.get_resources();

	// 	assert_eq!(generated_resources.len(), 1);

	// 	let resource = &generated_resources[0];

	// 	assert_eq!(resource.id, url);
	// 	assert_eq!(resource.class, "Image");
	// }
}
