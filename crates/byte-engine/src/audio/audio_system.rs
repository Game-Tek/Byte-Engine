use std::{collections::HashMap, sync::Arc};

use ahi::{
	self,
	audio_hardware_interface::{AudioHardwareInterface, HardwareParameters, Streams},
	Device,
};
use resource_management::{
	resource::{resource_manager::ResourceManager, ReadTargets, ReadTargetsMut},
	resources::audio::Audio,
	types::BitDepths,
	Reference,
};
use utils::Box as Boxy;

use super::{
	sound::{self, Sound},
	synthesizer::Synthesizer,
};
use crate::{
	audio::{
		emitter::Emitter,
		generator::{Generator, PlaybackSettings, PlaybackState},
		round_robin::RoundRobin,
	},
	core::{listener::Listener, Entity, EntityHandle},
	space::Positionable,
};

/// The [`AudioSystem`] trait defines the playback boundary used by application
/// audio workers.
///
/// Use [`DefaultAudioSystem`] for hardware output. Alternative implementations
/// can target offline rendering or tests while preserving generator handling.
pub trait AudioSystem: Entity {
	/// Plays an audio asset.
	fn play<'a>(&'a mut self, audio_asset_url: &'a str) -> ();

	/// Renders audio until the audio system stops.
	fn render(&mut self) {
		while self.render_available() {}
	}

	/// Processes audio data and submits it to the audio hardware interface.
	fn render_available(&mut self) -> bool;
}

/// The [`DefaultAudioSystem`] struct mixes generators and submits samples to the
/// platform audio device.
///
/// It is normally created by
/// [`crate::application::graphics::setup_default_audio`] rather than directly.
pub struct DefaultAudioSystem {
	device: Device,
	audio_resources: HashMap<String, (Audio, Vec<i16>)>,
	sources: Vec<Source>,
	channels: HashMap<String, Channel>,
	params: HardwareParameters,
	mix_buffer: Vec<f32>,
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
			mix_buffer: Vec::new(),
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
		render_sources(&self.audio_resources, &self.sources, self.params.get_sample_rate(), buffer);
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

fn render_sources(
	_audio_resources: &HashMap<String, (Audio, Vec<i16>)>,
	sources: &[Source],
	sample_rate: u32,
	buffer: &mut [f32],
) {
	let settings = PlaybackSettings { sample_rate };

	for playing_sound in sources {
		let current_sample = playing_sound.current_sample;

		let state = PlaybackState { current_sample };
		let _ = playing_sound.generator.render(settings, state, buffer);
	}
}

impl Entity for DefaultAudioSystem {}

impl AudioSystem for DefaultAudioSystem {
	fn play<'a>(&'a mut self, audio_asset_url: &'a str) {
		// self.load_asset(audio_asset_url);

		// self.sources.push(Source { generator: Generator::File { audio_asset_url: audio_asset_url.to_string() }, current_sample: 0, gain: 1f32 });
	}

	fn render_available(&mut self) -> bool {
		let Self {
			device,
			audio_resources,
			sources,
			params,
			mix_buffer,
			..
		} = self;
		let sample_rate = params.get_sample_rate();

		let frames = device
			.play(|streams| {
				match streams {
					Streams::MonoFloat32(buffer) => {
						// Hardware is the same format as what we use for rendering
						render_sources(audio_resources, sources, sample_rate, buffer);
					}
					Streams::Mono16Bit(buffer) => {
						if mix_buffer.len() < buffer.len() {
							mix_buffer.resize(buffer.len(), 0.0);
						}

						let (mix_buffer, _) = mix_buffer.split_at_mut(buffer.len());
						mix_buffer.fill(0.0);
						render_sources(audio_resources, sources, sample_rate, mix_buffer);

						for (destination, sample) in buffer.iter_mut().zip(mix_buffer.iter()) {
							*destination = f32_to_i16(*sample);
						}
					}
					Streams::Stereo16Bit(buffer) => {
						if mix_buffer.len() < buffer.len() {
							mix_buffer.resize(buffer.len(), 0.0);
						}

						let (mix_buffer, _) = mix_buffer.split_at_mut(buffer.len());
						mix_buffer.fill(0.0);
						render_sources(audio_resources, sources, sample_rate, mix_buffer);

						for ((left, right), sample) in buffer.iter_mut().zip(mix_buffer.iter()) {
							let sample = f32_to_i16(*sample);
							*left = sample;
							*right = sample;
						}
					}
					Streams::StereoFloat32(buffer) => {
						if mix_buffer.len() < buffer.len() {
							mix_buffer.resize(buffer.len(), 0.0);
						}

						let (mix_buffer, _) = mix_buffer.split_at_mut(buffer.len());
						mix_buffer.fill(0.0);
						render_sources(audio_resources, sources, sample_rate, mix_buffer);

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

/// The `Channel` struct reserves state for one audio mix channel.
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

#[cfg(test)]
mod tests {
	use std::{
		collections::HashMap,
		sync::{Arc, Mutex},
	};

	use super::{f32_to_i16, i16_to_f32, render_sources, Source};
	use crate::audio::generator::{Generator, PlaybackSettings, PlaybackState};

	struct ConstantGenerator {
		value: f32,
		observed: Arc<Mutex<Vec<(u32, u32)>>>,
	}

	impl Generator for ConstantGenerator {
		fn render<'a>(&self, settings: PlaybackSettings, state: PlaybackState, buffer: &'a mut [f32]) -> Option<&'a [f32]> {
			self.observed
				.lock()
				.unwrap()
				.push((settings.sample_rate, state.current_sample));
			for sample in buffer.iter_mut() {
				*sample += self.value;
			}
			Some(buffer)
		}

		fn done(&self, _settings: PlaybackSettings, _state: PlaybackState) -> bool {
			false
		}
	}

	#[test]
	fn pcm_conversion_preserves_zero_endpoints_and_monotonic_order() {
		assert_eq!(i16_to_f32(i16::MIN), -1.0);
		assert_eq!(i16_to_f32(0), 0.0);
		assert!(i16_to_f32(i16::MAX) < 1.0);
		assert_eq!(f32_to_i16(-1.0), i16::MIN);
		assert_eq!(f32_to_i16(0.0), 0);
		assert_eq!(f32_to_i16(1.0), i16::MAX);

		let samples = [-1.0, -0.5, 0.0, 0.5, 1.0];
		for pair in samples.windows(2) {
			assert!(f32_to_i16(pair[0]) < f32_to_i16(pair[1]));
		}
	}

	#[test]
	fn render_sources_mixes_all_generators_and_forwards_timeline_state() {
		let observed = Arc::new(Mutex::new(Vec::new()));
		let sources = [
			Source {
				generator: Arc::new(ConstantGenerator {
					value: 0.25,
					observed: observed.clone(),
				}),
				current_sample: 128,
				gain: 1.0,
			},
			Source {
				generator: Arc::new(ConstantGenerator {
					value: -0.1,
					observed: observed.clone(),
				}),
				current_sample: 256,
				gain: 1.0,
			},
		];
		let mut buffer = [0.5; 4];

		render_sources(&HashMap::new(), &sources, 48_000, &mut buffer);
		assert_eq!(buffer, [0.65; 4]);
		assert_eq!(*observed.lock().unwrap(), [(48_000, 128), (48_000, 256)]);
	}
}
