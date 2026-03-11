use resource_management::{
	resource::{resource_manager::ResourceManager, ReadTargets, ReadTargetsMut},
	resources::audio::Audio,
	types::BitDepths,
	Reference,
};
use smallvec::SmallVec;
use std::{collections::HashMap, sync::Arc};

use crate::{
	audio::{
		emitter::Emitter,
		generator::{Generator, PlaybackSettings, PlaybackState},
		round_robin::RoundRobin,
	},
	core::{listener::Listener, Entity, EntityHandle},
	space::Positionable,
};
use ahi::{
	self,
	audio_hardware_interface::{AudioHardwareInterface, HardwareParameters, Streams},
	Device,
};

use super::{
	sound::{self, Sound},
	synthesizer::Synthesizer,
};

use utils::Box as Boxy;

/// The `AudioSystem` trait defines the interface for an audio system which handles audio playback and processing.
/// It provides methods for playing audio assets, processing audio data, and managing audio channels.
pub trait AudioSystem: Entity {
	/// Plays an audio asset.
	fn play<'a>(&'a mut self, audio_asset_url: &'a str) -> ();

	/// Renders audio indefinitely.
	fn render(&mut self) {
		while self.render_available() {}
	}

	/// Processes audio data and sends it to the audio hardware interface.
	fn render_available(&mut self) -> bool;
}

pub struct DefaultAudioSystem {
	device: Device,
	audio_resources: HashMap<String, (Audio, Vec<i16>)>,
	sources: Vec<Source>,
	channels: HashMap<String, Channel>,
	params: HardwareParameters,
	last_reported_underrun_count: usize,
}

impl DefaultAudioSystem {
	pub fn try_new() -> Result<Self, &'static str> {
		let mut channels = HashMap::with_capacity(16);

		let params = HardwareParameters::new().channels(1);

		let device = Device::new(params).map_err(|e| {
			log::error!("Failed to create audio device: {}", e);
			"Failed to create audio device. Audio parameters may be invalid or device may not exist or be available."
		})?;

		channels.insert(
			"master".to_string(),
			Channel {
				samples: vec![0; device.get_period_size() * 2].into_boxed_slice(),
				gain: 1f32,
			},
		);

		Ok(Self {
			device,
			audio_resources: HashMap::with_capacity(1024),
			sources: Vec::with_capacity(64),
			channels,
			params,
			last_reported_underrun_count: 0,
		})
	}

	// fn load_asset<'a>(&'a mut self, audio_asset_url: &'a str) {
	// 	if let Some(a) = self.audio_resources.get(audio_asset_url) {
	// 		Some(a);
	// 	} else {
	// 		let resource_manager = self.resource_manager.read();
	// 		let mut audio_resource_reference: Reference<Audio> = resource_manager.request(audio_asset_url).unwrap();
	// 		let load_target = audio_resource_reference.load(ReadTargetsMut::create_buffer(&audio_resource_reference)).unwrap(); // Request resource be written into a managed buffer.

	// 		let audio_resource = audio_resource_reference.resource_mut();

	// 		let bytes = match load_target.buffer() {
	// 			Some(b) => {
	// 				match audio_resource.bit_depth {
	// 					BitDepths::Eight => {
	// 						if b.len() % 1 != 0 {
	// 							return; // Invalid length for 8-bit audio.
	// 						}

	// 						b.iter().map(|&byte| (byte as i8) as i16 * 256).collect::<Vec<_>>()
	// 					},
	// 					BitDepths::Sixteen => {
	// 						if b.len() % 2 != 0 {
	// 							return; // Invalid length for 16-bit audio.
	// 						}

	// 						b.chunks_exact(2).map(|chunk| {
	// 							let mut bytes = [0; 2];
	// 							bytes.copy_from_slice(chunk);
	// 							i16::from_le_bytes(bytes)
	// 						}).collect::<Vec<_>>()
	// 					},
	// 					_ => {
	// 						return; // Unsupported bit depth.
	// 					}
	// 				}
	// 			},
	// 			None => return,
	// 		};

	// 		self.audio_resources.insert(audio_asset_url.to_string(), (*audio_resource, bytes));

	// 		Some(self.audio_resources.get(audio_asset_url).unwrap());
	// 	}
	// }

	fn render_sources(&self, buffer: &mut [f32]) {
		let sample_rate = self.params.get_sample_rate();

		let mut to_destroy: SmallVec<[usize; 16]> = SmallVec::with_capacity(16);

		let settings = PlaybackSettings { sample_rate };

		for (idx, playing_sound) in self.sources.iter().enumerate() {
			let current_sample = playing_sound.current_sample;
			let gain = playing_sound.gain;

			let play_sound = |url: &str| {
				let (audio, audio_data) = self.audio_resources.get(url).unwrap();

				if current_sample >= audio.sample_count {
					return;
				}

				let current_sample = current_sample.min(audio.sample_count);

				let audio_data = &audio_data[current_sample as usize..];

				for (b, s) in buffer.iter_mut().zip(audio_data.iter()) {
					*b += i16_to_f32(*s) * gain;
				}
			};

			let state = PlaybackState { current_sample };

			if let None = playing_sound.generator.render(settings, state, buffer) {
				to_destroy.push(idx);
			}
		}
	}

	/// Reports newly observed underruns since the previous render call.
	fn report_new_underruns(&mut self) {
		let underrun_count = self.device.get_underrun_count();
		if underrun_count <= self.last_reported_underrun_count {
			return;
		}

		let new_underruns = underrun_count - self.last_reported_underrun_count;
		self.last_reported_underrun_count = underrun_count;

		log::warn!(
			"Audio underrun detected: {} new event(s), total {}",
			new_underruns,
			underrun_count
		);
	}

	pub fn create_generator(&mut self, generator: Arc<dyn Generator>) {
		let idx = self.sources.len();
		self.sources.push(Source {
			generator,
			current_sample: 0,
			gain: 1f32,
		});
	}
}

