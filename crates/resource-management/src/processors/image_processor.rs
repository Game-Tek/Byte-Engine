use std::alloc::{Allocator, Global};

use utils::Extent;

use crate::{
	asset::{asset_handler::LoadErrors, resource_id::ResourceIdBase, ResourceId},
	resources::{image::Image, mips::generate_mip_chain_in},
	types::{Formats, Gamma},
	Description, ProcessedAsset, StreamDescription,
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
	/// When `true`, a full power-of-two mip chain is generated and stored after the base level.
	pub generate_mipmaps: bool,
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
	process_image_in(id, description, buffer, Global)
}

/// Processes image pixels using the provided allocator for transient and output buffers.
pub fn process_image_in<'a, A: Allocator + Clone, B: Allocator>(
	id: ResourceId<'a>,
	description: ImageDescription,
	buffer: Box<[u8], B>,
	allocator: A,
) -> Result<(ProcessedAsset, Box<[u8], A>), LoadErrors> {
	let (resource, buffer, streams) = produce_image_in(&description, buffer, allocator)?;
	let asset = ProcessedAsset::new(id, resource);
	let asset = if let Some(streams) = streams {
		asset.with_streams(streams)
	} else {
		asset
	};
	Ok((asset, buffer))
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
		Formats::RGBA16F => Formats::RGBA16F,
		_ => {
			panic!("Unsupported format: {:#?}", source_format);
		}
	}
}

