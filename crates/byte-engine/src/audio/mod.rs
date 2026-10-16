//! Audio playback and procedural synthesis.
//!
//! Headed applications normally install [`audio_system::DefaultAudioSystem`]
//! through [`crate::application::graphics::setup_default_audio`]. Implement
//! [`generator::Generator`] for procedural or streamed audio and publish it through
//! [`crate::application::graphics::GraphicsApplication::generator_factory`].
//! Build an [`graph::AudioGraph`] and publish it through the world scope's
//! audio graph factory when playback starts from a baked PCM resource.

// The graph runtime and the sample loader serve the audio system, which only headed builds have. Headless builds still
// compile and test them so the same graphs can be authored and posted there, but nothing plays them.
#![cfg_attr(not(feature = "headed"), allow(dead_code, unused_imports))]

#[doc(hidden)]
// Playing graphs needs the audio hardware interface, which only headed builds have. Authoring and posting
// them works everywhere, so servers can run the same gameplay code unheard.
#[cfg(feature = "headed")]
pub mod audio_system;

pub mod graph;
pub(crate) mod sample_loader;

#[doc(hidden)]
pub mod generator;

#[cfg(feature = "headed")]
pub use audio_system::DefaultAudioSystem;
pub use generator::{Generator, PlaybackSettings, PlaybackState};
pub use sample_loader::{AudioSamplePoolConfig, DEFAULT_AUDIO_SAMPLE_POOL_BYTE_BUDGET};
