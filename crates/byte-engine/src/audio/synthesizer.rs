use crate::core::Entity;

/// The [`Synthesizer`] trait provides procedural samples for audio playback.
pub trait Synthesizer: Sync + Send {
	/// Renders the synthesizer output into the provided buffer.
	fn render<'a>(&self, current_sample: u64, buffer: &'a mut [f32]) -> &'a [f32];
}
