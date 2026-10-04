mod decode;
mod source;

pub(crate) use decode::{decode_rgba16f_in, png_declared_gamma};
pub(crate) use source::{CanonicalImageData, canonicalize_rgba16f_in};
pub use source::{ImageSource, SourceChannels, SourceEncoding};
use source::{append_canonical_image_in, canonicalize_image_in};

/// The `Semantic` enum tells the image processor how a texture is sampled, so it can pick gamma, format, and packing.
///
/// Importers infer it from material usage, or from the file name through [`guess_semantic_from_name`].
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Semantic {
	Albedo,
	Normal,
	/// A map whose metallic value is read from one channel; stored with all of its channels.
	Metallic,
	/// A map whose roughness value is read from one channel; stored with all of its channels.
	Roughness,
	/// A glTF metallic-roughness map read only through its green and blue channels.
	///
	/// The processor stores it as two channels through [`METALLIC_ROUGHNESS_PACKING`], and material generators remap
	/// their channel reads with the same table. See [`crate::pbr::BrdfMaterialDescription::pack_texture_channels`].
	MetallicRoughness,
	Emissive,
	Height,
	Opacity,
	Displacement,
	AO,
	Other,
}

/// The `ChannelPacking` struct selects which two source channels a packed image keeps, in stored order.
///
/// The image processor moves the selected channels into the first two channels of its filtering surface and stores
/// the result as `RG8` or `RG16`. Material generators remap their channel reads through
/// [`Self::stored_channel`], so a packed image is sampled where it was stored.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct ChannelPacking {
	/// Source channel index stored in each of the two output channels.
	pub source_channels: [usize; 2],
}

impl ChannelPacking {
	/// Returns the stored channel index that holds `source_channel`, or `None` when the packing drops it.
	pub fn stored_channel(self, source_channel: usize) -> Option<usize> {
		self.source_channels.iter().position(|&kept| kept == source_channel)
	}

	/// Moves the selected channels of every RGBA texel to the front, in place.
	///
	/// `SAMPLE` is the byte size of one channel. The remaining channels keep stale values; the level encoder drops
	/// them.
	fn pack_in_place<const SAMPLE: usize>(self, data: &mut [u8]) {
		let [first, second] = self.source_channels;
		for texel in data.as_chunks_mut::<SAMPLE>().0.chunks_exact_mut(4) {
			// Both samples are read before either is written, so any channel pair moves correctly.
			[texel[0], texel[1]] = [texel[first], texel[second]];
		}
	}
}

/// glTF stores roughness in green and metallic in blue; packed maps keep them as red and green.
pub const METALLIC_ROUGHNESS_PACKING: ChannelPacking = ChannelPacking { source_channels: [1, 2] };

/// Returns the packing an image with `semantic` is stored with, or `None` when it keeps all of its channels.
pub fn channel_packing_for_semantic(semantic: Semantic) -> Option<ChannelPacking> {
	(semantic == Semantic::MetallicRoughness).then_some(METALLIC_ROUGHNESS_PACKING)
}

/// The `ImageDescription` struct selects semantic processing and gamma for one decoded image.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct ImageDescription {
	pub gamma: Gamma,
	pub semantic: Semantic,
}

/// Processes image pixels into a stored image, using `allocator` for transient and output buffers.
///
/// Material importers pass their shared `mip_generator`, which stores a full mip chain after the base level, and the
/// call suspends while a GPU generator serves the request. Standalone image handlers pass `None`, so their authored
/// texture payload stays one level. A single block-compressed or packed level is encoded on the CPU. Packed semantics
/// move their kept channels to the front of the filtering surface first, so every level filters and stores the same
/// texels the unpacked image would.
pub async fn process_image_in<'a, A: Allocator + Clone>(
	id: ResourceId<'a>,
	description: ImageDescription,
	source: ImageSource<'_>,
	allocator: A,
	mip_generator: Option<&MipGenerator>,
) -> Result<(ProcessedAsset, Box<[u8], A>), LoadErrors> {
	let ImageDescription { semantic, gamma } = description;
	let source_format = source.natural_format().ok_or(LoadErrors::FailedToProcess)?;
	let extent = source.extent;
	let packing = channel_packing_for_semantic(semantic);
	let output_format = determine_image_format(source_format, semantic, gamma);

	// Every level is one stream, and the levels follow each other without padding.
	let levels = mip_extents(extent.width(), extent.height()).take(if mip_generator.is_some() { usize::MAX } else { 1 });
	let mut streams = Vec::new();
	let mut size = 0usize;
	for (index, (width, height)) in levels.enumerate() {
		let level_size =
			encoded_mip_level_size(output_format, Extent::rectangle(width, height)).ok_or(LoadErrors::FailedToProcess)?;
		streams.push(StreamDescription::new(format!("mip[{index}]"), level_size, size));
		size = size.checked_add(level_size).ok_or(LoadErrors::FailedToProcess)?;
	}

	let mut data = Vec::with_capacity_in(size, allocator.clone());
	// Block-compressed formats have no per-texel size.
	let block_compressed = output_format.texel_bytes().is_none();
	if mip_generator.is_none() && !block_compressed && packing.is_none() {
		// One level that needs no filtering, compression, or packing streams straight from the source.
		append_canonical_image_in(source, output_format, &mut data).ok_or(LoadErrors::FailedToProcess)?;
	} else {
		// Filtering, block compression, and packing all read a canonical copy of the source.
		let filtering_format = filtering_format(output_format);
		let mut intermediate =
			canonicalize_image_in(source, filtering_format, allocator.clone()).ok_or(LoadErrors::FailedToProcess)?;
		if let Some(packing) = packing {
			let intermediate = intermediate.to_mut(allocator.clone());
			match filtering_format {
				Formats::RGBA8 => packing.pack_in_place::<1>(intermediate),
				Formats::RGBA16 => packing.pack_in_place::<2>(intermediate),
				_ => return Err(LoadErrors::FailedToProcess),
			}
		}
		let intermediate = intermediate.as_slice();

		data.resize(size, 0);
		if let Some(mip_generator) = mip_generator {
			mip_generator
				.encode_mip_chain(output_format, gamma, extent.width(), extent.height(), intermediate, &mut data)
				.await
				.map_err(|_| LoadErrors::FailedToProcess)?;
		} else {
			encode_level_in(output_format, extent, intermediate, &mut data, allocator);
		}
	}

	let image = Image {
		format: output_format,
		extent: extent.as_array(),
		gamma,
		mip_count: streams.len() as u32,
		ibl: None,
		photometry: None,
	};
	Ok((ProcessedAsset::new(id, image).with_streams(streams), data.into_boxed_slice()))
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
		_ => Gamma::Linear,
	}
}

