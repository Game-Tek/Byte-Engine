use std::collections::HashMap;
use crate::{orchestrator::{System, Entity, EntityReturn}, ahi::{audio_hardware_interface::AudioHardwareInterface, self}};
use crate::orchestrator::EntityHandle;
use crate::resource_manager::audio_resource_handler;
use crate::resource_manager::resource_manager::ResourceManager;

pub trait AudioSystem: System {
	/// Plays an audio asset.
	fn play(&mut self, audio_asset_url: &str);

	/// Processes audio data and sends it to the audio hardware interface.
	fn render(&mut self);
}

pub struct DefaultAudioSystem {
	resource_manager: EntityHandle<ResourceManager>,
	ahi: Box<dyn AudioHardwareInterface>,
	audio_resources: HashMap<String, (audio_resource_handler::Audio, Vec<i16>)>,
	playing_audios: Vec<(String, u32)>,
	audio_buffer: Box<[i16]>,
}

impl DefaultAudioSystem {
	pub fn new(resource_manager: EntityHandle<ResourceManager>) -> Self {
		Self {
			resource_manager,
			ahi: Box::new(ahi::audio_hardware_interface::create_ahi()),
			audio_resources: HashMap::with_capacity(1024),
			playing_audios: Vec::with_capacity(32),
			audio_buffer: vec![0; 48000 / 60].into_boxed_slice(),
		}
	}

	pub fn new_as_system(resource_manager: EntityHandle<ResourceManager>) -> EntityReturn<'static, Self> {
		EntityReturn::new(Self::new(resource_manager))
	}
}

impl Entity for DefaultAudioSystem {}
impl System for DefaultAudioSystem {}

impl AudioSystem for DefaultAudioSystem {
	fn play(&mut self, audio_asset_url: &str) {
		let data = if let Some(a) = self.audio_resources.get(audio_asset_url) {
			Some(a)
		} else {
			self.resource_manager.get(|resource_manager|{
				if let Some((response, bytes)) = resource_manager.get(audio_asset_url) {
					let audio_resource = response.resources[0].resource.downcast_ref::<audio_resource_handler::Audio>().unwrap();

					assert_eq!(audio_resource.bit_depth, audio_resource_handler::BitDepths::Sixteen);

					let audio_data = bytes.chunks_exact(2).map(|chunk| {
						let mut bytes = [0; 2];
						bytes.copy_from_slice(chunk);
						i16::from_le_bytes(bytes)
					}).collect::<Vec<_>>();

					self.audio_resources.insert(audio_asset_url.to_string(), (*audio_resource, audio_data));

					Some(self.audio_resources.get(audio_asset_url).unwrap())
				} else {
					log::warn!("Audio asset {} not found.", audio_asset_url);
					None
				}
			})
		};

		if let Some((_, audio_data)) = data {
			self.playing_audios.push((audio_asset_url.to_string(), 0));
		}
	}

	fn render(&mut self) {
		{	
			let audio_buffer = self.audio_buffer.as_mut();

			for (audio_asset_url, current_sample) in &self.playing_audios {
				let (audio_resource, audio_data) = self.audio_resources.get(audio_asset_url).unwrap();

				let audio_data = &audio_data[*current_sample as usize..];

				if audio_data.len() > audio_buffer.len() {
					audio_buffer.copy_from_slice(&audio_data[..audio_buffer.len()]);
				} else {
					audio_buffer[..audio_data.len()].copy_from_slice(audio_data);
				};
			}
		}

		let audio_buffer = &self.audio_buffer;

		self.ahi.play(&self.audio_buffer[..]);

		for (_, index) in &mut self.playing_audios {
			*index += audio_buffer.len() as u32;
		}

		self.playing_audios.retain(|(audio, index)| *index < self.audio_resources.get(audio).unwrap().0.sample_count as u32);
	}
}