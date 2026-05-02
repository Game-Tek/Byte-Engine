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

pub fn determine_image_format(source_format: Formats, compress: bool, semantic: Semantic, gamma: Gamma) -> Formats {
	match source_format {
		Formats::RGB8 => {
			if compress {
				if semantic == Semantic::Normal {
					Formats::BC5
				} else if gamma == Gamma::SRGB {
					Formats::BC7SRGB
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
				} else if gamma == Gamma::SRGB {
					Formats::BC7SRGB
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
				} else if gamma == Gamma::SRGB {
					Formats::BC7SRGB
				} else {
					Formats::BC7
				}
			} else {
				Formats::RGBA16
			}
		}
		Formats::RGBA16 => {
			if compress {
				if semantic == Semantic::Normal {
					Formats::BC5
				} else if gamma == Gamma::SRGB {
					Formats::BC7SRGB
				} else {
					Formats::BC7
				}
			} else {
				Formats::RGBA16
			}
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
	let output_format = determine_image_format(*format, compress, *semantic, *gamma);

	let data = match (format, output_format) {
		(Formats::RGB8, Formats::RGBA8 | Formats::BC7 | Formats::BC7SRGB | Formats::BC5) => rgb8_to_rgba8(*extent, &buffer),
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
		(Formats::RGBA8, Formats::RGBA8 | Formats::BC7 | Formats::BC7SRGB) => buffer,
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
		(Formats::RGB16, Formats::RGBA16) => {
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
		(Formats::RGB16, Formats::BC7 | Formats::BC7SRGB) => rgb16_to_rgba8(*extent, &buffer),
		(Formats::RGBA16, Formats::RGBA16) => buffer,
		(Formats::RGBA16, Formats::BC7 | Formats::BC7SRGB) => rgba16_to_rgba8(*extent, &buffer),
		_ => {
			panic!("Unsupported format: {:#?}", format);
		}
	};

	let data = match output_format {
		Formats::BC5 => {
			let (data, width, height) = rgba8_bc_compression_surface(*extent, &data);
			let expected_surface_bytes = width as usize * height as usize * 4;
			assert_eq!(
				data.len(),
				expected_surface_bytes,
				"BC5 padded surface size mismatch. The most likely cause is that the BC compression padding copied an unexpected number of RGBA8 texels. extent={extent:?}, padded_width={width}, padded_height={height}, data_len={}, expected={expected_surface_bytes}",
				data.len()
			);
			let rgba_surface = intel_tex_2::RgSurface {
				data: &data,
				width,
				height,
				stride: width * 4,
			};

			let compressed = intel_tex_2::bc5::compress_blocks(&rgba_surface);
			let expected_payload_bytes = width as usize / 4 * (height as usize / 4) * 16;
			assert_eq!(
				compressed.len(),
				expected_payload_bytes,
				"BC5 payload size mismatch. The most likely cause is that the compressor block count no longer matches the padded image dimensions. extent={extent:?}, padded_width={width}, padded_height={height}, compressed_len={}, expected={expected_payload_bytes}",
				compressed.len()
			);
			compressed.into()
		}
		Formats::RGB8 | Formats::RGBA8 => data,
		Formats::BC7 | Formats::BC7SRGB => {
			let (data, width, height) = rgba8_bc_compression_surface(*extent, &data);
			let expected_surface_bytes = width as usize * height as usize * 4;
			assert_eq!(
				data.len(),
				expected_surface_bytes,
				"BC7 padded surface size mismatch. The most likely cause is that the BC compression padding copied an unexpected number of RGBA8 texels. format={output_format:?}, extent={extent:?}, padded_width={width}, padded_height={height}, data_len={}, expected={expected_surface_bytes}",
				data.len()
			);
			let rgba_surface = intel_tex_2::RgbaSurface {
				data: &data,
				width,
				height,
				stride: width * 4,
			};

			let settings = bc7_compression_settings(&data);

			let compressed = intel_tex_2::bc7::compress_blocks(&settings, &rgba_surface);
			let expected_payload_bytes = width as usize / 4 * (height as usize / 4) * 16;
			assert_eq!(
				compressed.len(),
				expected_payload_bytes,
				"BC7 payload size mismatch. The most likely cause is that the compressor block count no longer matches the padded image dimensions. format={output_format:?}, extent={extent:?}, padded_width={width}, padded_height={height}, compressed_len={}, expected={expected_payload_bytes}",
				compressed.len()
			);
			compressed.into()
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

	use super::{
		bc7_compression_settings, determine_image_format, guess_semantic_from_name, process_image,
		should_compress_for_semantic, ImageDescription, Semantic,
	};
	use crate::{
		asset::ResourceId,
		resources::image::Image,
		types::{Formats, Gamma},
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
		assert_eq!(should_compress_for_semantic(Semantic::Normal), true);
		assert_eq!(should_compress_for_semantic(Semantic::Other), false);
		assert_eq!(
			determine_image_format(Formats::RGB8, false, Semantic::Other, Gamma::SRGB),
			Formats::RGBA8
		);
		assert_eq!(
			determine_image_format(Formats::RGBA8, true, Semantic::Normal, Gamma::Linear),
			Formats::BC5
		);
		assert_eq!(
			determine_image_format(Formats::RGB16, true, Semantic::Albedo, Gamma::Linear),
			Formats::BC7
		);
		assert_eq!(
			determine_image_format(Formats::RGB16, true, Semantic::Albedo, Gamma::SRGB),
			Formats::BC7SRGB
		);
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
		assert_eq!(data.len(), 16);
	}

	#[test]
	fn process_image_compresses_srgb_albedo_to_bc7_srgb() {
		let description = ImageDescription {
			format: Formats::RGBA8,
			extent: Extent::rectangle(5, 7),
			gamma: Gamma::SRGB,
			semantic: Semantic::Albedo,
		};

		let source = vec![128_u8; 5 * 7 * 4].into_boxed_slice();
		let (asset, data) = process_image(ResourceId::new("textures/albedo.png"), description, source)
			.expect("Albedo image processing should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		assert_eq!(image.format, Formats::BC7SRGB);
		assert_eq!(image.gamma, Gamma::SRGB);
		assert_eq!(image.extent, [5, 7, 1]);
		assert_eq!(data.len(), 2 * 2 * 16);
	}

	#[test]
	fn bc7_compression_settings_preserve_alpha_when_needed() {
		let opaque = [1, 2, 3, 0xFF, 4, 5, 6, 0xFF];
		let transparent = [1, 2, 3, 0xFE, 4, 5, 6, 0xFF];

		assert_eq!(bc7_compression_settings(&opaque).channels, 3);
		assert_eq!(bc7_compression_settings(&transparent).channels, 4);
	}
}

/// Expands an RGB8 image into RGBA8 so all runtime texture uploads use GPU-supported color layouts.
fn rgb8_to_rgba8(extent: Extent, buffer: &[u8]) -> Box<[u8]> {
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

/// Converts RGB16 data to RGBA8 before BC compression because the compressor accepts 8-bit surfaces.
fn rgb16_to_rgba8(extent: Extent, buffer: &[u8]) -> Box<[u8]> {
	let mut buf: Box<[u8]> = vec![0_u8; extent.width() as usize * extent.height() as usize * 4].into();

	for y in 0..extent.height() {
		let source_row = &buffer[(y * extent.width() * 6) as usize..][..(extent.width() * 6) as usize];
		let dest_row = &mut buf[(y * extent.width() * 4) as usize..][..(extent.width() * 4) as usize];
		for x in 0..extent.width() {
			let source_pixel = &source_row[(x * 6) as usize..][..6];
			let dest_pixel = &mut dest_row[(x * 4) as usize..][..4];
			dest_pixel[0] = source_pixel[1];
			dest_pixel[1] = source_pixel[3];
			dest_pixel[2] = source_pixel[5];
			dest_pixel[3] = 0xFF;
		}
	}

	buf
}

/// Converts RGBA16 data to RGBA8 before BC compression because the compressor accepts 8-bit surfaces.
fn rgba16_to_rgba8(extent: Extent, buffer: &[u8]) -> Box<[u8]> {
	let mut buf: Box<[u8]> = vec![0_u8; extent.width() as usize * extent.height() as usize * 4].into();

	for y in 0..extent.height() {
		let source_row = &buffer[(y * extent.width() * 8) as usize..][..(extent.width() * 8) as usize];
		let dest_row = &mut buf[(y * extent.width() * 4) as usize..][..(extent.width() * 4) as usize];
		for x in 0..extent.width() {
			let source_pixel = &source_row[(x * 8) as usize..][..8];
			let dest_pixel = &mut dest_row[(x * 4) as usize..][..4];
			dest_pixel[0] = source_pixel[1];
			dest_pixel[1] = source_pixel[3];
			dest_pixel[2] = source_pixel[5];
			dest_pixel[3] = source_pixel[7];
		}
	}

	buf
}

/// Selects BC7 compressor settings that favor quality enough to avoid visible block-row artifacts.
fn bc7_compression_settings(data: &[u8]) -> intel_tex_2::bc7::EncodeSettings {
	let has_alpha = data.chunks_exact(4).any(|pixel| pixel[3] != 0xFF);

	if has_alpha {
		intel_tex_2::bc7::alpha_basic_settings()
	} else {
		intel_tex_2::bc7::opaque_basic_settings()
	}
}

/// Pads RGBA8 data to full BC blocks so compressed payload size matches GPU upload layout.
fn rgba8_bc_compression_surface(extent: Extent, data: &[u8]) -> (Box<[u8]>, u32, u32) {
	let width = extent.width().max(1);
	let height = extent.height().max(1);
	let expected_source_bytes = width as usize * height as usize * 4;
	assert_eq!(
		data.len(),
		expected_source_bytes,
		"BC compression source size mismatch. The most likely cause is that image format conversion did not produce one RGBA8 texel per source pixel. extent={extent:?}, width={width}, height={height}, data_len={}, expected={expected_source_bytes}",
		data.len()
	);
	let padded_width = width.next_multiple_of(4);
	let padded_height = height.next_multiple_of(4);
	let mut padded = vec![0u8; padded_width as usize * padded_height as usize * 4].into_boxed_slice();

	for y in 0..padded_height {
		let source_y = y.min(height - 1);
		for x in 0..padded_width {
			let source_x = x.min(width - 1);
			let source_offset = ((source_y * width + source_x) * 4) as usize;
			let destination_offset = ((y * padded_width + x) * 4) as usize;
			padded[destination_offset..destination_offset + 4].copy_from_slice(&data[source_offset..source_offset + 4]);
		}
	}

	(padded, padded_width, padded_height)
}
