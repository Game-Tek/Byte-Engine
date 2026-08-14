use std::sync::Arc;

use ahi::{
	self,
	audio_hardware_interface::{AudioHardwareInterface, HardwareParameters, Streams},
	Device,
};

use super::{
	generator::{Generator, PlaybackSettings, PlaybackState},
	graph::{AudioGraphTime, PlaybackRate, PreparedAudioGraphRenderPlan, RuntimeAudioProcessors, SamplePlaybackMode},
	sample_loader::{AudioSampleLease, AudioSampleLeaseId, AUDIO_GRAPH_CAPACITY, AUDIO_SAMPLE_RELEASE_CAPACITY},
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
	graph_buffer: Vec<f32>,
	last_reported_underrun_count: usize,
	released_sample_leases: Vec<AudioSampleLeaseId>,
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
			graph_buffer: vec![0.0; period_size],
			last_reported_underrun_count: 0,
			released_sample_leases: Vec::with_capacity(AUDIO_SAMPLE_RELEASE_CAPACITY),
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
		sample: AudioSampleLease,
		render_plan: PreparedAudioGraphRenderPlan,
	) {
		self.remove_audio_graph(handle);
		if self.audio_graphs.len() >= AUDIO_GRAPH_CAPACITY {
			log::warn!(
				"Audio graph was not created. The audio worker already has the maximum of {} active graphs.",
				AUDIO_GRAPH_CAPACITY
			);
			self.released_sample_leases.push(sample.into_id());
			return;
		}
		self.audio_graphs.push(AudioGraphPlayer::new(handle, sample, render_plan));
	}

	/// Removes a resource-backed graph at the next period boundary.
	pub(crate) fn remove_audio_graph(&mut self, handle: Handle) {
		if let Some(index) = self.audio_graphs.iter().position(|graph| graph.handle == handle) {
			let graph = self.audio_graphs.swap_remove(index);
			self.released_sample_leases.push(graph.into_sample_lease_id());
		}
	}

	pub(crate) fn audio_graph_count(&self) -> usize {
		self.audio_graphs.len()
	}

	/// Flushes returned lease IDs while retaining any that do not fit yet.
	pub(crate) fn flush_sample_lease_releases(&mut self, mut release: impl FnMut(AudioSampleLeaseId) -> bool) {
		while let Some(id) = self.released_sample_leases.last().copied() {
			if !release(id) {
				break;
			}
			self.released_sample_leases.pop();
		}
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

/// Advances generator timelines without wrapping if a process reaches the
/// maximum representable sample index.
fn advance_source_timelines(sources: &mut [Source], frames: usize) {
	let frames = frames as u64;
	for source in sources {
		source.current_sample = source.current_sample.saturating_add(frames);
	}
}

/// Mixes resource graphs after procedural generators so both source types use
/// the same output buffer and clipping boundary.
fn render_audio_graphs(audio_graphs: &mut [AudioGraphPlayer], sample_rate: u32, buffer: &mut [f32], graph_buffer: &mut [f32]) {
	debug_assert!(graph_buffer.len() >= buffer.len());
	for graph in audio_graphs {
		graph.render(sample_rate, buffer, &mut graph_buffer[..buffer.len()]);
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
			graph_buffer,
			..
		} = self;
		let sample_rate = params.get_sample_rate();

		let frames = match device.play(|streams| match streams {
			Streams::MonoFloat32(buffer) => {
				buffer.fill(0.0);
				render_sources(sources, sample_rate, buffer);
				render_audio_graphs(audio_graphs, sample_rate, buffer, graph_buffer);
			}
			Streams::Mono16Bit(buffer) => {
				let (mix_buffer, _) = mix_buffer.split_at_mut(buffer.len());
				mix_buffer.fill(0.0);
				render_sources(sources, sample_rate, mix_buffer);
				render_audio_graphs(audio_graphs, sample_rate, mix_buffer, graph_buffer);

				for (destination, sample) in buffer.iter_mut().zip(mix_buffer.iter()) {
					*destination = f32_to_i16(*sample);
				}
			}
			Streams::Stereo16Bit(buffer) => {
				let (mix_buffer, _) = mix_buffer.split_at_mut(buffer.len());
				mix_buffer.fill(0.0);
				render_sources(sources, sample_rate, mix_buffer);
				render_audio_graphs(audio_graphs, sample_rate, mix_buffer, graph_buffer);

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
				render_audio_graphs(audio_graphs, sample_rate, mix_buffer, graph_buffer);

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

		advance_source_timelines(&mut self.sources, frames);

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
		let mut index = 0;
		while index < self.audio_graphs.len() {
			if self.audio_graphs[index].finished() {
				let graph = self.audio_graphs.swap_remove(index);
				self.released_sample_leases.push(graph.into_sample_lease_id());
			} else {
				index += 1;
			}
		}

		true
	}
}

/// The `Source` struct retains one procedural generator and its output
/// timeline.
struct Source {
	generator: Arc<dyn Generator>,
	current_sample: u64,
}

/// The `SampleNode` struct retains resampling state for one immutable loaded
/// sample within an audio graph.
struct SampleNode {
	sample: AudioSampleLease,
	playback_mode: SamplePlaybackMode,
	playback_rate: PlaybackRate,
	source_frame: u64,
	rate_phase: u64,
	finished: bool,
}

impl SampleNode {
	fn new(sample: AudioSampleLease, playback_mode: SamplePlaybackMode, playback_rate: PlaybackRate) -> Self {
		Self {
			sample,
			playback_mode,
			playback_rate,
			source_frame: 0,
			rate_phase: 0,
			finished: false,
		}
	}

	/// Produces one output sample with exact rational phase accumulation.
	#[cfg(test)]
	fn next(&mut self, output_sample_rate: u32) -> Option<f32> {
		let mut output = 0.0;
		(self.process_block(output_sample_rate, 1, |_, sample| output = sample) == 1).then_some(output)
	}

	/// Writes a block of resampled source data and returns its valid length.
	fn render(&mut self, output_sample_rate: u32, buffer: &mut [f32]) -> usize {
		self.process_block(output_sample_rate, buffer.len(), |index, sample| buffer[index] = sample)
	}

	/// Mixes a block directly into its destination and returns the number of
	/// source samples produced.
	fn mix(&mut self, output_sample_rate: u32, buffer: &mut [f32]) -> usize {
		self.process_block(output_sample_rate, buffer.len(), |index, sample| buffer[index] += sample)
	}

	/// Scales and mixes a source block in one traversal.
	fn mix_scaled(&mut self, output_sample_rate: u32, buffer: &mut [f32], gain: f32) -> usize {
		self.process_block(output_sample_rate, buffer.len(), |index, sample| {
			buffer[index] += sample * gain
		})
	}

	/// Resamples one block while reusing rate constants for every output sample.
	/// Linear interpolation wraps loops and clamps one-shot boundaries.
	fn process_block(&mut self, output_sample_rate: u32, sample_count: usize, mut consume: impl FnMut(usize, f32)) -> usize {
		if self.finished {
			return 0;
		}

		let frame_count = self.sample.frame_count() as u64;
		let output_sample_rate = u64::from(output_sample_rate);
		let source_sample_rate = u64::from(self.sample.sample_rate());
		if self.source_frame >= frame_count {
			self.finished = true;
			return 0;
		}
		let phase_denominator = output_sample_rate * self.playback_rate.denominator;
		let phase_increment = source_sample_rate * self.playback_rate.numerator;
		if phase_increment == phase_denominator && self.rate_phase == 0 {
			return self.process_unity_block(sample_count, consume);
		}
		let mut rendered = 0;

		for index in 0..sample_count {
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
			let fraction = self.rate_phase as f32 / phase_denominator as f32;
			consume(index, current + (next - current) * fraction);
			rendered += 1;

			self.rate_phase += phase_increment;
			self.source_frame += self.rate_phase / phase_denominator;
			self.rate_phase %= phase_denominator;

			if self.playback_mode == SamplePlaybackMode::Loop {
				self.source_frame %= frame_count;
			} else if self.source_frame >= frame_count {
				self.finished = true;
				break;
			}
		}

		rendered
	}

	/// Reads exact-rate PCM in contiguous runs without interpolation or phase
	/// arithmetic. This is the normal path when an asset matches the device rate.
	fn process_unity_block(&mut self, sample_count: usize, mut consume: impl FnMut(usize, f32)) -> usize {
		let frame_count = self.sample.frame_count();
		let channel_count = usize::from(self.sample.channel_count());
		let samples = self.sample.samples();
		let mut source_frame = self.source_frame as usize;
		let mut rendered = 0;

		while rendered < sample_count {
			let run_length = (frame_count - source_frame).min(sample_count - rendered);
			if channel_count == 1 {
				for (offset, &sample) in samples[source_frame..source_frame + run_length].iter().enumerate() {
					consume(rendered + offset, sample);
				}
			} else {
				let start = source_frame * channel_count;
				let end = (source_frame + run_length) * channel_count;
				for (offset, frame) in samples[start..end].chunks_exact(2).enumerate() {
					consume(rendered + offset, (frame[0] + frame[1]) * 0.5);
				}
			}
			rendered += run_length;
			source_frame += run_length;

			if source_frame == frame_count {
				if self.playback_mode == SamplePlaybackMode::Loop {
					source_frame = 0;
				} else {
					self.finished = true;
					break;
				}
			}
		}

		self.source_frame = source_frame as u64;
		rendered
	}

	/// Advances a muted timeline for a complete output block without reading
	/// sample data or iterating over individual frames.
	fn advance_muted(&mut self, output_sample_rate: u32, sample_count: usize) -> usize {
		if self.finished || sample_count == 0 {
			return 0;
		}

		let frame_count = self.sample.frame_count() as u64;
		if self.source_frame >= frame_count {
			self.finished = true;
			return 0;
		}
		if self.playback_mode == SamplePlaybackMode::Loop {
			// A permanently muted loop has no observable source position and
			// remains alive until its lifecycle handle is deleted.
			return sample_count;
		}

		let phase_denominator = u128::from(output_sample_rate) * u128::from(self.playback_rate.denominator);
		let phase_increment = u128::from(self.sample.sample_rate()) * u128::from(self.playback_rate.numerator);
		let phase_to_end = u128::from(frame_count - self.source_frame) * phase_denominator - u128::from(self.rate_phase);
		let samples_to_end = phase_to_end.div_ceil(phase_increment);
		let advanced = u128::try_from(sample_count).unwrap().min(samples_to_end);
		let accumulated_phase = u128::from(self.rate_phase) + advanced * phase_increment;
		self.source_frame += u64::try_from(accumulated_phase / phase_denominator).unwrap();
		self.rate_phase = u64::try_from(accumulated_phase % phase_denominator).unwrap();
		if self.source_frame >= frame_count {
			self.finished = true;
		}
		usize::try_from(advanced).unwrap()
	}
}

/// The `AudioGraphPlayer` struct retains one loaded source and its compiled
/// processing chain for real-time playback.
struct AudioGraphPlayer {
	handle: Handle,
	sample: SampleNode,
	processors: RuntimeAudioProcessors,
	output_gain: f32,
	muted: bool,
	drain_latency: usize,
	drain_remaining: Option<usize>,
	rendered_sample_count: u64,
}

impl AudioGraphPlayer {
	fn new(handle: Handle, sample: AudioSampleLease, render_plan: PreparedAudioGraphRenderPlan) -> Self {
		Self {
			handle,
			sample: SampleNode::new(sample, render_plan.playback_mode, render_plan.playback_rate),
			processors: render_plan.processors,
			output_gain: render_plan.output_gain,
			muted: render_plan.muted,
			drain_latency: render_plan.drain_latency,
			drain_remaining: None,
			rendered_sample_count: 0,
		}
	}

	/// Renders one graph period into reusable scratch storage, processes the
	/// block through each compiled node, then mixes it into the destination.
	fn render(&mut self, output_sample_rate: u32, buffer: &mut [f32], graph_buffer: &mut [f32]) {
		debug_assert_eq!(buffer.len(), graph_buffer.len());
		if self.muted {
			let advanced = self.sample.advance_muted(output_sample_rate, buffer.len());
			if advanced < buffer.len() {
				let remaining = self.drain_remaining.get_or_insert(self.drain_latency);
				let drained = (*remaining).min(buffer.len() - advanced);
				*remaining -= drained;
			}
			return;
		}

		if self.processors.is_empty() {
			if self.output_gain == 1.0 {
				self.sample.mix(output_sample_rate, buffer);
			} else {
				self.sample.mix_scaled(output_sample_rate, buffer, self.output_gain);
			}
			return;
		}

		let source_sample_count = self.sample.render(output_sample_rate, graph_buffer);
		let mut rendered_sample_count = source_sample_count;
		if source_sample_count < graph_buffer.len() {
			let remaining = self.drain_remaining.get_or_insert(self.drain_latency);
			let drained = (*remaining).min(graph_buffer.len() - source_sample_count);
			graph_buffer[source_sample_count..source_sample_count + drained].fill(0.0);
			*remaining -= drained;
			rendered_sample_count += drained;
		}

		let rendered = &mut graph_buffer[..rendered_sample_count];
		let time = AudioGraphTime::new(self.rendered_sample_count, output_sample_rate);
		for processor in &mut self.processors {
			processor.process(time, rendered);
		}
		self.rendered_sample_count = self
			.rendered_sample_count
			.saturating_add(u64::try_from(rendered_sample_count).unwrap());
		if self.output_gain == 1.0 {
			for (destination, sample) in buffer.iter_mut().zip(rendered) {
				*destination += *sample;
			}
		} else {
			for (destination, sample) in buffer.iter_mut().zip(rendered) {
				*destination += *sample * self.output_gain;
			}
		}
	}

	fn finished(&self) -> bool {
		self.sample.finished && (self.drain_latency == 0 || self.drain_remaining == Some(0))
	}

	fn into_sample_lease_id(self) -> AudioSampleLeaseId {
		self.sample.sample.into_id()
	}
}

#[cfg(test)]
fn i16_to_f32(sample: i16) -> f32 {
	sample as f32 / 32768.0
}

fn f32_to_i16(sample: f32) -> i16 {
	(sample * 32768.0) as i16
}

/// Callback-only fixtures used by the external audio graph benchmark target.
#[doc(hidden)]
pub mod benchmarks {
	use super::AudioGraphPlayer;
	use crate::{
		audio::{
			graph::{AudioGraphRenderPlan, AudioProcessor, PlaybackRate, SamplePlaybackMode},
			sample_loader::AudioSampleLease,
		},
		core::{factory::Factory, listener::Listener},
	};

	pub const PERIOD_SIZE: usize = 256;

	/// The `AudioGraphBenchmark` enum selects one representative runtime graph.
	#[derive(Clone, Copy)]
	pub enum AudioGraphBenchmark {
		DirectUnity,
		Resample44100To48000,
		CustomProcessor,
		PitchShiftUp,
		PitchShiftDown,
	}

	/// The `AudioGraphBenchmarkState` struct owns one prepared graph and its
	/// reusable callback buffers.
	pub struct AudioGraphBenchmarkState {
		player: AudioGraphPlayer,
		_samples: Box<[f32]>,
		output: [f32; PERIOD_SIZE],
		graph_buffer: [f32; PERIOD_SIZE],
	}

	impl AudioGraphBenchmarkState {
		/// Prepares all graph state outside Divan's measured callback loop.
		pub fn new(benchmark: AudioGraphBenchmark) -> Self {
			let samples = (0..65_536)
				.map(|index| (index as f32 * 0.017).sin() * 0.25)
				.collect::<Vec<_>>()
				.into_boxed_slice();
			let (source_rate, render_plan) = match benchmark {
				AudioGraphBenchmark::DirectUnity => (48_000, render_plan([])),
				AudioGraphBenchmark::Resample44100To48000 => (44_100, render_plan([])),
				AudioGraphBenchmark::CustomProcessor => {
					let graph = crate::audio::graph::fns::custom(
						crate::audio::graph::fns::r#loop(crate::audio::graph::fns::sample("benchmark")),
						|_, samples| {
							for sample in samples {
								*sample = sample.mul_add(0.75, 0.01);
							}
						},
					);
					let (_, plan) = graph.compile().expect("valid benchmark graph").into_parts();
					(48_000, plan.prepare())
				}
				AudioGraphBenchmark::PitchShiftUp => (48_000, render_plan([AudioProcessor::PitchShift(1.5)])),
				AudioGraphBenchmark::PitchShiftDown => (48_000, render_plan([AudioProcessor::PitchShift(0.5)])),
			};
			let mut factory = Factory::new();
			let mut listener = factory.listener();
			let handle = factory.create(());
			let _ = listener.read();
			let sample = AudioSampleLease::for_benchmark(source_rate, 1, &samples);
			let mut state = Self {
				player: AudioGraphPlayer::new(handle, sample, render_plan),
				_samples: samples,
				output: [0.0; PERIOD_SIZE],
				graph_buffer: [0.0; PERIOD_SIZE],
			};
			// Populate cold processor and sample state before Divan measures it.
			state.render_period();
			state
		}

		/// Evaluates and mixes one 256-sample hardware-style period.
		pub fn render_period(&mut self) {
			self.player.render(48_000, &mut self.output, &mut self.graph_buffer);
		}
	}

	fn render_plan(processors: impl IntoIterator<Item = AudioProcessor>) -> crate::audio::graph::PreparedAudioGraphRenderPlan {
		AudioGraphRenderPlan {
			playback_mode: SamplePlaybackMode::Loop,
			playback_rate: PlaybackRate::UNITY,
			processors: processors.into_iter().collect(),
			muted: false,
			muted_drain_latency: 0,
		}
		.prepare()
	}
}

#[cfg(test)]
mod tests {
	use std::sync::{Arc, Mutex};

	use super::{advance_source_timelines, f32_to_i16, i16_to_f32, render_sources, AudioGraphPlayer, SampleNode, Source};
	use crate::{
		audio::{
			generator::{Generator, PlaybackSettings, PlaybackState},
			graph::{AudioGraphRenderPlan, AudioProcessor, PlaybackRate, SamplePlaybackMode},
			sample_loader::AudioSampleLease,
		},
		core::{factory::Factory, listener::Listener},
	};

	struct ConstantGenerator {
		value: f32,
		observed: Arc<Mutex<Vec<(u32, u64)>>>,
	}

	impl Generator for ConstantGenerator {
		fn render<'a>(&self, settings: PlaybackSettings, state: PlaybackState, buffer: &'a mut [f32]) -> Option<&'a [f32]> {
			self.observed
				.lock()
				.expect("expected test value")
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
		assert_eq!(*observed.lock().expect("expected test value"), [(48_000, 128), (48_000, 256)]);
	}

	#[test]
	fn generator_timeline_continues_past_u32_maximum() {
		let observed = Arc::new(Mutex::new(Vec::new()));
		let mut sources = [Source {
			generator: Arc::new(ConstantGenerator {
				value: 0.0,
				observed: observed.clone(),
			}),
			current_sample: u64::from(u32::MAX) - 1,
		}];
		let mut buffer = [0.0; 4];

		render_sources(&sources, 48_000, &mut buffer);
		advance_source_timelines(&mut sources, buffer.len());
		render_sources(&sources, 48_000, &mut buffer);

		assert_eq!(
			*observed.lock().expect("expected test value"),
			[(48_000, u64::from(u32::MAX) - 1), (48_000, u64::from(u32::MAX) + 3)]
		);
	}

	fn sample_node(samples: &[f32], source_rate: u32, playback_mode: SamplePlaybackMode) -> SampleNode {
		SampleNode::new(
			AudioSampleLease::for_test(source_rate, 1, samples.to_vec().into_boxed_slice()),
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
			AudioSampleLease::for_test(source_rate, 1, samples.to_vec().into_boxed_slice()),
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
		player.drain_latency = drain_latency;
		player
	}

	fn render_graph(player: &mut AudioGraphPlayer, output_sample_rate: u32, buffer: &mut [f32]) {
		let mut graph_buffer = vec![0.0; buffer.len()];
		player.render(output_sample_rate, buffer, &mut graph_buffer);
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
	fn unity_rate_stereo_uses_the_direct_path_and_downmixes_each_frame() {
		let mut player = SampleNode::new(
			AudioSampleLease::for_test(48_000, 2, vec![1.0, 3.0, -2.0, 2.0].into_boxed_slice()),
			SamplePlaybackMode::Once,
			PlaybackRate::UNITY,
		);
		let mut output = [0.0; 3];

		player.render(48_000, &mut output);

		assert_eq!(output, [2.0, 0.0, 0.0]);
		assert!(player.finished);
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
		render_graph(&mut faster, 4, &mut faster_output);
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
		render_graph(&mut slower, 4, &mut slower_output);
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

		render_graph(&mut player, 4, &mut buffer);

		assert_eq!(buffer, [0.25; 8]);
		assert!(player.sample.finished);
		assert_eq!(player.drain_remaining, Some(0));
		assert!(player.finished());
	}

	#[test]
	fn muted_one_shot_bulk_advance_matches_sample_by_sample_timing() {
		let rate = PlaybackRate {
			numerator: 3,
			denominator: 2,
		};
		let mut expected = SampleNode::new(
			AudioSampleLease::for_test(5, 1, vec![0.0; 7].into_boxed_slice()),
			SamplePlaybackMode::Once,
			rate,
		);
		let mut actual = SampleNode::new(
			AudioSampleLease::for_test(5, 1, vec![0.0; 7].into_boxed_slice()),
			SamplePlaybackMode::Once,
			rate,
		);

		let expected_count = (0..16).take_while(|_| expected.next(8).is_some()).count();
		let actual_count = actual.advance_muted(8, 16);

		assert_eq!(actual_count, expected_count);
		assert_eq!(actual.source_frame, expected.source_frame);
		assert_eq!(actual.rate_phase, expected.rate_phase);
		assert_eq!(actual.finished, expected.finished);
	}

	#[test]
	fn muted_loop_skips_source_timeline_work() {
		let mut player = muted_graph_player(&[1.0, 2.0, 3.0], 48_000, SamplePlaybackMode::Loop, PlaybackRate::UNITY, 0);
		let mut buffer = [0.25; 64];

		render_graph(&mut player, 48_000, &mut buffer);

		assert_eq!(buffer, [0.25; 64]);
		assert_eq!(player.sample.source_frame, 0);
		assert_eq!(player.sample.rate_phase, 0);
		assert!(!player.finished());
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
		render_graph(&mut split, 3, &mut first);
		render_graph(&mut split, 3, &mut second);

		let mut contiguous = graph_player(&[0.0, 10.0, 20.0], 2, SamplePlaybackMode::Loop, rate, []);
		let mut whole = [0.0; 8];
		render_graph(&mut contiguous, 3, &mut whole);

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

		render_graph(&mut graph, 48_000, &mut first);
		render_graph(&mut graph, 48_000, &mut second);

		assert_eq!(first, [0.0, 0.5]);
		assert_eq!(second, [1.0, 0.0, 0.5, 1.0, 0.0]);
		assert!(!graph.finished());
	}

	#[test]
	fn block_processing_stops_at_the_end_of_a_one_shot_source() {
		let mut graph = graph_player(
			&[1.0, 2.0],
			48_000,
			SamplePlaybackMode::Once,
			PlaybackRate::UNITY,
			[AudioProcessor::Gain(0.5)],
		);
		let mut output = [1.0; 4];

		render_graph(&mut graph, 48_000, &mut output);

		assert_eq!(output, [1.5, 2.0, 1.0, 1.0]);
		assert!(graph.finished());
	}

	#[test]
	fn block_processing_preserves_a_stateful_processor_tail() {
		let mut graph = graph_player(
			&[1.0, 2.0],
			48_000,
			SamplePlaybackMode::Once,
			PlaybackRate::UNITY,
			[AudioProcessor::PitchShift(2.0)],
		);
		let drain_latency = graph.drain_latency;
		let mut first = [0.0; 4];
		render_graph(&mut graph, 48_000, &mut first);
		assert_eq!(graph.drain_remaining, Some(drain_latency - 2));
		assert!(!graph.finished());

		let mut tail = vec![0.0; drain_latency - 2];
		render_graph(&mut graph, 48_000, &mut tail);
		assert_eq!(graph.drain_remaining, Some(0));
		assert!(graph.finished());
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

		render_graph(&mut graph, 48_000, &mut output);

		assert_eq!(output, [0.25, -0.5]);
		assert!(graph.finished());
	}
}
