use std::sync::Arc;

use ahi::{
	self,
	audio_hardware_interface::{AudioHardwareInterface, HardwareParameters, Streams},
	Device,
};

use super::{
	generator::{Generator, PlaybackSettings, PlaybackState},
	graph::{PlaybackRate, PreparedAudioGraphRenderPlan, RuntimeAudioProcessors, SamplePlaybackMode},
	sample_loader::{LoadedAudioSample, AUDIO_GRAPH_CAPACITY},
};
use crate::core::{factory::Handle, Entity};

/// The [`AudioSystem`] trait defines the playback boundary used by application
/// audio workers.
///
/// Use [`DefaultAudioSystem`] for hardware output. Alternative implementations
/// can target offline rendering or tests while preserving generator handling.
/// After construction, add generators or loaded audio sources, then call
/// [`Self::render_available`] from the audio worker until no period is ready.
pub trait AudioSystem: Entity {
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
/// After setup, publish a [`Generator`] through
/// [`crate::application::graphics::GraphicsApplication::generator_factory`] so
/// the worker can mix it into the hardware stream, or publish an
/// [`super::graph::AudioGraph`] through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
pub struct DefaultAudioSystem {
	device: Device,
	sources: Vec<Source>,
	audio_graphs: Vec<AudioGraphPlayer>,
	params: HardwareParameters,
	mix_buffer: Vec<f32>,
	last_reported_underrun_count: usize,
	sample_cache_prune_requested: bool,
}

impl DefaultAudioSystem {
	/// Opens the default audio device and preallocates its source and mix storage.
	///
	/// Applications normally call
	/// [`crate::application::graphics::setup_default_audio`] instead. Custom audio
	/// workers can add sources next and repeatedly call [`AudioSystem::render_available`].
	pub fn try_new() -> Result<Self, &'static str> {
		let params = HardwareParameters::new().channels(1);

		let device = Device::new(params).map_err(|e| {
			log::error!("Failed to create audio device: {}", e);
			"Failed to create audio device. Audio parameters may be invalid or device may not exist or be available."
		})?;
		let period_size = device.get_period_size();

		Ok(Self {
			device,
			sources: Vec::with_capacity(64),
			audio_graphs: Vec::with_capacity(AUDIO_GRAPH_CAPACITY),
			params,
			mix_buffer: vec![0.0; period_size],
			last_reported_underrun_count: 0,
			sample_cache_prune_requested: false,
		})
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
		self.sources.push(Source {
			generator,
			current_sample: 0,
		});
	}

	/// Adds a loaded graph without allocating audio-thread container storage.
	pub(crate) fn create_audio_graph(
		&mut self,
		handle: Handle,
		sample: Arc<LoadedAudioSample>,
		render_plan: PreparedAudioGraphRenderPlan,
	) {
		self.remove_audio_graph(handle);
		if self.audio_graphs.len() >= AUDIO_GRAPH_CAPACITY {
			log::warn!(
				"Audio graph was not created. The audio worker already has the maximum of {} active graphs.",
				AUDIO_GRAPH_CAPACITY
			);
			self.sample_cache_prune_requested = true;
			return;
		}
		self.audio_graphs.push(AudioGraphPlayer::new(handle, sample, render_plan));
	}

	/// Removes a resource-backed graph at the next period boundary.
	pub(crate) fn remove_audio_graph(&mut self, handle: Handle) {
		if let Some(index) = self.audio_graphs.iter().position(|graph| graph.handle == handle) {
			let graph = self.audio_graphs.swap_remove(index);
			drop(graph);
			self.sample_cache_prune_requested = true;
		}
	}

	pub(crate) fn audio_graph_count(&self) -> usize {
		self.audio_graphs.len()
	}

	pub(crate) fn take_sample_cache_prune_request(&mut self) -> bool {
		std::mem::take(&mut self.sample_cache_prune_requested)
	}
}

fn render_sources(sources: &[Source], sample_rate: u32, buffer: &mut [f32]) {
	let settings = PlaybackSettings { sample_rate };

	for playing_sound in sources {
		let current_sample = playing_sound.current_sample;

		let state = PlaybackState { current_sample };
		let _ = playing_sound.generator.render(settings, state, buffer);
	}
}

/// Mixes resource graphs after procedural generators so both source types use
/// the same output buffer and clipping boundary.
fn render_audio_graphs(audio_graphs: &mut [AudioGraphPlayer], sample_rate: u32, buffer: &mut [f32]) {
	for graph in audio_graphs {
		graph.render(sample_rate, buffer);
	}
}

