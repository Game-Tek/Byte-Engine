mod decode;
mod source;

pub(crate) use decode::{decode_rgba16f_in, png_declared_gamma};
pub(crate) use source::{CanonicalImageData, canonicalize_rgba16f_in};
pub use source::{ImageSource, SourceChannels, SourceEncoding};
use source::{append_canonical_image_in, canonicalize_image_in};

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

/// The `ImageDescription` struct selects semantic processing, gamma, and mip generation for one decoded image.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct ImageDescription {
	pub gamma: Gamma,
	pub semantic: Semantic,
	/// When `true`, a full power-of-two mip chain is generated and stored after the base level.
	pub generate_mipmaps: bool,
}

pub fn process_image<'a>(
	id: ResourceId<'a>,
	description: ImageDescription,
	source: ImageSource<'_>,
) -> Result<(ProcessedAsset, Box<[u8]>), LoadErrors> {
	process_image_in(id, description, source, Global)
}

/// Processes image pixels using the provided allocator for transient and output buffers.
pub fn process_image_in<'a, A: Allocator + Clone>(
	id: ResourceId<'a>,
	description: ImageDescription,
	source: ImageSource<'_>,
	allocator: A,
) -> Result<(ProcessedAsset, Box<[u8], A>), LoadErrors> {
	process_image_with_mip_backend_in(id, description, source, allocator, None)
}

/// Processes image pixels and delegates requested lower mip levels to an optional offline backend.
///
/// Material importers pass their GPU backend here. Standalone image handlers should call [`process_image_in`] so their
/// authored texture payload remains unchanged.
pub fn process_image_with_mip_backend_in<'a, A: Allocator + Clone>(
	id: ResourceId<'a>,
	description: ImageDescription,
	source: ImageSource<'_>,
	allocator: A,
	mip_backend: Option<&dyn MipGenerationBackend>,
) -> Result<(ProcessedAsset, Box<[u8], A>), LoadErrors> {
	let (resource, buffer, streams) = produce_image_in(&description, source, allocator, mip_backend)?;

	let asset = ProcessedAsset::new(id, resource);

	let asset = if let Some(streams) = streams {
		asset.with_streams(streams)
	} else {
		asset
	};

	Ok((asset, buffer))
}

pub fn guess_semantic_from_name(name: ResourceIdBase) -> Semantic {
	let name = name.as_ref();
	if has_suffix_token_sequence(name, &["base", "color"])
		|| has_suffix_token_sequence(name, &["albedo"])
		|| has_suffix_token_sequence(name, &["diffuse"])
	{
		Semantic::Albedo
	} else if has_suffix_token_sequence(name, &["normal"]) {
		Semantic::Normal
	} else if has_suffix_token_sequence(name, &["metallic"]) {
		Semantic::Metallic
	} else if has_suffix_token_sequence(name, &["roughness"]) {
		Semantic::Roughness
	} else if has_suffix_token_sequence(name, &["emissive"]) {
		Semantic::Emissive
	} else if has_suffix_token_sequence(name, &["height"]) {
		Semantic::Height
	} else if has_suffix_token_sequence(name, &["opacity"]) {
		Semantic::Opacity
	} else if has_suffix_token_sequence(name, &["displacement"]) {
		Semantic::Displacement
	} else if has_suffix_token_sequence(name, &["ao"]) {
		Semantic::AO
	} else {
		Semantic::Other
	}
}

fn has_suffix_token_sequence(name: &str, sequence: &[&str]) -> bool {
	let name = std::path::Path::new(name)
		.file_stem()
		.and_then(|stem| stem.to_str())
		.unwrap_or(name);
	let mut tokens = name
		.split(|character: char| !character.is_alphanumeric())
		.filter(|token| !token.is_empty())
		.rev();
	!sequence.is_empty()
		&& sequence
			.iter()
			.rev()
			.all(|expected| tokens.next().is_some_and(|token| token.eq_ignore_ascii_case(expected)))
}

