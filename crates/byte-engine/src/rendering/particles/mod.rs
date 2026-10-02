//! GPU-simulated particle systems.
//!
//! A particle system is a `.particles` asset: a list of data modules that baking turns into a simulation kernel and
//! a draw pipeline. Particles live and move entirely on the GPU: the CPU only says how many to spawn and where, and
//! the GPU sizes its own simulation and draw with indirect commands. Install the runtime with
//! [`crate::application::graphics::setup_particles`], then create a [`ParticleEmitter`] with a
//! [`crate::gameplay::Transform`] through the world.

mod emitter;
mod loader;
mod manager;
mod shader_data;
#[cfg(test)]
mod tests;

pub use emitter::ParticleEmitter;
pub(crate) use loader::ParticleSystemLoader;
pub use manager::ParticleManager;