impl Entity for DefaultAudioSystem {}

impl AudioSystem for DefaultAudioSystem {
	fn play<'a>(&'a mut self, audio_asset_url: &'a str) {
		// self.load_asset(audio_asset_url);

		// self.sources.push(Source { generator: Generator::File { audio_asset_url: audio_asset_url.to_string() }, current_sample: 0, gain: 1f32 });
	}

	fn render_available(&mut self) -> bool {
		let device = &self.device;

		let frames = device
			.play(|streams| {
				match streams {
					Streams::MonoFloat32(buffer) => {
						// Hardware is the same format as what we use for rendering
						self.render_sources(buffer);
					}
					Streams::Mono16Bit(buffer) => {
						let mut mix_buffer = vec![0f32; buffer.len()].into_boxed_slice();
						self.render_sources(&mut mix_buffer);

						for (destination, sample) in buffer.iter_mut().zip(mix_buffer.iter()) {
							*destination = f32_to_i16(*sample);
						}
					}
					Streams::Stereo16Bit(buffer) => {
						let mut mix_buffer = vec![0f32; buffer.len()].into_boxed_slice();
						self.render_sources(&mut mix_buffer);

						for ((left, right), sample) in buffer.iter_mut().zip(mix_buffer.iter()) {
							let sample = f32_to_i16(*sample);
							*left = sample;
							*right = sample;
						}
					}
					Streams::StereoFloat32(buffer) => {
						let mut mix_buffer = vec![0f32; buffer.len()].into_boxed_slice();
						self.render_sources(&mut mix_buffer);

						for ((left, right), sample) in buffer.iter_mut().zip(mix_buffer.iter()) {
							*left = *sample;
							*right = *sample;
						}
					}
				}
			})
			.unwrap();

		self.report_new_underruns();

		if frames == 0 {
			return true;
		}

		for e in &mut self.sources {
			e.current_sample += frames as u32;
		}

		{
			self.sources.retain(|playing_sound| {
				let settings = PlaybackSettings {
					sample_rate: self.params.get_sample_rate(),
				};

				let state = PlaybackState {
					current_sample: playing_sound.current_sample,
				};

				!playing_sound.generator.done(settings, state)
			});
		}

		true
	}
}

/// The `Channel` struct represents a channel in the audio system.
struct Channel {
	samples: Box<[i16]>,
	gain: f32,
}

struct Source {
	generator: Arc<dyn Generator>,
	current_sample: u32,
	gain: f32,
}

fn i16_to_f32(sample: i16) -> f32 {
	sample as f32 / 32768.0
}

fn f32_to_i16(sample: f32) -> i16 {
	(sample * 32768.0) as i16
}
