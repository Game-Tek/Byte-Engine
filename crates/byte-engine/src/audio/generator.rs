pub trait Generator {
	fn render<'a>(&self, settings: PlaybackSettings, state: PlaybackState, buffer: &'a mut [f32]) -> Option<&'a [f32]>;

	fn done(&self, settings: PlaybackSettings, state: PlaybackState) -> bool;
}

#[derive(Debug, Clone, Copy)]
pub struct PlaybackSettings {
	pub sample_rate: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct PlaybackState {
	pub current_sample: u32,
}
