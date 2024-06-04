use polodb_core::bson;
use serde::Deserialize;

use crate::{resource::resource_handler::ReadTargets, types::{Formats, Gamma}, LoadResults, Loader, Model, Reference, ReferenceModel, Resource, SolveErrors, Solver, StorageBackend};

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

impl <'a, 'de> Solver<'de, Reference<'a, Image>> for ReferenceModel<Image> {
	async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Reference<'a, Image>, SolveErrors> {
		let (gr, reader) = storage_backend.read(&self.id).await.ok_or_else(|| SolveErrors::StorageError)?;
		let Image { format, extent, gamma } = Image::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;

		Ok(Reference::new(&self.id, self.hash, gr.size, Image {
			format,
			extent,
			gamma,
		}, reader))
	}
}

impl <'a> Loader for Reference<'a, Image> {
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