pub fn gamma_from_semantic(semantic: Semantic) -> Gamma {
	match semantic {
		Semantic::Albedo | Semantic::Emissive | Semantic::Other => Gamma::SRGB,
		Semantic::Normal
		| Semantic::Metallic
		| Semantic::Roughness
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
			} else if gamma == Gamma::SRGB {
				Formats::RGBA8SRGB
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
			} else if gamma == Gamma::SRGB {
				Formats::RGBA8SRGB
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
		Formats::R16F => Formats::R16F,
		Formats::RGBA16F => Formats::RGBA16F,
		_ => {
			panic!("Unsupported format: {:#?}", source_format);
		}
	}
}

/// Produces one final image payload while retaining intermediate storage only when later stages require random access.
fn produce_image_in<A: Allocator + Clone>(
	description: &ImageDescription,
	source: ImageSource<'_>,
	allocator: A,
	mip_backend: Option<&dyn MipGenerationBackend>,
) -> Result<(Image, Box<[u8], A>, Option<Vec<StreamDescription>>), LoadErrors> {
	let ImageDescription {
		semantic,
		gamma,
		generate_mipmaps,
	} = description;
	let source_format = source.natural_format().ok_or(LoadErrors::FailedToProcess)?;
	let extent = source.extent;

	let compress = should_compress_for_semantic(*semantic);

	let output_format = determine_image_format(source_format, compress, *semantic, *gamma);
	let block_compressed = matches!(
		output_format,
		Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB
	);
	if !*generate_mipmaps && !block_compressed {
		let encoded_size = encoded_mip_level_size(output_format, extent).ok_or(LoadErrors::FailedToProcess)?;
		let mut data = Vec::with_capacity_in(encoded_size, allocator);
		append_canonical_image_in(source, output_format, &mut data).ok_or(LoadErrors::FailedToProcess)?;
		let data = data.into_boxed_slice();
		return Ok((
			Image {
				format: output_format,
				extent: extent.as_array(),
				gamma: *gamma,
				mip_count: 1,
				ibl: None,
				photometry: None,
			},
			data,
			Some(vec![StreamDescription::new("mip[0]", encoded_size, 0)]),
		));
	}

	// The format of the `intermediate` buffer — used for mip generation.
	let intermediate_format = match output_format {
		Formats::BC5 | Formats::BC5SNORM | Formats::BC7 | Formats::BC7SRGB | Formats::RGBA8SRGB => Formats::RGBA8,
		_ => output_format,
	};
	let intermediate =
		canonicalize_image_in(source, intermediate_format, allocator.clone()).ok_or(LoadErrors::FailedToProcess)?;
	let intermediate = intermediate.as_slice();

	let (mip_count, data, streams) = if *generate_mipmaps {
		let level_count = crate::resources::mips::mip_level_count(extent.width(), extent.height())
			.map_err(|_| LoadErrors::FailedToProcess)?;

		let encoded_size = encoded_mip_chain_size(output_format, extent).ok_or(LoadErrors::FailedToProcess)?;

		let mut all_data = Vec::with_capacity_in(encoded_size, allocator.clone());

		let mut streams = Vec::with_capacity(level_count as usize);

		let mut offset: usize = 0;

		// Both backends return one packed lower-level allocation. The common encoder keeps the borrowed base level separate.
		let lower_levels = match mip_backend {
			Some(mip_backend) => {
				mip_backend.generate_lower_levels(intermediate_format, *gamma, extent.width(), extent.height(), intermediate)
			}
			None => CPUMipGenerationBackend.generate_lower_levels(
				intermediate_format,
				*gamma,
				extent.width(),
				extent.height(),
				intermediate,
			),
		}
		.map_err(|_| LoadErrors::FailedToProcess)?;

		append_encoded_mip(
			output_format,
			extent,
			intermediate,
			0,
			&mut all_data,
			&mut streams,
			&mut offset,
			allocator.clone(),
		);

		for (lower_index, level) in lower_levels.levels().enumerate() {
			append_encoded_mip(
				output_format,
				Extent::rectangle(level.width, level.height),
				level.data,
				lower_index + 1,
				&mut all_data,
				&mut streams,
				&mut offset,
				allocator.clone(),
			);
		}

		(1 + lower_levels.len() as u32, all_data.into_boxed_slice(), Some(streams))
	} else {
		let encoded_size = encoded_mip_level_size(output_format, extent).ok_or(LoadErrors::FailedToProcess)?;
		let mut data = Vec::with_capacity_in(encoded_size, allocator.clone());
		append_encoded_level(output_format, extent, intermediate, &mut data, allocator);
		let data = data.into_boxed_slice();

		let streams = Some(vec![StreamDescription::new("mip[0]", data.len(), 0)]);

		(1_u32, data, streams)
	};

	Ok((
		Image {
			format: output_format,
			extent: extent.as_array(),
			gamma: *gamma,
			mip_count,
			ibl: None,
			photometry: None,
		},
		data,
		streams,
	))
}

