use crate::{asset::ResourceId, types::BitDepths, Model, Reference, ReferenceModel, Resource, SolveErrors, Solver, resource};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Audio {
	pub bit_depth: BitDepths,
	pub channel_count: u16,
	pub sample_rate: u32,
	pub sample_count: u32,
}

impl Resource for Audio {
	fn get_class(&self) -> &'static str { "Audio" }

	type Model = Audio;
}

impl Model for Audio {
	fn get_class() -> &'static str { "Audio" }
}

impl <'de> Solver<'de, Reference<Audio>> for ReferenceModel<Audio> {
	fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Audio>, SolveErrors> {
		let (resource, reader) = storage_backend.read(ResourceId::new(&self.id)).ok_or_else(|| SolveErrors::StorageError)?;
		let Audio { bit_depth, channel_count, sample_rate, sample_count } = crate::from_slice(&resource.resource).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::from_model(self, Audio {
			bit_depth,
			channel_count,
			sample_rate,
			sample_count,
		}, reader))
	}
}
