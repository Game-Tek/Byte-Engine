use crate::{orchestrator::{System, Entity, EntityReturn}, ahi::{audio_hardware_interface::AudioHardwareInterface, self}};

pub trait AudioSystem: System {
	/// Plays an audio asset.
	fn play(&self, audio_assest_url: &str);
}

pub struct DefaultAudioSystem {
	ahi: Box<dyn AudioHardwareInterface>,
}

impl DefaultAudioSystem {
	pub fn new() -> Self {
		Self {
			ahi: Box::new(ahi::audio_hardware_interface::create_ahi()),
		}
	}

	pub fn new_as_system() -> EntityReturn<'static, Self> {
		EntityReturn::new(Self::new())
	}
}

impl Entity for DefaultAudioSystem {}
impl System for DefaultAudioSystem {}

impl AudioSystem for DefaultAudioSystem {
	fn play(&self, audio_assest_url: &str) {
		self.ahi.play();
	}
}