//! Pull-based procedural audio generation.
//!
//! Implement [`Generator`] for sources consumed by
//! [`crate::audio::audio_system::DefaultAudioSystem`]. Generators receive
//! playback settings and state so implementations can remain independent of the
//! audio device.

/// The [`Generator`] trait defines a source that can fill an audio render buffer.
pub trait Generator {
	fn render<'a>(&self, settings: PlaybackSettings, state: PlaybackState, buffer: &'a mut [f32]) -> Option<&'a [f32]>;

	fn done(&self, settings: PlaybackSettings, state: PlaybackState) -> bool;
}

#[derive(Debug, Clone, Copy)]
/// The [`PlaybackSettings`] struct describes the output format relevant to a
/// generator.
pub struct PlaybackSettings {
	pub sample_rate: u32,
}

#[derive(Debug, Clone, Copy)]
/// The [`PlaybackState`] struct identifies the current position in a generator's
/// playback timeline.
pub struct PlaybackState {
	pub current_sample: u32,
}