pub fn should_compress_for_semantic(semantic: Semantic) -> bool {
	matches!(semantic, Semantic::Albedo | Semantic::Normal)
}

/// Selects the stored format for a source format, given how the image is sampled.
///
/// Eight- and sixteen-bit integer sources keep their depth. Packed semantics store two channels, normal maps compress
/// to BC5, other images that [`should_compress_for_semantic`] selects to BC7, and everything else stays RGBA.
pub fn determine_image_format(source_format: Formats, semantic: Semantic, gamma: Gamma) -> Formats {
	let sixteen_bit = match source_format {
		Formats::RGB8 | Formats::RGBA8 => false,
		Formats::RGB16 | Formats::RGBA16 => true,
		Formats::R16F => return Formats::R16F,
		Formats::RGBA16F => return Formats::RGBA16F,
		_ => panic!("Unsupported format: {:#?}", source_format),
	};
	let packed = channel_packing_for_semantic(semantic).is_some();
	let compress = should_compress_for_semantic(semantic);
	match semantic {
		_ if packed && sixteen_bit => Formats::RG16,
		_ if packed => Formats::RG8,
		Semantic::Normal if compress => Formats::BC5,
		_ if compress && gamma == Gamma::SRGB => Formats::BC7SRGB,
		_ if compress => Formats::BC7,
		_ if sixteen_bit => Formats::RGBA16,
		_ if gamma == Gamma::SRGB => Formats::RGBA8SRGB,
		_ => Formats::RGBA8,
	}
}

#[cfg(test)]
mod tests {
	use std::alloc::Global;

	use utils::Extent;

