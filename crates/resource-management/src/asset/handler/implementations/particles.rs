//! Particle system asset baking.

mod generator;
mod schema;

use utils::Extent;

use super::{
	ResourceId,
	besl::{PlatformShaderCompilerAdapter, ShaderCompiler},
	handler::{AssetHandler, BakeContext, LoadErrors},
	store_model, store_model_owned,
};
use crate::{
	resources::{
		particle_system::ParticleSystem,
		pipeline::{Attachment, BlendMode, CullMode, FaceWinding, FillMode, Format, Pipeline, PipelineKind, PushConstantRange},
	},
	shader::ShaderGenerationSettings,
};

/// The `ParticleSystemAssetHandler` struct exists to bake `.particles` module lists into a runnable GPU particle system.
///
/// Baking `effects/sparks.particles` stores the [`ParticleSystem`] under that ID, its generated shaders under
/// `effects/sparks.particles#shaders/simulate`, `#shaders/vertex`, and `#shaders/fragment`, and the pipelines named by
/// [`ParticleSystem::simulate_pipeline`] and [`ParticleSystem::draw_pipeline`]. Requesting any of them bakes them all.
pub struct ParticleSystemAssetHandler {
	compiler: Box<dyn ShaderCompiler>,
}

impl Default for ParticleSystemAssetHandler {
	fn default() -> Self {
		Self::new()
	}
}

impl ParticleSystemAssetHandler {
	pub fn new() -> Self {
		Self {
			compiler: Box::new(PlatformShaderCompilerAdapter),
		}
	}
}

impl AssetHandler for ParticleSystemAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "particles"
	}

	/// Generates and compiles the system's kernels, then stores them, its pipelines, and the system itself.
	async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
		let container = id.get_base();
		let container = container.as_ref();
		let (source, format) = context.resolve(ResourceId::new(container)).await?;

		if format != "particles" {
			return Err(LoadErrors::UnsupportedType);
		}

		let system: schema::ParticleSystemSource = serde_json::from_slice(&source)
			.map_err(|error| error.to_string())
			.and_then(|system: schema::ParticleSystemSource| system.validate().map(|()| system))
			.map_err(|error| {
				context.error(format_args!(
					"Particle system '{container}' could not be read: {error} The most likely cause is an invalid `.particles` file."
				));
				LoadErrors::FailedToProcess
			})?;

		let shader = |stage: &str| format!("{container}#shaders/{stage}");
		for (stage, source, settings) in [
			(
				"simulate",
				generator::simulate_program(&system),
				ShaderGenerationSettings::compute(Extent::line(generator::SIMULATION_WORKGROUP_SIZE)),
			),
			(
				"vertex",
				generator::vertex_program(&system),
				ShaderGenerationSettings::vertex(),
			),
			(
				"fragment",
				generator::fragment_program(&system.render.shape),
				ShaderGenerationSettings::fragment(),
			),
		] {
			let id = shader(stage);
			let (compiled, bytes) = self
				.compiler
				.compile(&source, None, settings.name(id.clone()))
				.await
				.map_err(|error| {
					context.error(format_args!(
						"Generated particle {stage} shader for '{container}' failed to compile: {error} The most likely cause is a defect in the particle shader generator."
					));
					LoadErrors::FailedToProcess
				})?;
			store_model_owned(context, &id, compiled, bytes).await?;
		}

		let simulate_pipeline = format!("{container}#simulate");
		let draw_pipeline = format!("{container}#draw");
		for (id, kind) in [
			(
				&simulate_pipeline,
				PipelineKind::Compute {
					shader: shader("simulate"),
					push_constants: Vec::new(),
				},
			),
			(
				&draw_pipeline,
				PipelineKind::Raster {
					shaders: vec![shader("vertex"), shader("fragment")],
					push_constants: vec![PushConstantRange {
						offset: 0,
						size: generator::DRAW_PUSH_CONSTANT_SIZE,
					}],
					vertex_elements: Vec::new(),
					// Particles add their light to the scene color and test against scene depth without writing it.
					attachments: vec![
						Attachment {
							format: Format::Rg11b10Float,
							layer: None,
							blend: BlendMode::Premultiplied,
						},
						Attachment {
							format: Format::Depth32,
							layer: None,
							blend: BlendMode::None,
						},
					],
					face_winding: FaceWinding::default(),
					cull_mode: CullMode::None,
					fill_mode: FillMode::Solid,
					depth_write: false,
				},
			),
		] {
			store_model(context, id, Pipeline { name: id.clone(), kind }, &[]).await?;
		}

		let system = ParticleSystem {
			capacity: system.capacity,
			longest_life: generator::longest_life(&system),
			rate: system.spawn.rate,
			burst: system.spawn.burst,
			simulate_pipeline,
			draw_pipeline,
		};
		store_model(context, container, system, &[]).await.map(|_| ())
	}
}

#[cfg(test)]
mod tests;
