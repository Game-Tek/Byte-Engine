use std::{collections::HashMap, f32::consts::PI};
use resource_management::{resource::{resource_manager::ResourceManager, ReadTargets, ReadTargetsMut}, resources::audio::Audio, types::BitDepths, Reference};

use crate::{audio::{emitter::Emitter, round_robin::RoundRobin}, core::{entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle}, gameplay::Positionable};
use ahi::{self, audio_hardware_interface::{AudioDevice, AudioHardwareInterface, HardwareParameters, Streams, Writer}};

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

		let params = HardwareParameters::new().channels(1);

		let ahi = AudioDevice::new(params).expect("Failed to create audio device");

		channels.insert("master".to_string(), Channel { samples: vec![0; ahi.get_period_size() * 2].into_boxed_slice(), gain: 1f32 });

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

	fn rndr(&self, buffer: &mut [f32]) {
		for playing_sound in &self.sources {
			let current_sample = playing_sound.current_sample;
			let gain = playing_sound.gain;

			let mut play_sound = |url: &str| {
				let (audio, audio_data) = self.audio_resources.get(url).unwrap();

				if current_sample >= audio.sample_count { return; }

				let current_sample = current_sample.min(audio.sample_count);

				let audio_data = &audio_data[current_sample as usize..];

				for (b, s) in buffer.iter_mut().zip(audio_data.iter()) {
					*b = i16_to_f32(*s) * gain;
				}
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
}

impl Entity for DefaultAudioSystem {}

impl AudioSystem for DefaultAudioSystem {
	fn play<'a>(&'a mut self, audio_asset_url: &'a str) {
		self.load_asset(audio_asset_url);

		self.sources.push(Source { generator: Generator::File { audio_asset_url: audio_asset_url.to_string() }, current_sample: 0, gain: 1f32 });
	}

	fn render(&mut self) {
		let frames = self.ahi.get_period_size();

		let frames = self.ahi.play(|streams| {
			if let Streams::MonoFloat32(mut buffer) = streams { // Hardware is the same format as what we use for rendering
				self.rndr(&mut buffer);
			} else {
				let mut buffer = vec![0f32; streams.frames()].into_boxed_slice();

				self.rndr(&mut buffer);

				match streams {
					Streams::Mono16Bit(b) => {
						for (b, s) in b.iter_mut().zip(buffer.iter()) {
							*b = f32_to_i16(*s);
						}
					}
					Streams::Stereo16Bit(b) => {
						for ((dr, ds), s) in b.iter_mut().zip(buffer.iter()) {
							*dr = f32_to_i16(*s);
							*ds = f32_to_i16(*s);
						}
					}
					Streams::MonoFloat32(b) => {
						for (b, s) in b.iter_mut().zip(buffer.iter()) {
							*b = *s;
						}
					}
					Streams::StereoFloat32(b) => {
						for ((dr, ds), s) in b.iter_mut().zip(buffer.iter()) {
							*dr = *s;
							*ds = *s;
						}
					}
				}
			}
		}, |copier| {
			let mut buffer = vec![0f32; frames].into_boxed_slice();

			self.rndr(&mut buffer);

			match copier {
				Writer::Mono16Bit(c) => {
					let mut conversion_buffer = vec![0; frames].into_boxed_slice();

					buffer.iter().zip(conversion_buffer.iter_mut()).for_each(|(s, d)| {
						*d = f32_to_i16(*s);
					});

					c(&conversion_buffer);

					frames
				}
				Writer::Stereo16Bit(c) => {
					let mut conversion_buffer = vec![(0, 0); frames].into_boxed_slice();

					buffer.iter().zip(conversion_buffer.iter_mut()).for_each(|(s, d)| {
						*d = (f32_to_i16(*s), f32_to_i16(*s));
					});

					c(&conversion_buffer);

					frames
				}
				Writer::MonoFloat32(c) => {
					c(&buffer); // Harware requested format is same as our format

					frames
				}
				Writer::StereoFloat32(c) => {
					let mut conversion_buffer = vec![(0f32, 0f32); frames].into_boxed_slice();

					buffer.iter().zip(conversion_buffer.iter_mut()).for_each(|(s, d)| {
						*d = (*s, *s);
					});

					c(&conversion_buffer);

					frames
				}
			}
		}).unwrap();

		for e in &mut self.sources {
			e.current_sample += frames as u32;
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
