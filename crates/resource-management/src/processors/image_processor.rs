use utils::Extent;

use crate::{
	asset::{asset_handler::LoadErrors, resource_id::ResourceIdBase, ResourceId},
	resources::image::Image,
	types::{Formats, Gamma},
	Description, ProcessedAsset,
};

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
	fn get_resource_class() -> &'static str {
		"Image"
	}
}

pub fn process_image<'a>(
	id: ResourceId<'a>,
	description: ImageDescription,
	buffer: Box<[u8]>,
) -> Result<(ProcessedAsset, Box<[u8]>), LoadErrors> {
	let (resource, buffer) = produce_image(&description, buffer);
	Ok((ProcessedAsset::new(id, resource), buffer))
}

pub fn guess_semantic_from_name(name: ResourceIdBase) -> Semantic {
	let tokens = tokenize_asset_name(name.as_ref());
	if has_suffix_token_sequence(&tokens, &["base", "color"])
		|| has_suffix_token_sequence(&tokens, &["albedo"])
		|| has_suffix_token_sequence(&tokens, &["diffuse"])
	{
		Semantic::Albedo
	} else if has_suffix_token_sequence(&tokens, &["normal"]) {
		Semantic::Normal
	} else if has_suffix_token_sequence(&tokens, &["metallic"]) {
		Semantic::Metallic
	} else if has_suffix_token_sequence(&tokens, &["roughness"]) {
		Semantic::Roughness
	} else if has_suffix_token_sequence(&tokens, &["emissive"]) {
		Semantic::Emissive
	} else if has_suffix_token_sequence(&tokens, &["height"]) {
		Semantic::Height
	} else if has_suffix_token_sequence(&tokens, &["opacity"]) {
		Semantic::Opacity
	} else if has_suffix_token_sequence(&tokens, &["displacement"]) {
		Semantic::Displacement
	} else if has_suffix_token_sequence(&tokens, &["ao"]) {
		Semantic::AO
	} else {
		Semantic::Other
	}
}

fn tokenize_asset_name(name: &str) -> Vec<String> {
	let name = std::path::Path::new(name)
		.file_stem()
		.and_then(|stem| stem.to_str())
		.unwrap_or(name);

	name.split(|character: char| !character.is_alphanumeric())
		.filter(|token| !token.is_empty())
		.map(|token| token.to_ascii_lowercase())
		.collect()
}

fn has_suffix_token_sequence(tokens: &[String], sequence: &[&str]) -> bool {
	if sequence.is_empty() || sequence.len() > tokens.len() {
		return false;
	}

	tokens[tokens.len() - sequence.len()..]
		.iter()
		.map(String::as_str)
		.zip(sequence.iter().copied())
		.all(|(token, expected)| token == expected)
}

pub fn gamma_from_semantic(semantic: Semantic) -> Gamma {
	match semantic {
		Semantic::Albedo | Semantic::Other => Gamma::SRGB,
		Semantic::Normal
		| Semantic::Metallic
		| Semantic::Roughness
		| Semantic::Emissive
		| Semantic::Height
		| Semantic::Opacity
		| Semantic::Displacement
		| Semantic::AO => Gamma::Linear,
	}
}

pub fn should_compress_for_semantic(semantic: Semantic) -> bool {
	matches!(semantic, Semantic::Albedo | Semantic::Normal)
}

pub fn determine_image_format(source_format: Formats, compress: bool, semantic: Semantic) -> Formats {
	match source_format {
		Formats::RGB8 => {
			if compress {
				if semantic == Semantic::Normal {
					Formats::BC5
				} else {
					Formats::BC7
				}
			} else {
				Formats::RGBA8
			}
		}
		Formats::RGBA8 => {
			if compress {
				if semantic == Semantic::Normal {
					Formats::BC5
				} else {
					Formats::BC7
				}
			} else {
				Formats::RGBA8
			}
		}
		Formats::RGB16 => {
			if compress {
				if semantic == Semantic::Normal {
					Formats::BC5
				} else {
					Formats::BC7
				}
			} else {
				Formats::RGBA16
			}
		}
		Formats::RGBA16 => {
			panic!("Unsupported format: {:#?}", source_format);
		}
		_ => {
			panic!("Unsupported format: {:#?}", source_format);
		}
	}
}

