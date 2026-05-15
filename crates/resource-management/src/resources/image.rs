use crate::{
	resource,
	solver::SolveErrors,
	types::{Formats, Gamma},
	Model, Reference, ReferenceModel, Resource, Solver,
};

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

impl Resource for Image {
	fn get_class(&self) -> &'static str {
		"Image"
	}

	type Model = Image;
}

impl Model for Image {
	fn get_class() -> &'static str {
		"Image"
	}
}

impl<'de> Solver<'de, Reference<Image>> for ReferenceModel<Image> {
	fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Image>, SolveErrors> {
		let (gr, reader) = storage_backend.read(self.id()).ok_or_else(|| SolveErrors::StorageError)?;
		let Image {
			format,
			extent,
			gamma,
			mip_count,
		} = crate::from_slice(&gr.resource).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::from_model(
			self,
			Image {
				format,
				extent,
				gamma,
				mip_count,
			},
			reader,
		))
	}
}
