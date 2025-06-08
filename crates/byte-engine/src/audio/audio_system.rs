use std::{collections::HashMap, f32::consts::PI};
use resource_management::{resource::{resource_manager::ResourceManager, ReadTargets, ReadTargetsMut}, resources::audio::Audio, types::BitDepths, Reference};

use crate::core::{entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle};
use ahi::{audio_hardware_interface::AudioHardwareInterface, self};

use super::{sound::{self, Sound}, synthesizer::Synthesizer};

pub trait AudioSystem: Entity {
	/// Plays an audio asset.
	fn play<'a>(&'a mut self, audio_asset_url: &'a str) -> ();

	/// Processes audio data and sends it to the audio hardware interface.
	fn render(&mut self);
}

pub struct DefaultAudioSystem {
	resource_manager: EntityHandle<ResourceManager>,
	ahi: Box<dyn AudioHardwareInterface>,
	audio_resources: HashMap<String, (Audio, Vec<i16>)>,
	playing_audios: Vec<PlayingSound>,
	channels: HashMap<String, Channel>,
}

impl DefaultAudioSystem {
	pub fn new(resource_manager: EntityHandle<ResourceManager>) -> Self {
		let mut channels = HashMap::with_capacity(16);

		channels.insert("master".to_string(), Channel { samples: vec![0; 48000 / 60].into_boxed_slice(), gain: 1f32 });

		Self {
			resource_manager,
			ahi: Box::new(ahi::audio_hardware_interface::create_ahi()),
			audio_resources: HashMap::with_capacity(1024),
			playing_audios: Vec::with_capacity(32),
			channels,
		}
	}

	pub fn new_as_system(resource_manager: EntityHandle<ResourceManager>) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self::new(resource_manager)).listen_to::<CreateEvent<Sound>>().listen_to::<CreateEvent<Synthesizer>>()
	}
}

impl Entity for DefaultAudioSystem {}

impl AudioSystem for DefaultAudioSystem {
	fn play<'a>(&'a mut self, audio_asset_url: &'a str) {
		let data = if let Some(a) = self.audio_resources.get(audio_asset_url) {
			Some(a)
		} else {
			let resource_manager = self.resource_manager.read();
			let mut audio_resource_reference: Reference<Audio> = resource_manager.request(audio_asset_url).unwrap();
			let load_target = audio_resource_reference.load(ReadTargetsMut::create_buffer(&audio_resource_reference)).unwrap(); // Request resource be written into a managed buffer.

			let audio_resource = audio_resource_reference.resource_mut();

			let bytes = match load_target.buffer() {
				Some(b) => {
					match audio_resource.bit_depth {
						BitDepths::Eight => {
							if b.len() % 1 != 0 {
								return; // Invalid length for 8-bit audio.
							}

							b.iter().map(|&byte| (byte as i8) as i16 * 256).collect::<Vec<_>>()
						},
						BitDepths::Sixteen => {
							if b.len() % 2 != 0 {
								return; // Invalid length for 16-bit audio.
							}

							b.chunks_exact(2).map(|chunk| {
								let mut bytes = [0; 2];
								bytes.copy_from_slice(chunk);
								i16::from_le_bytes(bytes)
							}).collect::<Vec<_>>()
						},
						_ => {
							return; // Unsupported bit depth.
						}
					}
				},
				None => return,
			};

			self.audio_resources.insert(audio_asset_url.to_string(), (*audio_resource, bytes));

			Some(self.audio_resources.get(audio_asset_url).unwrap())
		};

		if let Some(_) = data {
			self.playing_audios.push(PlayingSound { source: Sources::File { audio_asset_url: audio_asset_url.to_string() }, current_sample: 0, gain: 1f32 });
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
				let gain = channel_gain * playing_sound.gain;

				match &playing_sound.source {
					Sources::File { audio_asset_url } => {
						let current_sample = &playing_sound.current_sample;
		
						let (_, audio_data) = self.audio_resources.get(audio_asset_url).unwrap();
		
						let audio_data = &audio_data[*current_sample as usize..];
		
						let audio_data = if audio_data.len() > audio_buffer.len() {
							&audio_data[..audio_buffer.len()]
						} else {
							audio_data
						};
		
						for (i, sample) in audio_data.iter().enumerate() {
							audio_buffer[i] += ((((*sample as f32) / (65535f32 / 2f32)) * gain) * (65535f32 / 2f32)) as i16;
						}
					}
					Sources::Synthesizer { pitch } => {
						let current_sample = playing_sound.current_sample;

						for (i, sample) in audio_buffer.iter_mut().enumerate() {
							let sample_index = (current_sample + i as u32) % 48000;
							let t = sample_index as f32 / 48000f32;
							*sample += ((((2f32 * PI * pitch * t)).sin() * gain) * (65535f32 / 2f32)) as i16;
						}
					}
				}
			}
		}

		let audio_buffer = master_channel.samples.as_mut();

		self.ahi.play(&audio_buffer[..]);

		for playing_sound in &mut self.playing_audios {
			playing_sound.current_sample += audio_buffer.len() as u32;
		}

		// self.playing_audios.retain(|playing_sound| playing_sound.current_sample < self.audio_resources.get(&playing_sound.audio_asset_url).unwrap().0.sample_count as u32);
	}
}

struct Channel {
	samples: Box<[i16]>,
	gain: f32,
}

enum Sources {
	Synthesizer {
		pitch: f32,
	},
	File {
		audio_asset_url: String,
	}
}

struct PlayingSound {
	source: Sources,
	current_sample: u32,
	gain: f32,
}

impl Listener<CreateEvent<Sound>> for DefaultAudioSystem {
	fn handle<'a>(&'a mut self, event: &CreateEvent<Sound>) -> () {
		let handle = event.handle();
		let sound = handle.read();
		self.play(&sound.asset);
	}
}

impl Listener<CreateEvent<Synthesizer>> for DefaultAudioSystem {
	fn handle<'a>(&'a mut self, handle: &CreateEvent<Synthesizer>) -> () {
		self.playing_audios.push(PlayingSound { source: Sources::Synthesizer { pitch: 110f32 }, current_sample: 0, gain: 0.10f32 });
		self.playing_audios.push(PlayingSound { source: Sources::Synthesizer { pitch: 440f32 }, current_sample: 0, gain: 0.10f32 });
		self.playing_audios.push(PlayingSound { source: Sources::Synthesizer { pitch: 554f32 }, current_sample: 0, gain: 0.10f32 });
		self.playing_audios.push(PlayingSound { source: Sources::Synthesizer { pitch: 659f32 }, current_sample: 0, gain: 0.10f32 });
		self.playing_audios.push(PlayingSound { source: Sources::Synthesizer { pitch: 830f32 }, current_sample: 0, gain: 0.10f32 });
	}
}