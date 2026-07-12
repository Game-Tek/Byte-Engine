use crate::{resource, solver::SolveErrors, types::BitDepths, Model, Reference, ReferenceModel, Resource, Solver};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Audio {
	pub bit_depth: BitDepths,
	pub channel_count: u16,
	pub sample_rate: u32,
	pub sample_count: u32,
}

impl Resource for Audio {
	fn get_class(&self) -> &'static str {
		"Audio"
	}

	type Model = Audio;
}

impl Model for Audio {
	fn get_class() -> &'static str {
		"Audio"
	}
}

impl<'de> Solver<'de, Reference<Audio>> for ReferenceModel<Audio> {
	fn solve(self, storage_backend: &dyn resource::ReadStorageBackend) -> Result<Reference<Audio>, SolveErrors> {
		let (resource, reader) = storage_backend.read(self.id()).ok_or(SolveErrors::StorageError)?;
		let Audio {
			bit_depth,
			channel_count,
			sample_rate,
			sample_count,
		} = crate::from_slice(&resource.resource).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::from_model(
			self,
			Audio {
				bit_depth,
				channel_count,
				sample_rate,
				sample_count,
			},
			reader,
		))
	}
}

#[cfg(test)]
mod tests {
	use super::Audio;
	use crate::{
		asset::ResourceId,
		resource::{storage_backend::tests::TestStorageBackend, WriteStorageBackend},
		types::BitDepths,
		ProcessedAsset, ReferenceModel, Resource, Solver,
	};

	#[test]
	fn audio_reference_solve_preserves_playback_metadata() {
		let audio = Audio {
			bit_depth: BitDepths::TwentyFour,
			channel_count: 2,
			sample_rate: 48_000,
			sample_count: 9_600,
		};
		let model = ReferenceModel::new("sound.audio", 7, 5, &audio, None);
		let storage = TestStorageBackend::new();
		storage
			.store(ProcessedAsset::new(ResourceId::new("sound.audio"), audio), &[1, 2, 3, 4, 5])
			.unwrap();

		let reference = model.solve(&storage).expect("stored audio metadata");
		assert_eq!(reference.id(), "sound.audio");
		assert_eq!(reference.hash(), 7);
		assert_eq!(reference.size, 5);
		assert_eq!(reference.resource.bit_depth, BitDepths::TwentyFour);
		assert_eq!(reference.resource.channel_count, 2);
		assert_eq!(reference.resource.sample_rate, 48_000);
		assert_eq!(reference.resource.sample_count, 9_600);
		assert_eq!(reference.resource.get_class(), "Audio");
	}
}
