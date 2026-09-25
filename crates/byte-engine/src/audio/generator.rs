//! Pull-based procedural audio generation.
//!
//! Implement [`Generator`] for sources consumed by
//! [`crate::audio::audio_system::DefaultAudioSystem`]. Generators receive
//! playback settings and state so implementations can remain independent of the
//! audio device.

/// The [`Generator`] trait provides thread-safe procedural audio sources for the output mixer.
///
/// Implementors must also be [`Clone`], because the generator factory's
/// message channel hands each listener its own copy. Publish a generator with
/// [`crate::application::graphics::GraphicsApplication::generator_factory`].
pub trait Generator: GeneratorClone + Send + Sync {
	fn render<'a>(&self, settings: PlaybackSettings, state: PlaybackState, buffer: &'a mut [f32]) -> Option<&'a [f32]>;

	fn done(&self, settings: PlaybackSettings, state: PlaybackState) -> bool;
}

/// The [`GeneratorClone`] trait lets a boxed [`Generator`] be copied without knowing its concrete type.
///
/// It is implemented for every `Clone` generator, so implementors never write it by hand.
pub trait GeneratorClone {
	/// Copies this generator into a new single-owner box.
	fn clone_box(&self) -> Box<dyn Generator>;
}

impl<T: Generator + Clone + 'static> GeneratorClone for T {
	fn clone_box(&self) -> Box<dyn Generator> {
		Box::new(self.clone())
	}
}

impl Clone for Box<dyn Generator> {
	fn clone(&self) -> Self {
		self.clone_box()
	}
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