/// Appends one persisted mip without allocating an intermediate copy for uncompressed formats.
fn append_encoded_mip<A: Allocator + Clone>(
	output_format: Formats,
	extent: Extent,
	data: &[u8],
	index: usize,
	output: &mut Vec<u8, A>,
	streams: &mut Vec<StreamDescription>,
	offset: &mut usize,
	allocator: A,
) {
	let start = output.len();
	append_encoded_level(output_format, extent, data, output, allocator);

	let size = output.len() - start;

	streams.push(StreamDescription::new(format!("mip[{index}]"), size, *offset));

	*offset += size;
}

/// Calculates exact persisted storage for a complete encoded mip chain.
fn encoded_mip_chain_size(format: Formats, extent: Extent) -> Option<usize> {
	mip_extents(extent.width(), extent.height()).try_fold(0usize, |total, (width, height)| {
		total.checked_add(encoded_mip_level_size(format, Extent::rectangle(width, height))?)
	})
}

/// Returns the stored size of one level in a format the image processor can encode.
fn encoded_mip_level_size(format: Formats, extent: Extent) -> Option<usize> {
	match format {
		Formats::BC5
		| Formats::BC5SNORM
		| Formats::BC7
		| Formats::BC7SRGB
		| Formats::RGBA8
		| Formats::RGBA8SRGB
		| Formats::R16F
		| Formats::RGBA16
		| Formats::RGBA16F => format.level_size(extent),
		_ => None,
	}
}

/// Compresses a single mip level to the target `output_format`, or returns the data unchanged for
/// uncompressed formats. Accepts an RGBA8 surface for BC targets, or the natural format otherwise.
#[cfg(test)]
fn compress_bc_level(output_format: Formats, extent: Extent, data: &[u8]) -> Box<[u8]> {
	let mut output = Vec::new();
	append_encoded_level(output_format, extent, data, &mut output, Global);
	output.into_boxed_slice()
}

