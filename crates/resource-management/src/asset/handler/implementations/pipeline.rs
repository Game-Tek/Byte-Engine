//! Pipeline asset baking.

/// The `PipelineAssetHandler` struct exists to persist portable `.pipeline` descriptions.
pub struct PipelineAssetHandler;

impl AssetHandler for PipelineAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "pipeline"
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
		let (source, _, format) = context.resolve(id).await.inspect_err(|_| {
			context.error(format_args!(
				"Pipeline asset '{}' could not be loaded. The most likely cause is that the application's assets/byte-engine link does not expose the engine asset directory. See {}.",
				id.as_ref(),
				crate::online_docs_url("develop/resource-management/baking-app-resources")
			));
		})?;

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

		match &pipeline.kind {
			PipelineKind::Compute { shader, .. } => {
				context.bake_dependency::<Shader>(shader).await?;
			}
			PipelineKind::Raster { shaders, .. } => {
				context.bake_dependencies::<Shader>(shaders, 8).await?;
			}
		}

		context.store_primary(ProcessedAsset::new(id, pipeline), &[]).await
	}
}

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	ProcessedAsset,
	resources::material::Shader,
	resources::pipeline::{Pipeline, PipelineKind},
};