impl Entity for DefaultAudioSystem {}

impl AudioSystem for DefaultAudioSystem {
	fn render_available(&mut self) -> bool {
		let Self {
			device,
			sources,
			audio_graphs,
			params,
			mix_buffer,
			..
		} = self;
		let sample_rate = params.get_sample_rate();

		let frames = match device.play(|streams| match streams {
			Streams::MonoFloat32(buffer) => {
				buffer.fill(0.0);
				render_sources(sources, sample_rate, buffer);
				render_audio_graphs(audio_graphs, sample_rate, buffer);
			}
			Streams::Mono16Bit(buffer) => {
				let (mix_buffer, _) = mix_buffer.split_at_mut(buffer.len());
				mix_buffer.fill(0.0);
				render_sources(sources, sample_rate, mix_buffer);
				render_audio_graphs(audio_graphs, sample_rate, mix_buffer);

				for (destination, sample) in buffer.iter_mut().zip(mix_buffer.iter()) {
					*destination = f32_to_i16(*sample);
				}
			}
			Streams::Stereo16Bit(buffer) => {
				let (mix_buffer, _) = mix_buffer.split_at_mut(buffer.len());
				mix_buffer.fill(0.0);
				render_sources(sources, sample_rate, mix_buffer);
				render_audio_graphs(audio_graphs, sample_rate, mix_buffer);

				for ((left, right), sample) in buffer.iter_mut().zip(mix_buffer.iter()) {
					let sample = f32_to_i16(*sample);
					*left = sample;
					*right = sample;
				}
			}
			Streams::StereoFloat32(buffer) => {
				let (mix_buffer, _) = mix_buffer.split_at_mut(buffer.len());
				mix_buffer.fill(0.0);
				render_sources(sources, sample_rate, mix_buffer);
				render_audio_graphs(audio_graphs, sample_rate, mix_buffer);

				for ((left, right), sample) in buffer.iter_mut().zip(mix_buffer.iter()) {
					*left = *sample;
					*right = *sample;
				}
			}
		}) {
			Ok(frames) => frames,
			Err(error) => {
				log::error!(
					"Audio playback stopped. The hardware device rejected a playback period: {}",
					error
				);
				return false;
			}
		};

		self.report_new_underruns();

		if frames == 0 {
			self.device.wait_for_playback_space();
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
		let audio_graph_count = self.audio_graphs.len();
		self.audio_graphs.retain(|graph| !graph.finished());
		self.sample_cache_prune_requested |= self.audio_graphs.len() != audio_graph_count;

		true
	}
}

/// The `Source` struct retains one procedural generator and its output
/// timeline.
struct Source {
	generator: Arc<dyn Generator>,
	current_sample: u32,
}

/// The `SampleNode` struct retains resampling state for one immutable loaded
/// sample within an audio graph.
struct SampleNode {
	sample: Arc<LoadedAudioSample>,
	playback_mode: SamplePlaybackMode,
	playback_rate: PlaybackRate,
	source_frame: u64,
	rate_phase: u64,
	finished: bool,
}

impl SampleNode {
	fn new(sample: Arc<LoadedAudioSample>, playback_mode: SamplePlaybackMode, playback_rate: PlaybackRate) -> Self {
		Self {
			sample,
			playback_mode,
			playback_rate,
			source_frame: 0,
			rate_phase: 0,
			finished: false,
		}
	}

	/// Produces one output sample with exact rational phase accumulation. Linear
	/// interpolation wraps at a loop boundary and clamps at a one-shot boundary.
	fn next(&mut self, output_sample_rate: u32) -> Option<f32> {
		if self.finished {
			return None;
		}

		let frame_count = self.sample.frame_count() as u64;
		let output_sample_rate = u64::from(output_sample_rate);
		let source_sample_rate = u64::from(self.sample.sample_rate());
		if self.source_frame >= frame_count {
			self.finished = true;
			return None;
		}

		let current_frame = self.source_frame as usize;
		let next_frame = if self.source_frame + 1 < frame_count {
			current_frame + 1
		} else if self.playback_mode == SamplePlaybackMode::Loop {
			0
		} else {
			current_frame
		};
		let current = self.sample.mono_frame(current_frame);
		let next = self.sample.mono_frame(next_frame);
		let phase_denominator = output_sample_rate * self.playback_rate.denominator;
		let fraction = self.rate_phase as f32 / phase_denominator as f32;
		let output = current + (next - current) * fraction;

		self.rate_phase += source_sample_rate * self.playback_rate.numerator;
		self.source_frame += self.rate_phase / phase_denominator;
		self.rate_phase %= phase_denominator;

		if self.playback_mode == SamplePlaybackMode::Loop {
			self.source_frame %= frame_count;
		} else if self.source_frame >= frame_count {
			self.finished = true;
		}

		Some(output)
	}

