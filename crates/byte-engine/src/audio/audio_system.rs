use std::{collections::HashMap, f32::consts::PI};
use resource_management::{resource::{resource_manager::ResourceManager, ReadTargets, ReadTargetsMut}, resources::audio::Audio, types::BitDepths, Reference};

use crate::{audio::{emitter::Emitter, round_robin::RoundRobin}, core::{entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle}, gameplay::Positionable};
use ahi::{self, audio_hardware_interface::{AudioDevice, AudioHardwareInterface, HardwareParameters}};

use super::{sound::{self, Sound}, synthesizer::Synthesizer};

/// The `AudioSystem` trait defines the interface for an audio system which handles audio playback and processing.
/// It provides methods for playing audio assets, processing audio data, and managing audio channels.
pub trait AudioSystem: Entity {
	/// Plays an audio asset.
	fn play<'a>(&'a mut self, audio_asset_url: &'a str) -> ();

	/// Processes audio data and sends it to the audio hardware interface.
	fn render(&mut self);
}

pub struct DefaultAudioSystem {
	resource_manager: EntityHandle<ResourceManager>,
	ahi: AudioDevice,
	audio_resources: HashMap<String, (Audio, Vec<i16>)>,
	sources: Vec<Source>,
	channels: HashMap<String, Channel>,
}

impl DefaultAudioSystem {
	pub fn new(resource_manager: EntityHandle<ResourceManager>) -> Self {
		let mut channels = HashMap::with_capacity(16);

		channels.insert("master".to_string(), Channel { samples: vec![0; 48000 / 60].into_boxed_slice(), gain: 1f32 });

		let params = HardwareParameters::new().channels(1);

		let ahi = AudioDevice::new(params).expect("Failed to create audio device");

		Self {
			resource_manager,
			ahi,
			audio_resources: HashMap::with_capacity(1024),
			sources: Vec::with_capacity(32),
			channels,
		}
	}

	pub fn new_as_system(resource_manager: EntityHandle<ResourceManager>) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self::new(resource_manager))
			.listen_to::<CreateEvent<Sound>>()
			.listen_to::<CreateEvent<Synthesizer>>()
			.listen_to::<CreateEvent<RoundRobin>>()
			.listen_to::<CreateEvent<Emitter>>()
	}

	fn load_asset<'a>(&'a mut self, audio_asset_url: &'a str) {
		if let Some(a) = self.audio_resources.get(audio_asset_url) {
			Some(a);
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

			Some(self.audio_resources.get(audio_asset_url).unwrap());
		}
	}
}

impl Entity for DefaultAudioSystem {}

impl AudioSystem for DefaultAudioSystem {
	fn play<'a>(&'a mut self, audio_asset_url: &'a str) {
		self.load_asset(audio_asset_url);

		self.sources.push(Source { generator: Generator::File { audio_asset_url: audio_asset_url.to_string() }, current_sample: 0, gain: 1f32 });
	}

	fn render(&mut self) {
		let master_channel = self.channels.get_mut("master").expect("No master channel was found.");

		master_channel.samples.iter_mut().for_each(|sample| *sample = 0);

		let hardware_period_size = self.ahi.get_period_size();

		let audio_buffer = master_channel.samples.as_mut();
		let audio_buffer = &mut audio_buffer[..hardware_period_size]; // Prepare audio buffer considering the AHI buffer size

		{
			let channel_gain = master_channel.gain;

			for playing_sound in &mut self.sources {
				let current_sample = &mut playing_sound.current_sample;
				let gain = channel_gain * playing_sound.gain;

				let mut play_sound = |url: &str| {
					let (audio, audio_data) = self.audio_resources.get(url).unwrap();

					let audio_data = &audio_data[*current_sample as usize..];

					let audio_data = if audio_data.len() > audio_buffer.len() {
						&audio_data[..audio_buffer.len()]
					} else {
						audio_data
					};

					for (i, sample) in audio_data.iter().enumerate() {
						audio_buffer[i] += f32_to_i16(i16_to_f32(*sample) * gain);
					}

					*current_sample += audio_data.len() as u32 % audio.sample_count;
				};

				match &playing_sound.generator {
					Generator::File { audio_asset_url } => {
						play_sound(audio_asset_url);
					}
					Generator::RoundRobin(handle) => {
						let mut rr = handle.write();
						if let Some(e) = rr.get() {
							play_sound(e);
						}
					}
					_ => {}
				}
			}
		}

		let frames = self.ahi.play(|| {
			audio_buffer
		}, |hw_buffer| {
			hw_buffer.copy_from_slice(audio_buffer);
		}).unwrap();

		if frames != hardware_period_size {
			log::warn!(" {} where written to hardware buffer but {} is the expected period size", frames, hardware_period_size);
		}
	}
}

/// The `Channel` struct represents a channel in the audio system.
struct Channel {
	samples: Box<[i16]>,
	gain: f32,
}

enum Generator {
	Synthesizer {
		pitch: f32,
	},
	File {
		audio_asset_url: String,
	},
	RoundRobin(EntityHandle<RoundRobin>),
}

struct Source {
	generator: Generator,
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
		self.sources.push(Source { generator: Generator::Synthesizer { pitch: 110f32 }, current_sample: 0, gain: 0.10f32 });
		self.sources.push(Source { generator: Generator::Synthesizer { pitch: 440f32 }, current_sample: 0, gain: 0.10f32 });
		self.sources.push(Source { generator: Generator::Synthesizer { pitch: 554f32 }, current_sample: 0, gain: 0.10f32 });
		self.sources.push(Source { generator: Generator::Synthesizer { pitch: 659f32 }, current_sample: 0, gain: 0.10f32 });
		self.sources.push(Source { generator: Generator::Synthesizer { pitch: 830f32 }, current_sample: 0, gain: 0.10f32 });
	}
}

impl Listener<CreateEvent<RoundRobin>> for DefaultAudioSystem {
	fn handle<'a>(&'a mut self, event: &CreateEvent<RoundRobin>) -> () {
		let handle = event.handle();

		let rr = handle.read();
		let sources = rr.get_assets();

		for source in sources {
			self.load_asset(&source);
		}
	}
}

impl Listener<CreateEvent<Emitter>> for DefaultAudioSystem {
	fn handle<'a>(&'a mut self, event: &CreateEvent<Emitter>) -> () {
		let handle = event.handle();

		{
			let emitter = handle.read();
			let position = emitter.get_position();
			let source = emitter.source();

			if let Some(rr) = source.downcast::<RoundRobin>() {
				{
					let rr = rr.read();
					let sources = rr.get_assets();

					for source in sources {
						self.load_asset(&source);
					}
				}

				self.sources.push(Source { generator: Generator::RoundRobin(rr), current_sample: 0, gain: 1.0f32 });
			}
		}
	}
}

fn i16_to_f32(sample: i16) -> f32 {
	sample as f32 / 32768.0
}

fn f32_to_i16(sample: f32) -> i16 {
	(sample * 32768.0) as i16
}
