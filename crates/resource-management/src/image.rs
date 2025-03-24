use crate::{asset::ResourceId, types::{Formats, Gamma}, Model, Reference, ReferenceModel, Resource, SolveErrors, Solver, resource};

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct Image {
	// pub compression: Option<CompressionSchemes>,
	pub format: Formats,
	pub gamma: Gamma,
	pub extent: [u32; 3],
}

impl Resource for Image {
	fn get_class(&self) -> &'static str { "Image" }

	type Model = Image;
}

impl Model for Image {
	fn get_class() -> &'static str { "Image" }
}

impl <'de> Solver<'de, Reference<Image>> for ReferenceModel<Image> {
	fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Image>, SolveErrors> {
		let (gr, reader) = storage_backend.read(ResourceId::new(&self.id)).ok_or_else(|| SolveErrors::StorageError)?;
		let Image { format, extent, gamma } = crate::from_slice(&gr.resource).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::from_model(self, Image {
			format,
			extent,
			gamma,
		}, reader))
	}
}
