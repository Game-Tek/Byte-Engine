//! Audio playback, synthesis, and spatial emitter support.
//!
//! Headed applications normally install [`audio_system::DefaultAudioSystem`]
//! through [`crate::application::graphics::setup_default_audio`]. Implement
//! [`generator::Generator`] for procedural or streamed audio, or
//! [`synthesizer::Synthesizer`] when producing samples from a synthesizer model.
//! [`emitter::Emitter`] connects generated sound to a position in the game world.

pub mod audio_system;

pub mod source;

pub mod round_robin;
pub mod sound;
pub mod synthesizer;

pub mod emitter;
pub mod generator;
