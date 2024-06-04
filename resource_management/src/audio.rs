use polodb_core::bson;
use serde::Deserialize;

use crate::{resource::resource_handler::ReadTargets, types::BitDepths, LoadResults, Loader, Model, Reference, ReferenceModel, Resource, SolveErrors, Solver, StorageBackend};

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

impl <'a, 'de> Solver<'de, Reference<'a, Audio>> for ReferenceModel<Audio> {
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Reference<'a, Audio>, SolveErrors> {
		let (resource, reader) = storage_backend.read(&self.id).await.ok_or_else(|| SolveErrors::StorageError)?;
		let Audio { bit_depth, channel_count, sample_rate, sample_count } = Audio::deserialize(bson::Deserializer::new(self.resource)).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::new(&self.id, self.hash, resource.size, Audio {
			bit_depth,
			channel_count,
			sample_rate,
			sample_count,
		}, reader))
	}
}

impl <'a> Loader for Reference<'a, Audio> {
	async fn load(mut self,) -> Result<Self, LoadResults> {
		let reader = &mut self.reader;

		if let Some(read_target) = &mut self.read_target {
			match read_target {
				ReadTargets::Buffer(buffer) => {
					reader.read_into(0, buffer).await.ok_or(LoadResults::LoadFailed)?;
				},
				ReadTargets::Box(buffer) => {
					reader.read_into(0, buffer).await.ok_or(LoadResults::LoadFailed)?;
				},
				_ => {
					return Err(LoadResults::NoReadTarget);
				}
				
			}
		}

		Ok(self)
	}
}