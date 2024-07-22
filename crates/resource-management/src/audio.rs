use polodb_core::bson;
use serde::Deserialize;

use crate::{asset::ResourceId, types::BitDepths, Model, Reference, ReferenceModel, Resource, SolveErrors, Solver, StorageBackend};

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
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Reference<Audio>, SolveErrors> {
		let (resource, reader) = storage_backend.read(ResourceId::new(&self.id)).await.ok_or_else(|| SolveErrors::StorageError)?;
		let Audio { bit_depth, channel_count, sample_rate, sample_count } = Audio::deserialize(bson::Deserializer::new(resource.resource)).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::from_model(self, Audio {
			bit_depth,
			channel_count,
			sample_rate,
			sample_count,
		}, reader))
	}
}