	/// Advances one output frame without reading sample data. Muted graphs use
	/// this path to preserve playback timing without doing unnecessary mixing.
	fn advance(&mut self, output_sample_rate: u32) -> bool {
		if self.finished {
			return false;
		}

		let frame_count = self.sample.frame_count() as u64;
		if self.source_frame >= frame_count {
			self.finished = true;
			return false;
		}

		let phase_denominator = u64::from(output_sample_rate) * self.playback_rate.denominator;
		self.rate_phase += u64::from(self.sample.sample_rate()) * self.playback_rate.numerator;
		self.source_frame += self.rate_phase / phase_denominator;
		self.rate_phase %= phase_denominator;

		if self.playback_mode == SamplePlaybackMode::Loop {
			self.source_frame %= frame_count;
		} else if self.source_frame >= frame_count {
			self.finished = true;
		}
		true
	}
}

/// The `AudioGraphPlayer` struct retains one loaded source and its compiled
/// processing chain for real-time playback.
struct AudioGraphPlayer {
	handle: Handle,
	sample: SampleNode,
	processors: RuntimeAudioProcessors,
	muted: bool,
	muted_drain_latency: usize,
	drain_remaining: Option<usize>,
}

impl AudioGraphPlayer {
	fn new(handle: Handle, sample: Arc<LoadedAudioSample>, render_plan: PreparedAudioGraphRenderPlan) -> Self {
		Self {
			handle,
			sample: SampleNode::new(sample, render_plan.playback_mode, render_plan.playback_rate),
			processors: render_plan.processors,
			muted: render_plan.muted,
			muted_drain_latency: render_plan.muted_drain_latency,
			drain_remaining: None,
		}
	}

	/// Renders one graph period by processing each source sample through the
	/// precompiled scalar node chain before mixing it into the destination.
	fn render(&mut self, output_sample_rate: u32, buffer: &mut [f32]) {
		if self.muted {
			for _ in buffer {
				if self.sample.advance(output_sample_rate) {
					continue;
				}
				let remaining = self.drain_remaining.get_or_insert(self.muted_drain_latency);
				if *remaining == 0 {
					break;
				}
				*remaining -= 1;
			}
			return;
		}

		for destination in buffer {
			let mut sample = match self.sample.next(output_sample_rate) {
				Some(sample) => sample,
				None => {
					let remaining = self
						.drain_remaining
						.get_or_insert_with(|| self.processors.iter().map(|processor| processor.latency()).sum());
					if *remaining == 0 {
						break;
					}
					*remaining -= 1;
					0.0
				}
			};
			for processor in &mut self.processors {
				sample = processor.process(sample);
			}
			*destination += sample;
		}
	}

	fn finished(&self) -> bool {
		if self.muted {
			return self.sample.finished && (self.muted_drain_latency == 0 || self.drain_remaining == Some(0));
		}
		self.sample.finished
			&& (self.drain_remaining == Some(0) || self.processors.iter().all(|processor| processor.latency() == 0))
	}
}

#[cfg(test)]
fn i16_to_f32(sample: i16) -> f32 {
	sample as f32 / 32768.0
}

fn f32_to_i16(sample: f32) -> i16 {
	(sample * 32768.0) as i16
}

#[cfg(test)]
mod tests {
	use std::sync::{Arc, Mutex};