/// Appends one compressed or uncompressed mip directly to its final packed writer.
fn append_encoded_level<A: Allocator + Clone>(
	output_format: Formats,
	extent: Extent,
	data: &[u8],
	output: &mut Vec<u8, A>,
	allocator: A,
) {
	match output_format {
		Formats::BC5 | Formats::BC5SNORM => {
			// RgSurface<2> expects tightly packed RG pairs (2 bytes per pixel),
			// not interleaved RGBA. Convert the RGBA8 intermediate to RG8
			// before compression to avoid reading B/A as the second pixel's R/G.
			let (rg_data, width, height) = rga_to_rg_surface_in(data, extent, allocator);

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

			output.extend_from_slice(&compressed);
		}
		Formats::BC7 | Formats::BC7SRGB => {
			let (data, width, height) = rgba8_bc_compression_surface_in(extent, data, allocator);
			let data = data.as_slice();

			let expected_surface_bytes = width as usize * height as usize * 4;

			assert_eq!(
				data.len(),
				expected_surface_bytes,
				"BC7 padded surface size mismatch. The most likely cause is that the BC compression padding copied an unexpected number of RGBA8 texels. format={output_format:?}, extent={extent:?}, padded_width={width}, padded_height={height}, data_len={}, expected={expected_surface_bytes}",
				data.len()
			);

			let rgba_surface = intel_tex_2::RgbaSurface {
				data,
				width,
				height,
				stride: width * 4,
			};

			let settings = bc7_compression_settings(data);

			let compressed = intel_tex_2::bc7::compress_blocks(&settings, &rgba_surface);

			let expected_payload_bytes = width as usize / 4 * (height as usize / 4) * 16;

			assert_eq!(
				compressed.len(),
				expected_payload_bytes,
				"BC7 payload size mismatch. The most likely cause is that the compressor block count no longer matches the padded image dimensions. format={output_format:?}, extent={extent:?}, padded_width={width}, padded_height={height}, compressed_len={}, expected={expected_payload_bytes}",
				compressed.len()
			);

			output.extend_from_slice(&compressed);
		}
		Formats::RGB8
		| Formats::RGBA8
		| Formats::RGBA8SRGB
		| Formats::RGB16
		| Formats::RGBA16
		| Formats::R16F
		| Formats::RGBA16F => {
			output.extend_from_slice(data);
		}
		_ => {
			panic!("Unsupported format")
		}
	};
}

#[cfg(test)]
mod tests {
	use utils::Extent;

	use super::{
		ImageDescription, ImageSource, Semantic, bc7_compression_settings, compress_bc_level, gamma_from_semantic,
		guess_semantic_from_name, process_image, rga_to_rg_surface,
	};
	use crate::{
		asset::ResourceId,
		resources::image::Image,
		types::{Formats, Gamma},
	};

