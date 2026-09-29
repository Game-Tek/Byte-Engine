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
///
/// Mip chains go to `mip_backend`, or to the CPU backend when there is none. A single block-compressed level is encoded
/// on the CPU.
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

	// Filtering and block compression both read a canonical copy of the source.
	let intermediate =
		canonicalize_image_in(source, filtering_format(output_format), allocator.clone()).ok_or(LoadErrors::FailedToProcess)?;
	let intermediate = intermediate.as_slice();

	// Every level is one stream, and the levels follow each other without padding.
	let levels = mip_extents(extent.width(), extent.height()).take(if *generate_mipmaps { usize::MAX } else { 1 });
	let mut streams = Vec::new();
	let mut size = 0usize;
	for (index, (width, height)) in levels.enumerate() {
		let level_size =
			encoded_mip_level_size(output_format, Extent::rectangle(width, height)).ok_or(LoadErrors::FailedToProcess)?;
		streams.push(StreamDescription::new(format!("mip[{index}]"), level_size, size));
		size = size.checked_add(level_size).ok_or(LoadErrors::FailedToProcess)?;
	}

	let mut data = Vec::with_capacity_in(size, allocator.clone());
	data.resize(size, 0);
	if *generate_mipmaps {
		mip_backend
			.unwrap_or(&CPUMipGenerationBackend)
			.encode_mip_chain(
				output_format,
				*gamma,
				extent.width(),
				extent.height(),
				intermediate,
				&mut data,
			)
			.map_err(|_| LoadErrors::FailedToProcess)?;
	} else {
		encode_level_in(output_format, extent, intermediate, &mut data, allocator);
	}

	Ok((
		Image {
			format: output_format,
			extent: extent.as_array(),
			gamma: *gamma,
			mip_count: streams.len() as u32,
			ibl: None,
			photometry: None,
		},
		data.into_boxed_slice(),
		Some(streams),
	))
}

#[cfg(test)]
mod tests {
	use utils::Extent;

	use super::{ImageDescription, ImageSource, Semantic, gamma_from_semantic, guess_semantic_from_name, process_image};
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

use std::alloc::{Allocator, Global};

use utils::Extent;

use crate::{
	ProcessedAsset, StreamDescription,
	asset::{ResourceId, handler::LoadErrors, resource_id::ResourceIdBase},
	resources::{
		image::Image,
		mips::{
			CPUMipGenerationBackend, MipGenerationBackend, encode_level_in, encoded_mip_level_size, filtering_format,
			mip_extents,
		},
	},
	types::{Formats, Gamma},
};
