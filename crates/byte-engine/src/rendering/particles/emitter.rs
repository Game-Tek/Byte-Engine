use crate::inspector::Inspectable;

/// The `ParticleEmitter` struct places a source of one particle system in the world, such as a grinder throwing
/// sparks or a chimney trailing smoke.
///
/// Create one through [`crate::gameplay::world::DefaultWorld::create`] together with a
/// [`crate::gameplay::Transform`]. The transform moves, turns, and scales the system's spawn modules, so a cone
/// fires along the transform's local `+Z` axis. Publish a new value with [`crate::core::factory::Factory::derive`]
/// on the same handle to change it, and delete the handle to stop it; particles already in flight finish their
/// lives.
///
/// ```
/// # use byte_engine::rendering::ParticleEmitter;
/// let grinder = ParticleEmitter::new("byte-engine/particles/sparks.particles");
/// let hit = ParticleEmitter { rate: Some(0.0), burst: Some(60), ..grinder.clone() };
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleEmitter {
	/// The ID of the `.particles` asset this emitter spawns.
	pub system: String,
	/// Particles spawned per second, or `None` for the system's `spawn.rate`.
	pub rate: Option<f32>,
	/// Particles spawned once each time this value is published, or `None` for the system's `spawn.burst`.
	pub burst: Option<u32>,
}

impl ParticleEmitter {
	/// Creates an emitter that spawns `system` at the rate and burst its asset sets.
	pub fn new(system: impl Into<String>) -> Self {
		Self {
			system: system.into(),
			rate: None,
			burst: None,
		}
	}
}

impl Inspectable for ParticleEmitter {
	fn as_string(&self) -> String {
		format!("{:?}", self)
	}
}
