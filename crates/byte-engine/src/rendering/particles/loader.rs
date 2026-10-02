use resource_management::resources::particle_system::ParticleSystem;

use crate::core::EntityHandle;
use crate::rendering::loading::{LoadError, LoadPipeline, LoaderLane};

/// The `ParticleSystemLoader` struct reads baked particle systems on the loading thread, so the render thread never
/// waits for a `.particles` asset to bake or load.
///
/// [`crate::application::graphics::setup_particles`] spawns it; the [`super::ParticleManager`] requests systems by ID
/// through its client.
pub(crate) struct ParticleSystemLoader {
	pub(crate) resources: EntityHandle<resource_management::ResourceManager>,
}

impl LoadPipeline for ParticleSystemLoader {
	type Key = String;
	type Request = String;
	type Resident = ParticleSystem;

	fn key(request: &Self::Request) -> Self::Key {
		request.clone()
	}

	/// Reads one system's record. Its pipelines compile separately, through the renderer's pipeline compiler.
	async fn load(&self, id: String, _lane: &mut LoaderLane<Self>) -> Result<ParticleSystem, LoadError> {
		self.resources
			.request::<ParticleSystem>(&id)
			.await
			.map(|system| system.resource().clone())
			.map_err(|error| LoadError(format!("Particle system '{id}' could not be loaded. {error}")))
	}
}