	use super::{f32_to_i16, i16_to_f32, render_sources, AudioGraphPlayer, SampleNode, Source};
	use crate::{
		audio::{
			generator::{Generator, PlaybackSettings, PlaybackState},
			graph::{AudioGraphRenderPlan, AudioProcessor, PlaybackRate, SamplePlaybackMode},
			sample_loader::LoadedAudioSample,
		},
		core::{factory::Factory, listener::Listener},
	};

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
			},
			Source {
				generator: Arc::new(ConstantGenerator {
					value: -0.1,
					observed: observed.clone(),
				}),
				current_sample: 256,
			},
		];
		let mut buffer = [0.5; 4];

		render_sources(&sources, 48_000, &mut buffer);
		assert_eq!(buffer, [0.65; 4]);
		assert_eq!(*observed.lock().unwrap(), [(48_000, 128), (48_000, 256)]);
	}

	fn sample_node(samples: &[f32], source_rate: u32, playback_mode: SamplePlaybackMode) -> SampleNode {
		SampleNode::new(
			Arc::new(LoadedAudioSample::from_normalized_samples(
				source_rate,
				1,
				samples.to_vec().into_boxed_slice(),
			)),
			playback_mode,
			PlaybackRate::UNITY,
		)
	}

	fn render_sample_node(node: &mut SampleNode, output_sample_rate: u32, buffer: &mut [f32]) {
		for destination in buffer {
			let Some(sample) = node.next(output_sample_rate) else {
				break;
			};
			*destination += sample;
		}
	}

	fn graph_player(
		samples: &[f32],
		source_rate: u32,
		playback_mode: SamplePlaybackMode,
		playback_rate: PlaybackRate,
		processors: impl IntoIterator<Item = AudioProcessor>,
	) -> AudioGraphPlayer {
		let mut factory = Factory::new();
		let mut listener = factory.listener();
		let handle = factory.create(());
		let _ = listener.read();
		AudioGraphPlayer::new(
			handle,
			Arc::new(LoadedAudioSample::from_normalized_samples(
				source_rate,
				1,
				samples.to_vec().into_boxed_slice(),
			)),
			AudioGraphRenderPlan {
				playback_mode,
				playback_rate,
				processors: processors.into_iter().collect(),
				muted: false,
				muted_drain_latency: 0,
			}
			.prepare(),
		)
	}

	fn muted_graph_player(
		samples: &[f32],
		source_rate: u32,
		playback_mode: SamplePlaybackMode,
		playback_rate: PlaybackRate,
		drain_latency: usize,
	) -> AudioGraphPlayer {
		let mut player = graph_player(samples, source_rate, playback_mode, playback_rate, []);
		player.muted = true;
		player.muted_drain_latency = drain_latency;
		player
	}

	fn assert_samples_close(actual: &[f32], expected: &[f32]) {
		assert_eq!(actual.len(), expected.len());
		for (actual, expected) in actual.iter().zip(expected) {
			assert!((actual - expected).abs() < 0.000_01, "expected {expected}, got {actual}");
		}
	}

	#[test]
	fn looping_sample_continues_across_output_periods() {
		let mut player = sample_node(&[0.0, 1.0, 2.0], 48_000, SamplePlaybackMode::Loop);
		let mut first = [0.0; 2];
		let mut second = [0.0; 5];

		render_sample_node(&mut player, 48_000, &mut first);
		render_sample_node(&mut player, 48_000, &mut second);

		assert_eq!(first, [0.0, 1.0]);
		assert_eq!(second, [2.0, 0.0, 1.0, 2.0, 0.0]);
		assert!(!player.finished);
	}

	#[test]
	fn one_shot_clamps_its_last_resampled_frame_then_finishes() {
		let mut same_rate = sample_node(&[0.0, 1.0, 2.0], 48_000, SamplePlaybackMode::Once);
		let mut same_rate_output = [0.0; 5];
		render_sample_node(&mut same_rate, 48_000, &mut same_rate_output);
		assert_eq!(same_rate_output, [0.0, 1.0, 2.0, 0.0, 0.0]);
		assert!(same_rate.finished);

		let mut upsampled = sample_node(&[0.0, 1.0, 2.0], 2, SamplePlaybackMode::Once);
		let mut upsampled_output = [0.0; 6];
		render_sample_node(&mut upsampled, 4, &mut upsampled_output);
		assert_eq!(upsampled_output, [0.0, 0.5, 1.0, 1.5, 2.0, 2.0]);
		assert!(upsampled.finished);
	}

	#[test]
	fn downsampling_advances_over_intermediate_source_frames() {
		let mut player = sample_node(&[0.0, 1.0, 2.0, 3.0, 4.0], 4, SamplePlaybackMode::Once);
		let mut output = [0.0; 3];

		render_sample_node(&mut player, 2, &mut output);

		assert_eq!(output, [0.0, 2.0, 4.0]);
		assert!(player.finished);
	}

	#[test]
	fn rational_resampling_is_stable_across_period_boundaries() {
		let expected = [0.0, 6.666_666_5, 13.333_333, 20.0, 6.666_666_5, 3.333_333_3, 10.0, 16.666_666];
		let mut split = sample_node(&[0.0, 10.0, 20.0], 2, SamplePlaybackMode::Loop);
		let mut first = [0.0; 3];
		let mut second = [0.0; 5];
		render_sample_node(&mut split, 3, &mut first);
		render_sample_node(&mut split, 3, &mut second);

		let mut contiguous = sample_node(&[0.0, 10.0, 20.0], 2, SamplePlaybackMode::Loop);
		let mut whole = [0.0; 8];
		render_sample_node(&mut contiguous, 3, &mut whole);

		assert_samples_close(&first, &expected[..3]);
		assert_samples_close(&second, &expected[3..]);
		assert_samples_close(&whole, &expected);
		assert_eq!(split.source_frame, contiguous.source_frame);
		assert_eq!(split.rate_phase, contiguous.rate_phase);
	}

	#[test]
	fn rational_phase_has_no_long_term_44100_to_48000_drift() {
		let samples = vec![0.0; 50_000];
		let mut player = sample_node(&samples, 44_100, SamplePlaybackMode::Once);
		let mut output = vec![0.0; 48_000];

		render_sample_node(&mut player, 48_000, &mut output);

		assert_eq!(player.source_frame, 44_100);
		assert_eq!(player.rate_phase, 0);
		assert!(!player.finished);
	}

	#[test]
	fn varispeed_changes_playback_duration_and_sample_pitch_together() {
		let mut faster = graph_player(
			&[0.0, 1.0, 2.0, 3.0, 4.0],
			4,
			SamplePlaybackMode::Once,
			PlaybackRate {
				numerator: 2,
				denominator: 1,
			},
			[],
		);
		let mut faster_output = [0.0; 5];
		faster.render(4, &mut faster_output);
		assert_eq!(faster_output, [0.0, 2.0, 4.0, 0.0, 0.0]);
		assert!(faster.finished());

		let mut slower = graph_player(
			&[0.0, 1.0, 2.0],
			4,
			SamplePlaybackMode::Once,
			PlaybackRate {
				numerator: 1,
				denominator: 2,
			},
			[],
		);
		let mut slower_output = [0.0; 6];
		slower.render(4, &mut slower_output);
		assert_eq!(slower_output, [0.0, 0.5, 1.0, 1.5, 2.0, 2.0]);
		assert!(slower.finished());
	}

	#[test]
	fn muted_graph_advances_timing_without_touching_the_mix_buffer() {
		let mut player = muted_graph_player(
			&[1.0, 2.0, 3.0],
			4,
			SamplePlaybackMode::Once,
			PlaybackRate {
				numerator: 1,
				denominator: 2,
			},
			2,
		);
		let mut buffer = [0.25; 8];

		player.render(4, &mut buffer);

		assert_eq!(buffer, [0.25; 8]);
		assert!(player.sample.finished);
		assert_eq!(player.drain_remaining, Some(0));
		assert!(player.finished());
	}

	#[test]
	fn varispeed_phase_is_stable_across_output_periods() {
		let rate = PlaybackRate {
			numerator: 3,
			denominator: 2,
		};
		let mut split = graph_player(&[0.0, 10.0, 20.0], 2, SamplePlaybackMode::Loop, rate, []);
		let mut first = [0.0; 3];
		let mut second = [0.0; 5];
		split.render(3, &mut first);
		split.render(3, &mut second);

		let mut contiguous = graph_player(&[0.0, 10.0, 20.0], 2, SamplePlaybackMode::Loop, rate, []);
		let mut whole = [0.0; 8];
		contiguous.render(3, &mut whole);

		assert_samples_close(&first, &whole[..3]);
		assert_samples_close(&second, &whole[3..]);
		assert_eq!(split.sample.source_frame, contiguous.sample.source_frame);
		assert_eq!(split.sample.rate_phase, contiguous.sample.rate_phase);
	}

	#[test]
	fn gain_node_scales_a_looping_sample_after_the_source_node() {
		let mut graph = graph_player(
			&[0.0, 1.0, 2.0],
			48_000,
			SamplePlaybackMode::Loop,
			PlaybackRate::UNITY,
			[AudioProcessor::Gain(0.5)],
		);
		let mut first = [0.0; 2];
		let mut second = [0.0; 5];

		graph.render(48_000, &mut first);
		graph.render(48_000, &mut second);

		assert_eq!(first, [0.0, 0.5]);
		assert_eq!(second, [1.0, 0.0, 0.5, 1.0, 0.0]);
		assert!(!graph.finished());
	}

	#[test]
	fn sample_node_has_unity_output_without_a_gain_node() {
		let mut graph = graph_player(
			&[0.25, -0.5],
			48_000,
			SamplePlaybackMode::Once,
			PlaybackRate::UNITY,
			std::iter::empty(),
		);
		let mut output = [0.0; 2];

		graph.render(48_000, &mut output);

		assert_eq!(output, [0.25, -0.5]);
		assert!(graph.finished());
	}
}
