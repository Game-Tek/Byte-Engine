//! Audio playback, synthesis, and spatial emitter support.
//!
//! Headed applications normally install [`audio_system::DefaultAudioSystem`]
//! through [`crate::application::graphics::setup_default_audio`]. Implement
//! [`generator::Generator`] for procedural or streamed audio, or
//! [`synthesizer::Synthesizer`] when producing samples from a synthesizer model.
//! [`emitter::Emitter`] connects generated sound to a position in the game world.

#[doc(hidden)]
pub mod audio_system;

#[doc(hidden)]
pub mod source;

#[doc(hidden)]
pub mod round_robin;
#[doc(hidden)]
pub mod sound;
#[doc(hidden)]
pub mod synthesizer;

#[doc(hidden)]
pub mod emitter;
#[doc(hidden)]
pub mod generator;

pub use audio_system::{AudioSystem, DefaultAudioSystem};
pub use emitter::Emitter;
pub use generator::{Generator, PlaybackSettings, PlaybackState};
pub use round_robin::RoundRobin;
pub use sound::Sound;
pub use source::Source;
pub use synthesizer::Synthesizer;
