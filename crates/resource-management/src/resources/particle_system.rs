//! Baked GPU particle systems.

/// The `ParticleSystem` struct lets the renderer run a particle system authored as a `.particles` asset.
///
/// Baking the asset also stores the system's generated shaders and the two pipelines named here, so the renderer
/// only sizes buffers from this record and requests the pipelines by ID. Request it with
/// [`crate::ResourceManager::request`], then request [`Self::simulate_pipeline`] and [`Self::draw_pipeline`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ParticleSystem {
	/// How many particles of this system can be alive at once.
	pub capacity: u32,
	/// The longest a particle can live, in seconds. An emitter's slot is reused only this long after its emitter is
	/// removed, once its last particle has died.
	pub longest_life: f32,
	/// Particles spawned per second by an emitter that does not set its own rate.
	pub rate: f32,
	/// Particles spawned once when an emitter that does not set its own burst is published.
	pub burst: u32,
	/// The ID of the compute pipeline that spawns, moves, and packs this system's particles.
	pub simulate_pipeline: String,
	/// The ID of the raster pipeline that draws this system's particles.
	pub draw_pipeline: String,
}

super::impl_direct_resource!(ParticleSystem, "ParticleSystem");