fn produce_image_in<A: Allocator + Clone, B: Allocator>(
	description: &ImageDescription,
	buffer: Box<[u8], B>,
	allocator: A,
) -> Result<(Image, Box<[u8], A>, Option<Vec<StreamDescription>>), LoadErrors> {
	let ImageDescription {
		format,
		extent,
		semantic,
		gamma,
		generate_mipmaps,
	} = description;

	let compress = should_compress_for_semantic(*semantic);
	let output_format = determine_image_format(*format, compress, *semantic, *gamma);

	// Convert the source data into the uncompressed intermediate that mip generation and BC
	// compression both expect as input (always RGBA8 for BC targets, otherwise the natural format).
	let intermediate: Box<[u8], A> = match (format, output_format) {
		(Formats::RGB8, Formats::RGBA8 | Formats::BC7 | Formats::BC7SRGB | Formats::BC5 | Formats::BC5SNORM) => {
			rgb8_to_rgba8_in(*extent, &buffer, allocator.clone())
		}
		(Formats::RGBA8, Formats::BC5 | Formats::BC5SNORM) => {
			let mut buf = zeroed_boxed_slice_in(extent.width() as usize * extent.height() as usize * 4, allocator.clone());

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
		(Formats::RGBA8, Formats::RGBA8 | Formats::BC7 | Formats::BC7SRGB) => copy_slice_in(&buffer, allocator.clone()),
		(Formats::RGB16, Formats::BC5 | Formats::BC5SNORM) => {
			let mut buf = zeroed_boxed_slice_in(extent.width() as usize * extent.height() as usize * 4, allocator.clone());

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
			let mut buf = zeroed_boxed_slice_in(extent.width() as usize * extent.height() as usize * 8, allocator.clone());

			for y in 0..extent.height() {
				let source_row = &buffer[(y * extent.width() * 6) as usize..][..(extent.width() * 6) as usize];
				let dest_row = &mut buf[(y * extent.width() * 8) as usize..][..(extent.width() * 8) as usize];
				for x in 0..extent.width() {
					let source_pixel = &source_row[(x * 6) as usize..][..6];
					let dest_pixel = &mut dest_row[(x * 8) as usize..][..8];
					dest_pixel[..6].copy_from_slice(source_pixel);
					dest_pixel[6] = 0xFF;
					dest_pixel[7] = 0xFF;
				}
			}

			buf
		}
		(Formats::RGB16, Formats::BC7 | Formats::BC7SRGB) => rgb16_to_rgba8_in(*extent, &buffer, allocator.clone()),
		(Formats::RGBA16, Formats::RGBA16) => copy_slice_in(&buffer, allocator.clone()),
		(Formats::RGBA16, Formats::BC5 | Formats::BC5SNORM) => rgba16_to_rgba8_in(*extent, &buffer, allocator.clone()),
		(Formats::RGBA16, Formats::BC7 | Formats::BC7SRGB) => rgba16_to_rgba8_in(*extent, &buffer, allocator.clone()),
		(Formats::RGBA16F, Formats::RGBA16F) => copy_slice_in(&buffer, allocator.clone()),
		_ => {
			panic!("Unsupported format: {:#?}", format);
		}
	};

	// The format of the `intermediate` buffer — used for mip generation.
	let intermediate_format = match output_format {
		Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB => Formats::RGBA8,
		_ => output_format,
	};

	let (mip_count, data, streams) = if *generate_mipmaps {
		let chain = generate_mip_chain_in(
			intermediate_format,
			extent.width(),
			extent.height(),
			&intermediate,
			allocator.clone(),
		)
		.map_err(|_| LoadErrors::FailedToProcess)?;
		let mip_count = chain.len() as u32;

		let mut all_data = Vec::new_in(allocator.clone());
		let mut streams = Vec::new();
		let mut offset: usize = 0;

		for (index, level) in chain.levels().enumerate() {
			let level_extent = Extent::rectangle(level.width, level.height);
			let level_data = compress_bc_level_in(output_format, level_extent, level.data, allocator.clone());
			let size = level_data.len();
			streams.push(StreamDescription::new(&format!("mip[{index}]"), size, offset));
			all_data.extend_from_slice(&level_data);
			offset += size;
		}
		(mip_count, all_data.into_boxed_slice(), Some(streams))
	} else {
		let data = compress_bc_level_in(output_format, *extent, &intermediate, allocator.clone());
		let streams = Some(vec![StreamDescription::new("mip[0]", data.len(), 0)]);
		(1_u32, data, streams)
	};

	Ok((
		Image {
			format: output_format,
			extent: image_resource_extent(output_format, *extent),
			gamma: *gamma,
			mip_count,
			ibl: None,
		},
		data,
		streams,
	))
}

fn image_resource_extent(format: Formats, extent: Extent) -> [u32; 3] {
	match format {
		Formats::BC5 | Formats::BC5SNORM => extent.as_array(),
		_ => [extent.width(), extent.height(), extent.depth().max(1)],
	}
}

/// Compresses a single mip level to the target `output_format`, or returns the data unchanged for
/// uncompressed formats. Accepts an RGBA8 surface for BC targets, or the natural format otherwise.
#[cfg(test)]
fn compress_bc_level(output_format: Formats, extent: Extent, data: &[u8]) -> Box<[u8]> {
	compress_bc_level_in(output_format, extent, data, Global)
}

/// Compresses or copies one mip level using the provided allocator for padding and output buffers.
fn compress_bc_level_in<A: Allocator + Clone>(
	output_format: Formats,
	extent: Extent,
	data: &[u8],
	allocator: A,
) -> Box<[u8], A> {
	match output_format {
		Formats::BC5 | Formats::BC5SNORM => {
			// RgSurface<2> expects tightly packed RG pairs (2 bytes per pixel),
			// not interleaved RGBA. Convert the RGBA8 intermediate to RG8
			// before compression to avoid reading B/A as the second pixel's R/G.
			let (rg_data, width, height) = rga_to_rg_surface_in(data, extent, allocator.clone());
			let rg_surface = intel_tex_2::RgSurface {
				data: &rg_data,
				width,
				height,
				stride: width * 2,
			};

			let compressed = intel_tex_2::bc5::compress_blocks(&rg_surface);
			let expected_payload_bytes = width as usize / 4 * (height as usize / 4) * 16;
			assert_eq!(
					compressed.len(),
					expected_payload_bytes,
					"BC5 payload size mismatch. The most likely cause is that the compressor block count no longer matches the padded image dimensions. extent={extent:?}, padded_width={width}, padded_height={height}, compressed_len={}, expected={expected_payload_bytes}",
					compressed.len()
				);
			move_boxed_slice_in(compressed.into_boxed_slice(), allocator)
		}
		Formats::BC7 | Formats::BC7SRGB => {
			let (data, width, height) = rgba8_bc_compression_surface_in(extent, data, allocator.clone());
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
			move_boxed_slice_in(compressed.into_boxed_slice(), allocator)
		}
		Formats::RGB8 | Formats::RGBA8 | Formats::RGB16 | Formats::RGBA16 | Formats::RGBA16F => {
			let mut output = Vec::with_capacity_in(data.len(), allocator);
			output.extend_from_slice(data);
			output.into_boxed_slice()
		}
		_ => {
			panic!("Unsupported format")
		}
	}
}

#[cfg(test)]
mod tests {
	use utils::Extent;

	use super::{
		bc7_compression_settings, compress_bc_level, determine_image_format, guess_semantic_from_name, process_image,
		rga_to_rg_surface, should_compress_for_semantic, ImageDescription, Semantic,
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
			generate_mipmaps: false,
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
			generate_mipmaps: false,
		};

		let source = vec![128_u8; 4 * 4 * 4].into_boxed_slice();
		let (asset, data) = process_image(ResourceId::new("textures/normal.png"), description, source)
			.expect("Normal map processing should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		assert_eq!(image.format, Formats::BC5);
		assert_eq!(image.gamma, Gamma::Linear);
		assert_eq!(image.extent, [4, 4, 0]);
		assert_eq!(data.len(), 16);
	}

	#[test]
	fn rga_to_rg_surface_extracts_only_r_and_g_channels() {
		// RGBA8 input with distinct channel values so the test fails if
		// the wrong channels leak into the RG output.
		let rgba: Vec<u8> = (0u8..64).collect(); // 4×4 RGBA = 64 bytes: R,G,B,A,R,G,B,A,...
		let extent = Extent::rectangle(4, 4);

		let (rg, width, height) = rga_to_rg_surface(&rgba, extent);

		assert_eq!(width, 4);
		assert_eq!(height, 4);
		// Output should be RG pairs: [0,1], [4,5], [8,9], ... — only R and G from each pixel
		assert_eq!(rg.len(), 4 * 4 * 2);
		assert_eq!(&rg[0..2], &[0, 1]); // R₀, G₀
		assert_eq!(&rg[2..4], &[4, 5]); // R₁, G₁ (skipping B₀=2, A₀=3)
		assert_eq!(&rg[4..6], &[8, 9]); // R₂, G₂ (skipping B₁=6, A₁=7)
		assert_eq!(&rg[6..8], &[12, 13]); // R₃, G₃
	}

	#[test]
	fn rga_to_rg_surface_pads_to_block_aligned_dimensions() {
		// 5×7 input should be padded to 8×8 (next multiples of 4).
		let rgba = vec![0u8; 5 * 7 * 4];
		let extent = Extent::rectangle(5, 7);

		let (rg, width, height) = rga_to_rg_surface(&rgba, extent);

		assert_eq!(width, 8);
		assert_eq!(height, 8);
		assert_eq!(rg.len(), 8 * 8 * 2);

		// The last pixel of the first row (source x=4) should be replicated
		// into the padding area (x=5,6,7). Verify padding byte pattern.
		let last_rg_pixel = &rg[4 * 2..5 * 2];
		let padded_pixel = &rg[5 * 2..6 * 2];
		assert_eq!(last_rg_pixel, padded_pixel, "Edge pixel should be clamped into padding");
	}

	#[test]
	fn bc5_compressor_uses_rg_surface_not_rgba_interleaved() {
		// Regression: RgSurface<2> reads 2 bytes per pixel. If we accidentally
		// feed it an RGBA surface with stride=width*4, the compressor mixes B/A
		// channels into the output. This test verifies the compressor receives
		// pure RG pairs by checking that a known RG input produces consistent
		// compressed output regardless of B/A channel values.
		//
		// Create an RGBA8 surface where the B channel differs from the A channel
		// across the image. If the compressor were reading RGBA as 2-byte pixels,
		// the output would differ because the "second pixel" would be B/A instead
		// of the real R/G from the next pixel.
		let extent = Extent::rectangle(4, 4);

		// Variant A: R=0, G=1, B and A are 0xFF (all pixels identical)
		let mut a = vec![0u8; 4 * 4 * 4];
		for i in 0..16 {
			a[i * 4] = 0;
			a[i * 4 + 1] = 1;
			a[i * 4 + 2] = 0xFF;
			a[i * 4 + 3] = 0xFF;
		}

		// Variant B: R=0, G=1, B and A are 0x00 (all pixels identical)
		let mut b = vec![0u8; 4 * 4 * 4];
		for i in 0..16 {
			b[i * 4] = 0;
			b[i * 4 + 1] = 1;
			b[i * 4 + 2] = 0x00;
			b[i * 4 + 3] = 0x00;
		}

		let compressed_a = compress_bc_level(Formats::BC5, extent, &a);
		let compressed_b = compress_bc_level(Formats::BC5, extent, &b);

		// Both have identical R and G channels; the compressed output must also
		// be identical because B and A should not influence BC5 compression.
		assert_eq!(compressed_a, compressed_b, "BC5 should ignore B and A channels");
	}

	#[test]
	fn process_image_compresses_srgb_albedo_to_bc7_srgb() {
		let description = ImageDescription {
			format: Formats::RGBA8,
			extent: Extent::rectangle(5, 7),
			gamma: Gamma::SRGB,
			semantic: Semantic::Albedo,
			generate_mipmaps: false,
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
	fn process_image_compresses_rgb16_albedo_to_bc7() {
		// Regression: the old code built an RGBA16 intermediate (8 bytes/pixel) but passed it to
		// the BC7 compressor with stride = width * 4 (an RGBA8 stride), halving the effective row
		// width and producing horizontal stripes. The correct path converts RGB16 → RGBA8 first.
		let description = ImageDescription {
			format: Formats::RGB16,
			extent: Extent::rectangle(4, 4),
			gamma: Gamma::Linear,
			semantic: Semantic::Albedo,
			generate_mipmaps: false,
		};

		// RGB16: 3 channels × 2 bytes = 6 bytes per pixel
		let source = vec![128_u8; 4 * 4 * 6].into_boxed_slice();
		let (asset, data) = process_image(ResourceId::new("textures/albedo16.png"), description, source)
			.expect("RGB16 albedo processing should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		assert_eq!(image.format, Formats::BC7);
		assert_eq!(image.extent, [4, 4, 1]);
		// 4×4 image → 1×1 block grid → 1 block × 16 bytes
		assert_eq!(data.len(), 16);
	}

	#[test]
	fn process_image_compresses_rgba16_normal_to_bc5() {
		// BC5 compresses RGBA16 normal maps by first converting to RGBA8
		// and then compressing R and G channels with BC5.
		let description = ImageDescription {
			format: Formats::RGBA16,
			extent: Extent::rectangle(4, 4),
			gamma: Gamma::Linear,
			semantic: Semantic::Normal,
			generate_mipmaps: false,
		};

		// RGBA16: 4 channels × 2 bytes = 8 bytes per pixel
		let source = vec![128_u8; 4 * 4 * 8].into_boxed_slice();
		let (asset, data) = process_image(ResourceId::new("textures/normal16.png"), description, source)
			.expect("RGBA16 normal map processing should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		assert_eq!(image.format, Formats::BC5);
		assert_eq!(image.extent, [4, 4, 0]);
		// 4×4 image → 1×1 block grid → 1 block × 16 bytes
		assert_eq!(data.len(), 16);
	}

	#[test]
	fn bc7_compression_settings_preserve_alpha_when_needed() {
		let opaque = [1, 2, 3, 0xFF, 4, 5, 6, 0xFF];
		let transparent = [1, 2, 3, 0xFE, 4, 5, 6, 0xFF];

		assert_eq!(bc7_compression_settings(&opaque).channels, 3);
		assert_eq!(bc7_compression_settings(&transparent).channels, 4);
	}

	#[test]
	fn process_image_without_mipmaps_stores_mip_count_one() {
		let description = ImageDescription {
			format: Formats::RGBA8,
			extent: Extent::rectangle(4, 4),
			gamma: Gamma::SRGB,
			semantic: Semantic::Other,
			generate_mipmaps: false,
		};

		let source = vec![128_u8; 4 * 4 * 4].into_boxed_slice();
		let (asset, _data) =
			process_image(ResourceId::new("textures/test.png"), description, source).expect("Image processing should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		assert_eq!(image.mip_count, 1);
	}

	#[test]
	fn process_image_with_mipmaps_produces_full_chain_for_rgba8() {
		// 4×4 → 4 levels: 4×4, 2×2, 1×1 … wait, 4→2→1 = 3 levels.
		let width = 4_u32;
		let height = 4_u32;
		let description = ImageDescription {
			format: Formats::RGBA8,
			extent: Extent::rectangle(width, height),
			gamma: Gamma::SRGB,
			semantic: Semantic::Other,
			generate_mipmaps: true,
		};

		let source = vec![200_u8; (width * height * 4) as usize].into_boxed_slice();
		let (asset, data) = process_image(ResourceId::new("textures/mip_rgba8.png"), description, source)
			.expect("Mip generation should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		// 4×4 → 2×2 → 1×1  =  3 levels
		let expected_levels = crate::resources::mips::mip_level_count(width, height).unwrap();
		assert_eq!(image.mip_count, expected_levels);
		assert_eq!(image.format, Formats::RGBA8);

		// Each level is RGBA8: 4×4×4 + 2×2×4 + 1×1×4 = 64 + 16 + 4 = 84 bytes
		let expected_bytes = (4 * 4 * 4) + (2 * 2 * 4) + (1 * 1 * 4);
		assert_eq!(data.len(), expected_bytes);
	}

	#[test]
	fn process_image_with_mipmaps_produces_correct_mip_count_for_bc5_normal_map() {
		// BC5 compresses RGBA8 intermediate in 4×4 blocks.
		let width = 8_u32;
		let height = 8_u32;
		let description = ImageDescription {
			format: Formats::RGBA8,
			extent: Extent::rectangle(width, height),
			gamma: Gamma::Linear,
			semantic: Semantic::Normal,
			generate_mipmaps: true,
		};

		// RGBA8: 4 bytes/pixel
		let source = vec![128_u8; (width * height * 4) as usize].into_boxed_slice();
		let (asset, data) = process_image(ResourceId::new("textures/mip_normal_bc5.png"), description, source)
			.expect("BC5 mip generation should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		// 8×8 → 4×4 → 2×2 → 1×1  =  4 levels
		let expected_levels = crate::resources::mips::mip_level_count(width, height).unwrap();
		assert_eq!(image.mip_count, expected_levels);
		assert_eq!(image.format, Formats::BC5);

		// Level 0: 8×8  → padded 8×8  → 2×2 blocks → 2*2*16 =  64 bytes
		// Level 1: 4×4  → padded 4×4  → 1×1 block  → 1*1*16 =  16 bytes
		// Level 2: 2×2  → padded 4×4  → 1×1 block  →          16 bytes
		// Level 3: 1×1  → padded 4×4  → 1×1 block  →          16 bytes
		let expected_bytes = (2 * 2 * 16) + (1 * 1 * 16) + (1 * 1 * 16) + (1 * 1 * 16);
		assert_eq!(data.len(), expected_bytes);
	}

	#[test]
	fn process_image_with_mipmaps_produces_correct_mip_count_for_bc7_albedo() {
		let width = 8_u32;
		let height = 8_u32;
		let description = ImageDescription {
			format: Formats::RGBA8,
			extent: Extent::rectangle(width, height),
			gamma: Gamma::SRGB,
			semantic: Semantic::Albedo,
			generate_mipmaps: true,
		};

		let source = vec![128_u8; (width * height * 4) as usize].into_boxed_slice();
		let (asset, data) = process_image(ResourceId::new("textures/mip_albedo_bc7.png"), description, source)
			.expect("BC7 mip generation should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		let expected_levels = crate::resources::mips::mip_level_count(width, height).unwrap();
		assert_eq!(image.mip_count, expected_levels);
		assert_eq!(image.format, Formats::BC7SRGB);

		// Same block sizing as BC5 (16 bytes per 4×4 block)
		let expected_bytes = (2 * 2 * 16) + (1 * 1 * 16) + (1 * 1 * 16) + (1 * 1 * 16);
		assert_eq!(data.len(), expected_bytes);
	}

	#[test]
	fn process_image_with_mipmaps_non_power_of_two_rgba8() {
		// Non-power-of-two dimensions: verify that mip count and data length are consistent.
		let width = 5_u32;
		let height = 3_u32;
		let description = ImageDescription {
			format: Formats::RGBA8,
			extent: Extent::rectangle(width, height),
			gamma: Gamma::SRGB,
			semantic: Semantic::Other,
			generate_mipmaps: true,
		};

		let source = vec![100_u8; (width * height * 4) as usize].into_boxed_slice();
		let (asset, data) = process_image(ResourceId::new("textures/mip_npot.png"), description, source)
			.expect("Non-power-of-two mip generation should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		let expected_levels = crate::resources::mips::mip_level_count(width, height).unwrap();
		assert_eq!(image.mip_count, expected_levels);

		// Manually compute expected byte count: 5×3, 2×1, 1×1
		let expected_bytes = (5 * 3 * 4) + (2 * 1 * 4) + (1 * 1 * 4);
		assert_eq!(data.len(), expected_bytes);
	}
}

/// Expands an RGB8 image into RGBA8 using caller-provided storage.
fn rgb8_to_rgba8_in<A: Allocator + Clone>(extent: Extent, buffer: &[u8], allocator: A) -> Box<[u8], A> {
	let mut buf = zeroed_boxed_slice_in(extent.width() as usize * extent.height() as usize * 4, allocator);

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

/// Converts RGB16 data to RGBA8 using caller-provided storage.
fn rgb16_to_rgba8_in<A: Allocator + Clone>(extent: Extent, buffer: &[u8], allocator: A) -> Box<[u8], A> {
	let mut buf = zeroed_boxed_slice_in(extent.width() as usize * extent.height() as usize * 4, allocator);

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

/// Converts RGBA16 data to RGBA8 using caller-provided storage.
fn rgba16_to_rgba8_in<A: Allocator + Clone>(extent: Extent, buffer: &[u8], allocator: A) -> Box<[u8], A> {
	let mut buf = zeroed_boxed_slice_in(extent.width() as usize * extent.height() as usize * 4, allocator);

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

/// Pads RGBA8 data to BC block dimensions using caller-provided storage.
fn rgba8_bc_compression_surface_in<A: Allocator + Clone>(
	extent: Extent,
	data: &[u8],
	allocator: A,
) -> (Box<[u8], A>, u32, u32) {
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
	let mut padded = zeroed_boxed_slice_in(padded_width as usize * padded_height as usize * 4, allocator);

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

/// Produces a tightly packed RG surface (2 bytes per pixel) from RGBA8 data,
/// padded to 4×4 block boundaries. RgSurface<2> expects the pixel stride to
/// be exactly 2 bytes, not interleaved RGBA.
#[cfg(test)]
fn rga_to_rg_surface(data: &[u8], extent: Extent) -> (Box<[u8]>, u32, u32) {
	rga_to_rg_surface_in(data, extent, Global)
}

/// Produces a BC5 RG surface using caller-provided storage.
fn rga_to_rg_surface_in<A: Allocator + Clone>(data: &[u8], extent: Extent, allocator: A) -> (Box<[u8], A>, u32, u32) {
	let width = extent.width().max(1);
	let height = extent.height().max(1);
	let padded_width = width.next_multiple_of(4);
	let padded_height = height.next_multiple_of(4);
	let mut padded = zeroed_boxed_slice_in(padded_width as usize * padded_height as usize * 2, allocator);

	for y in 0..padded_height {
		let source_y = y.min(height - 1);
		for x in 0..padded_width {
			let source_x = x.min(width - 1);
			let source_offset = ((source_y * width + source_x) * 4) as usize;
			let destination_offset = ((y * padded_width + x) * 2) as usize;
			// Copy only R and G channels from the RGBA source
			padded[destination_offset..destination_offset + 2].copy_from_slice(&data[source_offset..source_offset + 2]);
		}
	}

	(padded, padded_width, padded_height)
}

fn zeroed_boxed_slice_in<A: Allocator + Clone>(len: usize, allocator: A) -> Box<[u8], A> {
	let mut buffer = Vec::with_capacity_in(len, allocator);
	buffer.resize(len, 0_u8);
	buffer.into_boxed_slice()
}

fn copy_slice_in<A: Allocator + Clone>(buffer: &[u8], allocator: A) -> Box<[u8], A> {
	let mut output = Vec::with_capacity_in(buffer.len(), allocator);
	output.extend_from_slice(buffer);
	output.into_boxed_slice()
}

fn move_boxed_slice_in<A: Allocator + Clone>(buffer: Box<[u8]>, allocator: A) -> Box<[u8], A> {
	copy_slice_in(&buffer, allocator)
}