fn produce_image(description: &ImageDescription, buffer: Box<[u8]>) -> (Image, Box<[u8]>) {
	let ImageDescription {
		format,
		extent,
		semantic,
		gamma,
	} = description;

	let compress = should_compress_for_semantic(*semantic);
	let output_format = determine_image_format(*format, compress, *semantic);

	let data = match (format, output_format) {
		(Formats::RGB8, Formats::RGBA8 | Formats::BC7 | Formats::BC5) => {
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

			buf
		}
		(Formats::RGBA8, Formats::BC5) => {
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

			buf
		}
		(Formats::RGBA8, Formats::RGBA8 | Formats::BC7) => buffer,
		(Formats::RGB16, Formats::BC5) => {
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
					dest_pixel[0] = x;
					dest_pixel[1] = y;
					dest_pixel[2] = 0x00;
					dest_pixel[3] = 0xFF;
				}
			}

			buf
		}
		(Formats::RGB16, Formats::RGBA16 | Formats::BC7) => {
			let mut buf: Box<[u8]> = vec![0_u8; extent.width() as usize * extent.height() as usize * 8].into();

			for y in 0..extent.height() {
				let source_row = &buffer[(y * extent.width() * 6) as usize..][..(extent.width() * 6) as usize];
				let dest_row = &mut buf[(y * extent.width() * 8) as usize..][..(extent.width() * 8) as usize];
				for x in 0..extent.width() {
					let source_pixel = &source_row[(x * 6) as usize..][..6];
					let dest_pixel = &mut dest_row[(x * 8) as usize..][..8];
					dest_pixel[..6].copy_from_slice(&source_pixel);
					dest_pixel[6] = 0xFF;
					dest_pixel[7] = 0xFF;
				}
			}

			buf
		}
		_ => {
			panic!("Unsupported format: {:#?}", format);
		}
	};

	let data = match output_format {
		Formats::BC5 => {
			let rgba_surface = intel_tex_2::RgSurface {
				data: &data,
				width: extent.width(),
				height: extent.height(),
				stride: extent.width() * 4,
			};

			intel_tex_2::bc5::compress_blocks(&rgba_surface).into()
		}
		Formats::RGB8 | Formats::RGBA8 => data,
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
		Formats::RGB16 | Formats::RGBA16 => data,
		_ => {
			panic!("Unsupported format")
		}
	};

	(
		Image {
			format: output_format,
			extent: extent.as_array(),
			gamma: *gamma,
		},
		data,
	)
}

#[cfg(test)]
mod tests {
	use utils::Extent;

	use crate::{
		asset::ResourceId,
		resources::image::Image,
		types::{Formats, Gamma},
	};

	use super::{
		determine_image_format, guess_semantic_from_name, process_image, should_compress_for_semantic, ImageDescription,
		Semantic,
	};

	#[test]
	fn extracts_semantic_from_asset_name() {
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_Base_color.png").get_base()),
			Semantic::Albedo
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_Diffuse.png").get_base()),
			Semantic::Albedo
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_Albedo.png").get_base()),
			Semantic::Albedo
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_Normal.png").get_base()),
			Semantic::Normal
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_Metallic.png").get_base()),
			Semantic::Metallic
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_Roughness.png").get_base()),
			Semantic::Roughness
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_Emissive.png").get_base()),
			Semantic::Emissive
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_Height.png").get_base()),
			Semantic::Height
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_Opacity.png").get_base()),
			Semantic::Opacity
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_Displacement.png").get_base()),
			Semantic::Displacement
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_AO.png").get_base()),
			Semantic::AO
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/brick_wall_Color.png").get_base()),
			Semantic::Other
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/diffuse_bomb_icon.png").get_base()),
			Semantic::Other
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/icon_diffuse.png").get_base()),
			Semantic::Albedo
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/DiffuseBombIcon.png").get_base()),
			Semantic::Other
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/NormalityChecker.png").get_base()),
			Semantic::Other
		);
		assert_eq!(
			guess_semantic_from_name(ResourceId::new("textures/AOGenerator.png").get_base()),
			Semantic::Other
		);
	}

	#[test]
	fn determines_output_format_from_compression_and_semantic() {
		assert_eq!(should_compress_for_semantic(Semantic::Albedo), true);
		assert_eq!(should_compress_for_semantic(Semantic::Other), false);
		assert_eq!(determine_image_format(Formats::RGB8, false, Semantic::Other), Formats::RGBA8);
		assert_eq!(determine_image_format(Formats::RGBA8, true, Semantic::Normal), Formats::BC5);
		assert_eq!(determine_image_format(Formats::RGB16, true, Semantic::Albedo), Formats::BC7);
	}

	#[test]
	fn process_image_expands_rgb8_into_rgba8_without_compression() {
		let description = ImageDescription {
			format: Formats::RGB8,
			extent: Extent::rectangle(2, 1),
			gamma: Gamma::SRGB,
			semantic: Semantic::Other,
		};

		let (asset, data) = process_image(
			ResourceId::new("textures/test.png"),
			description,
			vec![1, 2, 3, 4, 5, 6].into_boxed_slice(),
		)
		.expect("Image processing should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		assert_eq!(asset.id, "textures/test.png");
		assert_eq!(asset.class, "Image");
		assert_eq!(image.format, Formats::RGBA8);
		assert_eq!(image.gamma, Gamma::SRGB);
		assert_eq!(image.extent, [2, 1, 1]);
		assert_eq!(&*data, &[1, 2, 3, 0xFF, 4, 5, 6, 0xFF]);
	}

	#[test]
	fn process_image_compresses_normal_map_to_bc5() {
		let description = ImageDescription {
			format: Formats::RGBA8,
			extent: Extent::rectangle(4, 4),
			gamma: Gamma::Linear,
			semantic: Semantic::Normal,
		};

		let source = vec![128_u8; 4 * 4 * 4].into_boxed_slice();
		let (asset, data) = process_image(ResourceId::new("textures/normal.png"), description, source)
			.expect("Normal map processing should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		assert_eq!(image.format, Formats::BC5);
		assert_eq!(image.gamma, Gamma::Linear);
		assert_eq!(image.extent, [4, 4, 1]);
		assert!(!data.is_empty());
	}
}