	fn image_source(extent: Extent, format: Formats, data: &[u8]) -> ImageSource<'_> {
		ImageSource::from_format(extent, format, data).expect("test format should be a supported image source")
	}

	#[test]
	fn infers_texture_semantic_and_default_gamma_from_asset_name() {
		let cases = [
			("textures/brick_wall_Base_color.png", Semantic::Albedo, Gamma::SRGB),
			("textures/brick_wall_Diffuse.png", Semantic::Albedo, Gamma::SRGB),
			("textures/brick_wall_Albedo.png", Semantic::Albedo, Gamma::SRGB),
			("textures/brick_wall_Normal.png", Semantic::Normal, Gamma::Linear),
			("textures/brick_wall_Metallic.png", Semantic::Metallic, Gamma::Linear),
			("textures/brick_wall_Roughness.png", Semantic::Roughness, Gamma::Linear),
			("textures/brick_wall_Emissive.png", Semantic::Emissive, Gamma::SRGB),
			("textures/brick_wall_Height.png", Semantic::Height, Gamma::Linear),
			("textures/brick_wall_Opacity.png", Semantic::Opacity, Gamma::Linear),
			("textures/brick_wall_Displacement.png", Semantic::Displacement, Gamma::Linear),
			("textures/brick_wall_AO.png", Semantic::AO, Gamma::Linear),
			("textures/brick_wall_Color.png", Semantic::Other, Gamma::SRGB),
			("textures/diffuse_bomb_icon.png", Semantic::Other, Gamma::SRGB),
			("textures/icon_diffuse.png", Semantic::Albedo, Gamma::SRGB),
			("textures/DiffuseBombIcon.png", Semantic::Other, Gamma::SRGB),
			("textures/NormalityChecker.png", Semantic::Other, Gamma::SRGB),
			("textures/AOGenerator.png", Semantic::Other, Gamma::SRGB),
		];

		for (id, expected_semantic, expected_gamma) in cases {
			let semantic = guess_semantic_from_name(ResourceId::new(id).get_base());

			assert_eq!(
				(semantic, gamma_from_semantic(semantic)),
				(expected_semantic, expected_gamma),
				"asset: {id}"
			);
		}
	}

	#[test]
	fn process_image_expands_rgb8_into_rgba8_without_compression() {
		let extent = Extent::rectangle(2, 1);
		let description = ImageDescription {
			gamma: Gamma::SRGB,
			semantic: Semantic::Other,
			generate_mipmaps: false,
		};
		let source = [1, 2, 3, 4, 5, 6];

		let (asset, data) = process_image(
			ResourceId::new("textures/test.png"),
			description,
			image_source(extent, Formats::RGB8, &source),
		)
		.expect("Image processing should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		assert_eq!(asset.id, "textures/test.png");
		assert_eq!(asset.class, "Image");
		assert_eq!(image.format, Formats::RGBA8SRGB);
		assert_eq!(image.gamma, Gamma::SRGB);
		assert_eq!(image.extent, [2, 1, 0]);
		assert_eq!(&*data, &[1, 2, 3, 0xFF, 4, 5, 6, 0xFF]);
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
	fn process_image_compresses_rgb16_albedo_to_bc7() {
		// Regression: the old code built an RGBA16 intermediate (8 bytes/pixel) but passed it to
		// the BC7 compressor with stride = width * 4 (an RGBA8 stride), halving the effective row
		// width and producing horizontal stripes. The correct path converts RGB16 → RGBA8 first.
		let extent = Extent::rectangle(4, 4);
		let description = ImageDescription {
			gamma: Gamma::Linear,
			semantic: Semantic::Albedo,
			generate_mipmaps: false,
		};

		// RGB16: 3 channels × 2 bytes = 6 bytes per pixel
		let source = vec![128_u8; 4 * 4 * 6].into_boxed_slice();

		let (asset, data) = process_image(
			ResourceId::new("textures/albedo16.png"),
			description,
			image_source(extent, Formats::RGB16, &source),
		)
		.expect("RGB16 albedo processing should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		assert_eq!(image.format, Formats::BC7);
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
	fn process_image_with_mipmaps_produces_correct_mip_count_for_bc5_normal_map() {
		// BC5 compresses RGBA8 intermediate in 4×4 blocks.
		let width = 8_u32;

		let height = 8_u32;
		let extent = Extent::rectangle(width, height);

		let description = ImageDescription {
			gamma: Gamma::Linear,
			semantic: Semantic::Normal,
			generate_mipmaps: true,
		};

		// RGBA8: 4 bytes/pixel
		let source = vec![128_u8; (width * height * 4) as usize].into_boxed_slice();

		let (asset, data) = process_image(
			ResourceId::new("textures/mip_normal_bc5.png"),
			description,
			image_source(extent, Formats::RGBA8, &source),
		)
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
}

/// Selects BC7 compressor settings that favor quality enough to avoid visible block-row artifacts.
fn bc7_compression_settings(data: &[u8]) -> intel_tex_2::bc7::EncodeSettings {
	let has_alpha = data.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0xFF);

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
) -> (CanonicalImageData<'_, A>, u32, u32) {
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
	if padded_width == width && padded_height == height {
		return (CanonicalImageData::Borrowed(data), width, height);
	}

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

	(CanonicalImageData::Owned(padded), padded_width, padded_height)
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

use std::alloc::{Allocator, Global};

use utils::Extent;

use crate::{
	ProcessedAsset, StreamDescription,
	asset::{ResourceId, handler::LoadErrors, resource_id::ResourceIdBase},
	resources::{
		image::Image,
		mips::{CPUMipGenerationBackend, MipGenerationBackend, mip_extents},
	},
	types::{Formats, Gamma},
};
