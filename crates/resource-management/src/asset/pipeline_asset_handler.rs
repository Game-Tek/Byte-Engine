//! Pipeline asset baking.

use super::{
	asset_handler::{AssetHandler, BakeContext, LoadErrors},
	ResourceId,
};
use crate::{resources::pipeline::Pipeline, ProcessedAsset};

/// The `PipelineAssetHandler` struct exists to persist portable `.pipeline` descriptions.
pub struct PipelineAssetHandler;

impl AssetHandler for PipelineAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "pipeline"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (source, _, format) = context.resolve(id).await?;
		if format != "pipeline" {
			return Err(LoadErrors::UnsupportedType);
		}
		let pipeline: Pipeline = serde_json::from_slice(&source).map_err(|error| {
			log::error!(
				"Pipeline asset could not be parsed for '{}': {error}. The most likely cause is invalid pipeline JSON.",
				id.as_ref()
			);
			LoadErrors::FailedToProcess
		})?;
		context.store_primary(ProcessedAsset::new(id, pipeline), &[])
	}
}
