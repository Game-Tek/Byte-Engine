//! Pull-based procedural audio generation.
//!
//! Implement [`Generator`] for sources consumed by
//! [`crate::audio::audio_system::DefaultAudioSystem`]. Generators receive
//! playback settings and state so implementations can remain independent of the
//! audio device.

/// The [`Generator`] trait provides procedural audio sources for the output mixer.
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
/// The [`PlaybackState`] struct provides a generator's position in its playback
/// timeline.
pub struct PlaybackState {
	pub current_sample: u64,
}
