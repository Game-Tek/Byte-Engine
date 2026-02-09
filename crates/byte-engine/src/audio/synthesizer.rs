use crate::core::Entity;

pub trait Synthesizer: Sync + Send {
	/// Render the synthesizer's output into the provided buffer.
	fn render<'a>(&self, current_sample: u32, buffer: &'a mut [f32]) -> &'a [f32];
}