	use super::{ImageDescription, ImageSource, Semantic, gamma_from_semantic, guess_semantic_from_name, process_image_in};
	use crate::{
		asset::ResourceId,
		resources::{image::Image, mips::MipGenerator},
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

	#[crate::r#async::test]
	async fn process_image_expands_rgb8_into_rgba8_without_compression() {
		let extent = Extent::rectangle(2, 1);
		let description = ImageDescription {
			gamma: Gamma::SRGB,
			semantic: Semantic::Other,
		};
		let source = [1, 2, 3, 4, 5, 6];

		let (asset, data) = process_image_in(
			ResourceId::new("textures/test.png"),
			description,
			image_source(extent, Formats::RGB8, &source),
			Global,
			None,
		)
		.await
		.expect("Image processing should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		assert_eq!(asset.id, "textures/test.png");
		assert_eq!(asset.class, "Image");
		assert_eq!(image.format, Formats::RGBA8SRGB);
		assert_eq!(image.gamma, Gamma::SRGB);
		assert_eq!(image.extent, [2, 1, 0]);
		assert_eq!(&*data, &[1, 2, 3, 0xFF, 4, 5, 6, 0xFF]);
	}

	/// Processes `source` as a metallic-roughness map and as an unpacked metallic map, and returns both payloads.
	async fn packed_and_unpacked(
		extent: Extent,
		format: Formats,
		source: &[u8],
		generate_mipmaps: bool,
	) -> (Image, Box<[u8]>, Box<[u8]>) {
		let process = async |semantic| {
			let description = ImageDescription {
				gamma: Gamma::Linear,
				semantic,
			};
			let (asset, data) = process_image_in(
				ResourceId::new("textures/metallic_roughness.png"),
				description,
				image_source(extent, format, source),
				Global,
				generate_mipmaps.then_some(&MipGenerator::Cpu),
			)
			.await
			.expect("metallic-roughness processing should succeed");
			let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");
			(image, data)
		};
		let (packed, packed_data) = process(Semantic::MetallicRoughness).await;
		let (_, unpacked_data) = process(Semantic::Metallic).await;
		(packed, packed_data, unpacked_data)
	}

	#[crate::r#async::test]
	async fn process_image_packs_metallic_roughness_green_and_blue_into_rg8() {
		let extent = Extent::rectangle(2, 2);
		let source = [10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 0, 100, 110, 120, 128];

		let (image, data, _) = packed_and_unpacked(extent, Formats::RGBA8, &source, false).await;

		assert_eq!(image.format, Formats::RG8);
		assert_eq!(image.gamma, Gamma::Linear);
		assert_eq!(image.mip_count, 1);
		assert_eq!(&*data, &[20, 30, 50, 60, 80, 90, 110, 120]);
	}

	#[crate::r#async::test]
	async fn packed_metallic_roughness_mip_chain_matches_the_unpacked_green_and_blue_channels() {
		// Filtering is per channel, so packing before filtering must store exactly the G and B texels the RGBA8 chain
		// stores, on every level.
		let (width, height) = (8_u32, 4_u32);
		let source = (0..width * height)
			.flat_map(|index| [index as u8, (index * 7 % 251) as u8, (index * 13 % 241) as u8, 255])
			.collect::<Vec<_>>();

		let (image, packed, unpacked) =
			packed_and_unpacked(Extent::rectangle(width, height), Formats::RGBA8, &source, true).await;

		assert_eq!(image.format, Formats::RG8);
		assert_eq!(image.mip_count, 4);
		assert_eq!(packed.len(), (32 + 8 + 2 + 1) * 2);
		let expected = unpacked
			.as_chunks::<4>()
			.0
			.iter()
			.flat_map(|texel| [texel[1], texel[2]])
			.collect::<Vec<_>>();
		assert_eq!(&*packed, &*expected);
	}

	#[crate::r#async::test]
	async fn process_image_packs_sixteen_bit_metallic_roughness_into_rg16() {
		let extent = Extent::rectangle(2, 1);
		// RGB16 little-endian texels: (1, 2, 3) and (4, 5, 6).
		let source = [1_u16, 2, 3, 4, 5, 6]
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect::<Vec<_>>();

		let (image, data, _) = packed_and_unpacked(extent, Formats::RGB16, &source, false).await;

		assert_eq!(image.format, Formats::RG16);
		assert_eq!(&*data, &[2, 0, 3, 0, 5, 0, 6, 0]);
	}

	#[crate::r#async::test]
	async fn process_image_compresses_rgb16_albedo_to_bc7() {
		// Regression: the old code built an RGBA16 intermediate (8 bytes/pixel) but passed it to
		// the BC7 compressor with stride = width * 4 (an RGBA8 stride), halving the effective row
		// width and producing horizontal stripes. The correct path converts RGB16 → RGBA8 first.
		let extent = Extent::rectangle(4, 4);
		let description = ImageDescription {
			gamma: Gamma::Linear,
			semantic: Semantic::Albedo,
		};

		// RGB16: 3 channels × 2 bytes = 6 bytes per pixel
		let source = vec![128_u8; 4 * 4 * 6].into_boxed_slice();

		let (asset, data) = process_image_in(
			ResourceId::new("textures/albedo16.png"),
			description,
			image_source(extent, Formats::RGB16, &source),
			Global,
			None,
		)
		.await
		.expect("RGB16 albedo processing should succeed");

		let image: Image = crate::from_slice(&asset.resource).expect("Processed asset should deserialize as an image");

		assert_eq!(image.format, Formats::BC7);
		assert_eq!(image.extent, [4, 4, 0]);
		// 4×4 image → 1×1 block grid → 1 block × 16 bytes

		assert_eq!(data.len(), 16);
	}

	#[crate::r#async::test]
	async fn process_image_with_mipmaps_produces_correct_mip_count_for_bc5_normal_map() {
		// BC5 compresses RGBA8 intermediate in 4×4 blocks.
		let width = 8_u32;

		let height = 8_u32;
		let extent = Extent::rectangle(width, height);

		let description = ImageDescription {
			gamma: Gamma::Linear,
			semantic: Semantic::Normal,
		};

		// RGBA8: 4 bytes/pixel
		let source = vec![128_u8; (width * height * 4) as usize].into_boxed_slice();

		let (asset, data) = process_image_in(
			ResourceId::new("textures/mip_normal_bc5.png"),
			description,
			image_source(extent, Formats::RGBA8, &source),
			Global,
			Some(&MipGenerator::Cpu),
		)
		.await
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

use std::alloc::Allocator;

use utils::Extent;

use crate::{
	ProcessedAsset, StreamDescription,
	asset::{ResourceId, handler::LoadErrors, resource_id::ResourceIdBase},
	resources::{
		image::Image,
		mips::{MipGenerator, encode_level_in, encoded_mip_level_size, filtering_format, mip_extents},
	},
	types::{Formats, Gamma},
};
