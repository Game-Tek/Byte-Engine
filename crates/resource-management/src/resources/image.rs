use crate::types::{Formats, Gamma};

/// The stream name for the original image's base mip.
pub const IMAGE_BASE_MIP_STREAM_NAME: &str = "mip[0]";

/// The stream name for the environment's diffuse irradiance image.
pub const IBL_DIFFUSE_IRRADIANCE_STREAM_NAME: &str = "ibl.diffuse_irradiance.mip[0]";

/// Number of perceptual-roughness levels baked for specular image-based lighting.
pub const IBL_PREFILTERED_SPECULAR_MIP_COUNT: u32 = 8;

const IBL_PREFILTERED_SPECULAR_STREAM_PREFIX: &str = "ibl.prefiltered_specular.mip";

/// Returns the stream name for one roughness-prefiltered specular mip.
pub fn ibl_prefiltered_specular_stream_name(mip_level: u32) -> String {
	format!("{IBL_PREFILTERED_SPECULAR_STREAM_PREFIX}[{mip_level}]")
}

/// The `ImageSubresource` struct provides upload metadata for one image in a parent image's binary streams.
#[derive(
	Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, PartialEq, Eq,
)]
pub struct ImageSubresource {
	pub format: Formats,
	pub gamma: Gamma,
	pub extent: [u32; 3],
	/// Number of mip levels stored for this subresource, including its base level.
	pub mip_count: u32,
}

/// The `ImageIbl` struct groups the baked image-based-lighting maps derived from an environment image.
#[derive(
	Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, PartialEq, Eq,
)]
pub struct ImageIbl {
	pub diffuse_irradiance: ImageSubresource,
	/// Roughness maps linearly to mip level from zero through `mip_count - 1`.
	pub prefiltered_specular: ImageSubresource,
}

/// The `Image` struct stores the metadata needed to upload a baked texture to the GPU.
#[derive(Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone)]
pub struct Image {
	// pub compression: Option<CompressionSchemes>,
	pub format: Formats,
	pub gamma: Gamma,
	pub extent: [u32; 3],
	/// Number of mip levels stored in the accompanying data buffer, including the base level.
	#[serde(default = "default_mip_count")]
	pub mip_count: u32,
	/// Baked lighting maps stored as named binary subresources of this image.
	#[serde(default)]
	pub ibl: Option<ImageIbl>,
}

fn default_mip_count() -> u32 {
	1
}

super::impl_direct_resource!(Image, "Image");

#[cfg(test)]
mod tests {
	use super::{Image, ImageIbl, ImageSubresource};
	use crate::{
		asset::ResourceId,
		resource::{storage_backend::tests::TestStorageBackend, WriteStorageBackend},
		solver::SolveErrors,
		types::{Formats, Gamma},
		ProcessedAsset, ReferenceModel, Resource, Solver,
	};

	#[test]
	fn image_reference_solve_restores_metadata_and_binary_reader() {
		let image = Image {
			format: Formats::BC7SRGB,
			gamma: Gamma::SRGB,
			extent: [8, 4, 1],
			mip_count: 4,
			ibl: Some(ImageIbl {
				diffuse_irradiance: ImageSubresource {
					format: Formats::RGBA16F,
					gamma: Gamma::Linear,
					extent: [32, 16, 1],
					mip_count: 1,
				},
				prefiltered_specular: ImageSubresource {
					format: Formats::RGBA16F,
					gamma: Gamma::Linear,
					extent: [128, 64, 1],
					mip_count: 8,
				},
			}),
		};
		let model = ReferenceModel::new("texture.image", 99, 3, &image, None);
		let storage = TestStorageBackend::new();
		storage
			.store(ProcessedAsset::new(ResourceId::new("texture.image"), image), &[1, 2, 3])
			.unwrap();

		let reference = model.solve(&storage).expect("stored image metadata");
		assert_eq!(reference.id(), "texture.image");
		assert_eq!(reference.hash(), 99);
		assert_eq!(reference.size, 3);
		assert_eq!(reference.resource.format, Formats::BC7SRGB);
		assert_eq!(reference.resource.gamma, Gamma::SRGB);
		assert_eq!(reference.resource.extent, [8, 4, 1]);
		assert_eq!(reference.resource.mip_count, 4);
		let ibl = reference.resource.ibl.as_ref().expect("stored IBL metadata");
		assert_eq!(ibl.diffuse_irradiance.extent, [32, 16, 1]);
		assert_eq!(ibl.prefiltered_specular.extent, [128, 64, 1]);
		assert_eq!(ibl.prefiltered_specular.mip_count, 8);
		assert_eq!(reference.resource.get_class(), "Image");
	}

	#[test]
	fn image_reference_solve_distinguishes_missing_and_malformed_storage() {
		let image = Image {
			format: Formats::RGBA8,
			gamma: Gamma::Linear,
			extent: [1, 1, 1],
			mip_count: 1,
			ibl: None,
		};
		let missing = ReferenceModel::new("missing.image", 0, 0, &image, None);
		assert!(matches!(
			missing.solve(&TestStorageBackend::new()),
			Err(SolveErrors::StorageError)
		));

		let storage = TestStorageBackend::new();
		storage
			.store(
				ProcessedAsset::new_with_serialized("broken.image", "Image", vec![1, 2, 3]),
				&[],
			)
			.unwrap();
		let broken = ReferenceModel::new("broken.image", 0, 0, &image, None);
		assert!(matches!(broken.solve(&storage), Err(SolveErrors::DeserializationFailed(_))));
	}

	#[test]
	fn ibl_stream_names_are_stable_and_level_specific() {
		assert_eq!(super::IBL_PREFILTERED_SPECULAR_MIP_COUNT, 8);
		assert_eq!(super::IMAGE_BASE_MIP_STREAM_NAME, "mip[0]");
		assert_eq!(super::IBL_DIFFUSE_IRRADIANCE_STREAM_NAME, "ibl.diffuse_irradiance.mip[0]");
		assert_eq!(
			super::ibl_prefiltered_specular_stream_name(0),
			"ibl.prefiltered_specular.mip[0]"
		);
		assert_eq!(
			super::ibl_prefiltered_specular_stream_name(7),
			"ibl.prefiltered_specular.mip[7]"
		);
	}
}
