use crate::types::{Formats, Gamma};

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
}

fn default_mip_count() -> u32 {
	1
}

super::impl_direct_resource!(Image, "Image");

#[cfg(test)]
mod tests {
	use super::Image;
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
		assert_eq!(reference.resource.get_class(), "Image");
	}

	#[test]
	fn image_reference_solve_distinguishes_missing_and_malformed_storage() {
		let image = Image {
			format: Formats::RGBA8,
			gamma: Gamma::Linear,
			extent: [1, 1, 1],
			mip_count: 1,
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
}
