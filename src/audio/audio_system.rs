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
	playing_audios: Vec<PlayingSound>,
	channels: HashMap<String, Channel>,
}

impl DefaultAudioSystem {
	pub fn new(resource_manager: EntityHandle<ResourceManager>) -> Self {
		let mut channels = HashMap::with_capacity(16);

		channels.insert("master".to_string(), Channel { samples: vec![0; 48000 / 60].into_boxed_slice(), gain: 0.25f32 });

		Self {
			resource_manager,
			ahi: Box::new(ahi::audio_hardware_interface::create_ahi()),
			audio_resources: HashMap::with_capacity(1024),
			playing_audios: Vec::with_capacity(32),
			channels,
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
			self.playing_audios.push(PlayingSound { audio_asset_url: audio_asset_url.to_string(), current_sample: 0 });
		}
	}

	fn render(&mut self) {
		let master_channel = self.channels.get_mut("master").expect("No master channel was found.");

		master_channel.samples.iter_mut().for_each(|sample| *sample = 0);

		// let non_master_channels = self.channels.iter_mut().filter(|(name, _)| name.as_str() != "master");

		{	
			let audio_buffer = master_channel.samples.as_mut();
			let channel_gain = master_channel.gain;

			for playing_sound in &self.playing_audios {
				let audio_asset_url = &playing_sound.audio_asset_url;
				let current_sample = &playing_sound.current_sample;

				let (audio_resource, audio_data) = self.audio_resources.get(audio_asset_url).unwrap();

				let audio_data = &audio_data[*current_sample as usize..];

				let audio_data = if audio_data.len() > audio_buffer.len() {
					&audio_data[..audio_buffer.len()]
				} else {
					audio_data
				};

				for (i, sample) in audio_data.iter().enumerate() {
					audio_buffer[i] += ((((*sample as f32) / (65535f32 / 2f32)) * channel_gain) * (65535f32 / 2f32)) as i16;
				}
			}
		}

		let audio_buffer = master_channel.samples.as_mut();

		self.ahi.play(&audio_buffer[..]);

		for playing_sound in &mut self.playing_audios {
			playing_sound.current_sample += audio_buffer.len() as u32;
		}

		self.playing_audios.retain(|playing_sound| playing_sound.current_sample < self.audio_resources.get(&playing_sound.audio_asset_url).unwrap().0.sample_count as u32);
	}
}

struct Channel {
	samples: Box<[i16]>,
	gain: f32,
}

struct PlayingSound {
	audio_asset_url: String,
	current_sample: u32,
}