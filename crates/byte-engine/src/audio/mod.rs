//! Audio playback and procedural synthesis.
//!
//! Headed applications normally install [`audio_system::DefaultAudioSystem`]
//! through [`crate::application::graphics::setup_default_audio`]. Implement
//! [`generator::Generator`] for procedural or streamed audio and publish it through
//! [`crate::application::graphics::GraphicsApplication::generator_factory`].
//! Build an [`graph::AudioGraph`] and publish it through the default world's
//! audio graph factory when playback starts from a baked PCM resource.

#[doc(hidden)]
pub mod audio_system;

pub mod graph;
pub(crate) mod sample_loader;

#[doc(hidden)]
pub mod generator;

pub use audio_system::DefaultAudioSystem;
pub use generator::{Generator, PlaybackSettings, PlaybackState};
pub use sample_loader::{AudioSamplePoolConfig, DEFAULT_AUDIO_SAMPLE_POOL_BYTE_BUDGET};
