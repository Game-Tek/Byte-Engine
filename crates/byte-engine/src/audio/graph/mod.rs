//! Inline-authored audio processing graphs.
//!
//! Build a graph with the functions in [`fns`], then publish it through
//! [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`]. The
//! default audio worker validates and compiles each graph before its sample
//! resources cross to the audio thread.

use smallbox::{smallbox, space::S4, SmallBox};
use smallvec::SmallVec;

use crate::core::{
	factory::{CreateMessage, Factory, Handle},
	listener::DefaultListener,
	Entity,
};

pub mod fns;
mod pitch_shift;

const INLINE_AUDIO_NODE_CAPACITY: usize = 8;
pub(crate) const MAX_AUDIO_GRAPH_NODES: usize = INLINE_AUDIO_NODE_CAPACITY;
pub(crate) type AudioProcessors = SmallVec<[AudioProcessor; INLINE_AUDIO_NODE_CAPACITY]>;
pub(crate) type RuntimeAudioProcessors = SmallVec<[SmallBox<dyn RuntimeAudioProcessor + Send, S4>; INLINE_AUDIO_NODE_CAPACITY]>;

/// The `AudioNodeId` struct identifies one node inside an [`AudioGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AudioNodeId(usize);

/// The `AudioGraph` struct describes one source and the processing nodes that
/// route it to the default audio output.
///
/// The current graph format accepts one sample source and up to seven unary
/// processing nodes. Build it with [`fns::sample`], `loop`, [`fns::gain`],
/// [`fns::varispeed`], and [`fns::pitch_shift`].
/// Next,
/// publish it through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`].
#[must_use = "Audio graphs do not play until they are published through the world's audio graph factory"]
#[derive(Debug, Clone, PartialEq)]
pub struct AudioGraph {
	nodes: SmallVec<[SmallBox<AudioNode, S4>; INLINE_AUDIO_NODE_CAPACITY]>,
	output: AudioNodeId,
}

impl AudioGraph {
	/// Creates a graph whose output is one resource-backed sample node.
	fn sample(resource_id: impl Into<String>) -> Self {
		let mut nodes = SmallVec::new();
		nodes.push(SmallBox::new(AudioNode::Sample {
			resource_id: resource_id.into(),
		}));
		Self {
			nodes,
			output: AudioNodeId(0),
		}
	}

	/// Appends a loop node and makes it the graph output.
	fn looping(mut self) -> Self {
		let input = self.output;
		self.push(AudioNode::Loop { input });
		self
	}

	/// Appends a gain node and makes it the graph output.
	fn with_gain(mut self, gain: f32) -> Self {
		assert!(
			gain.is_finite() && gain >= 0.0,
			"Invalid audio graph gain. The gain must be finite and non-negative."
		);
		let input = self.output;
		self.push(AudioNode::Gain { input, gain });
		self
	}

	/// Appends a varispeed node that changes playback speed and pitch together.
	fn with_varispeed(mut self, rate: f32) -> Self {
		assert!(
			rate.is_finite() && (0.25..=4.0).contains(&rate),
			"Invalid audio graph varispeed rate. The provided rate is not finite or is outside 0.25..=4.0."
		);
		assert!(
			!self.nodes.iter().any(|node| matches!(&**node, AudioNode::Varispeed { .. })),
			"Invalid audio graph varispeed. A second varispeed node was added to the graph."
		);
		let input = self.output;
		self.push(AudioNode::Varispeed { input, rate });
		self
	}

	/// Appends a duration-preserving pitch-shift node.
	fn with_pitch_shift(mut self, ratio: f32) -> Self {
		assert!(
			ratio.is_finite() && (0.5..=2.0).contains(&ratio),
			"Invalid audio graph pitch ratio. The ratio must be finite and between 0.5 and 2.0."
		);
		assert!(
			!self.nodes.iter().any(|node| matches!(&**node, AudioNode::PitchShift { .. })),
			"Invalid audio graph pitch shift. A graph can contain at most one pitch-shift node."
		);
		let input = self.output;
		self.push(AudioNode::PitchShift { input, ratio });
		self
	}

	fn push(&mut self, node: AudioNode) {
		assert!(
			self.nodes.len() < MAX_AUDIO_GRAPH_NODES,
			"Audio graph is too large. A graph can contain at most {MAX_AUDIO_GRAPH_NODES} nodes."
		);
		self.output = AudioNodeId(self.nodes.len());
		self.nodes.push(SmallBox::new(node));
	}

	/// Compiles the authored chain into the state needed by the sample loader
	/// and audio worker.
	pub(crate) fn compile(self) -> Result<CompiledAudioGraph, String> {
		if self.nodes.is_empty() || self.nodes.len() > MAX_AUDIO_GRAPH_NODES {
			return Err(format!(
				"Invalid audio graph. A graph must contain between 1 and {MAX_AUDIO_GRAPH_NODES} nodes."
			));
		}

		let output = self.output;
		let mut current = None;
		let mut resource_id = None;
		let mut playback_mode = SamplePlaybackMode::Once;
		let mut playback_rate = PlaybackRate::UNITY;
		let mut processors = SmallVec::new();
		let mut has_varispeed = false;
		let mut has_pitch_shift = false;

		for (index, node) in self.nodes.into_iter().enumerate() {
			let node_id = AudioNodeId(index);
			match node.into_inner() {
				AudioNode::Sample {
					resource_id: sample_resource_id,
				} => {
					if current.is_some() {
						return Err(
							"Invalid audio graph. This graph implementation accepts one sample source followed by processing nodes."
								.to_string(),
						);
					}
					if sample_resource_id.is_empty() {
						return Err("Invalid audio sample node. The sample resource ID must not be empty.".to_string());
					}
					resource_id = Some(sample_resource_id);
				}
				AudioNode::Loop { input } => {
					validate_input(current, input)?;
					playback_mode = SamplePlaybackMode::Loop;
				}
				AudioNode::Gain { input, gain } => {
					validate_input(current, input)?;
					if !gain.is_finite() || gain < 0.0 {
						return Err("Invalid audio gain node. The gain must be finite and non-negative.".to_string());
					}
					processors.push(AudioProcessor::Gain(gain));
				}
				AudioNode::Varispeed { input, rate } => {
					validate_input(current, input)?;
					if !rate.is_finite() || !(0.25..=4.0).contains(&rate) {
						return Err(
							"Invalid audio varispeed node. Its rate is not finite or is outside 0.25..=4.0.".to_string(),
						);
					}
					if has_varispeed {
						return Err("Invalid audio graph. It contains more than one varispeed node.".to_string());
					}
					has_varispeed = true;
					playback_rate = PlaybackRate::from_rate(rate);
				}
				AudioNode::PitchShift { input, ratio } => {
					validate_input(current, input)?;
					if !ratio.is_finite() || !(0.5..=2.0).contains(&ratio) {
						return Err(
							"Invalid audio pitch-shift node. The ratio must be finite and between 0.5 and 2.0.".to_string(),
						);
					}
					if has_pitch_shift {
						return Err("Invalid audio graph. A graph can contain at most one pitch-shift node.".to_string());
					}
					has_pitch_shift = true;
					if ratio != 1.0 {
						processors.push(AudioProcessor::PitchShift(ratio));
					}
				}
			}
			current = Some(node_id);
		}

		if current != Some(output) {
			return Err("Invalid audio graph output. The output must refer to the final connected node.".to_string());
		}
		let resource_id =
			resource_id.ok_or_else(|| "Invalid audio graph. The graph must contain one sample source.".to_string())?;

		Ok(CompiledAudioGraph {
			resource_id,
			playback_mode,
			playback_rate,
			processors,
		})
	}
}

impl Entity for AudioGraph {}

/// The `AudioGraphFactory` struct validates authored graphs before publishing
/// their prepared resource and render plans.
///
/// Create graphs through [`Self::create`]. The default audio setup consumes the
/// prepared plans at hardware-period boundaries.
#[derive(Clone)]
pub struct AudioGraphFactory {
	compiled_graphs: Factory<CompiledAudioGraph>,
}

impl Default for AudioGraphFactory {
	fn default() -> Self {
		Self::new()
	}
}

impl AudioGraphFactory {
	/// Creates an empty graph factory.
	///
	/// Applications normally use
	/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`] instead.
	pub fn new() -> Self {
		Self {
			compiled_graphs: Factory::new(),
		}
	}

	/// Compiles and publishes a graph with a new lifecycle handle.
	///
	/// Send the returned handle through
	/// [`crate::gameplay::world::DefaultWorld::delete_channel_mut`] to stop the
	/// graph.
	pub fn create(&mut self, graph: AudioGraph) -> Handle {
		let compiled = compile_for_factory(graph);
		self.compiled_graphs.create(compiled)
	}

	/// Compiles and publishes a replacement with an existing lifecycle handle.
	///
	/// Use this to replace a graph while preserving the identity returned by
	/// [`Self::create`].
	pub fn derive(&mut self, handle: Handle, graph: AudioGraph) {
		let compiled = compile_for_factory(graph);
		self.compiled_graphs.derive(handle, compiled);
	}

	pub(crate) fn listener(&self) -> DefaultListener<CreateMessage<CompiledAudioGraph>> {
		self.compiled_graphs.listener()
	}

	pub(crate) fn drain_created_before_listener(&mut self) -> Vec<CreateMessage<CompiledAudioGraph>> {
		self.compiled_graphs.drain_created_before_listener()
	}
}

/// Compiles an authored graph on its creating thread before the factory sends
/// any work to the audio worker.
fn compile_for_factory(graph: AudioGraph) -> CompiledAudioGraph {
	graph
		.compile()
		.unwrap_or_else(|error| panic!("Audio graph was not created. The authored graph is invalid: {error}"))
}

fn validate_input(current: Option<AudioNodeId>, input: AudioNodeId) -> Result<(), String> {
	if current == Some(input) {
		Ok(())
	} else {
		Err("Invalid audio graph connection. Each processing node must consume the preceding node.".to_string())
	}
}

/// Stores one authored node in the graph's inline node list.
#[derive(Debug, Clone, PartialEq)]
enum AudioNode {
	Sample { resource_id: String },
	Loop { input: AudioNodeId },
	Gain { input: AudioNodeId, gain: f32 },
	Varispeed { input: AudioNodeId, rate: f32 },
	PitchShift { input: AudioNodeId, ratio: f32 },
}

/// The `CompiledAudioGraph` struct carries validated sample and processing
/// settings from graph creation to resource loading.
#[derive(Clone)]
pub(crate) struct CompiledAudioGraph {
	pub(crate) resource_id: String,
	pub(crate) playback_mode: SamplePlaybackMode,
	pub(crate) playback_rate: PlaybackRate,
	pub(crate) processors: AudioProcessors,
}

impl CompiledAudioGraph {
	/// Separates the resource request from the render plan retained while that
	/// resource loads.
	pub(crate) fn into_parts(self) -> (String, AudioGraphRenderPlan) {
		(
			self.resource_id,
			AudioGraphRenderPlan {
				playback_mode: self.playback_mode,
				playback_rate: self.playback_rate,
				processors: self.processors,
			},
		)
	}
}

/// The `AudioGraphRenderPlan` struct preserves validated playback and
/// processing state while the graph's sample resource loads.
#[derive(Debug)]
pub(crate) struct AudioGraphRenderPlan {
	pub(crate) playback_mode: SamplePlaybackMode,
	pub(crate) playback_rate: PlaybackRate,
	pub(crate) processors: AudioProcessors,
}

/// Selects what happens when the graph's sample reaches its final frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SamplePlaybackMode {
	Once,
	Loop,
}

/// The `PlaybackRate` struct keeps one authored varispeed rate as an exact
/// rational value for drift-free source-phase accumulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlaybackRate {
	pub(crate) numerator: u64,
	pub(crate) denominator: u64,
}

impl PlaybackRate {
	pub(crate) const UNITY: Self = Self {
		numerator: 1,
		denominator: 1,
	};

	/// Converts the exact binary value of a validated positive `f32` rate into
	/// its smallest power-of-two fraction.
	fn from_rate(rate: f32) -> Self {
		debug_assert!(rate.is_finite() && rate > 0.0);
		let bits = rate.to_bits();
		let significand = u64::from((bits & 0x7f_ffff) | 0x80_0000);
		let binary_exponent = ((bits >> 23) & 0xff) as i32 - 127 - 23;
		let (mut numerator, mut denominator) = if binary_exponent >= 0 {
			(significand << binary_exponent, 1)
		} else {
			(significand, 1_u64 << -binary_exponent)
		};
		let common_power_of_two = numerator.trailing_zeros().min(denominator.trailing_zeros());
		numerator >>= common_power_of_two;
		denominator >>= common_power_of_two;
		Self { numerator, denominator }
	}
}

/// Describes one allocation-free scalar processor in a compiled graph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum AudioProcessor {
	Gain(f32),
	PitchShift(f32),
}

impl AudioProcessor {
	/// Prepares this processor before it crosses to the audio worker.
	pub(crate) fn prepare(self) -> SmallBox<dyn RuntimeAudioProcessor + Send, S4> {
		match self {
			Self::Gain(gain) => smallbox!(GainProcessor(gain)),
			Self::PitchShift(ratio) => smallbox!(pitch_shift::PitchShiftProcessor::new(ratio)),
		}
	}
}

/// The `RuntimeAudioProcessor` trait provides allocation-free sample
/// processing after a graph has been prepared.
pub(crate) trait RuntimeAudioProcessor {
	fn process(&mut self, sample: f32) -> f32;
	fn latency(&self) -> usize;

	#[cfg(test)]
	fn gain_for_test(&self) -> Option<f32> {
		None
	}
}

/// The `GainProcessor` struct keeps one scalar multiplier inline in its
/// runtime node box.
struct GainProcessor(f32);

impl RuntimeAudioProcessor for GainProcessor {
	fn process(&mut self, sample: f32) -> f32 {
		sample * self.0
	}

	fn latency(&self) -> usize {
		0
	}

	#[cfg(test)]
	fn gain_for_test(&self) -> Option<f32> {
		Some(self.0)
	}
}

impl AudioGraphRenderPlan {
	/// Allocates stateful processors on the loader task before playback.
	pub(crate) fn prepare(self) -> PreparedAudioGraphRenderPlan {
		PreparedAudioGraphRenderPlan {
			playback_mode: self.playback_mode,
			playback_rate: self.playback_rate,
			processors: self.processors.into_iter().map(AudioProcessor::prepare).collect(),
		}
	}
}

/// The `PreparedAudioGraphRenderPlan` struct owns initialized processing state
/// ready to move to the audio worker.
pub(crate) struct PreparedAudioGraphRenderPlan {
	pub(crate) playback_mode: SamplePlaybackMode,
	pub(crate) playback_rate: PlaybackRate,
	pub(crate) processors: RuntimeAudioProcessors,
}

#[cfg(test)]
mod tests {
	use super::{
		fns::{gain, pitch_shift, r#loop, sample, varispeed},
		AudioProcessor, PlaybackRate, SamplePlaybackMode,
	};

	#[test]
	fn nested_functions_compile_to_a_looping_sample_and_gain_processor() {
		let graph = gain(r#loop(sample("audio/music.ogg")), 0.5);
		assert!(!graph.nodes.spilled());
		assert!(graph.nodes.iter().all(|node| !node.is_heap()));

		let compiled = graph.compile().expect("valid graph");
		assert_eq!(compiled.resource_id, "audio/music.ogg");
		assert_eq!(compiled.playback_mode, SamplePlaybackMode::Loop);
		assert!(!compiled.processors.spilled());
		assert_eq!(&compiled.processors[..], &[AudioProcessor::Gain(0.5)]);
	}

	#[test]
	fn pitch_shift_compiles_in_processor_order_and_unity_is_bypassed() {
		let compiled = gain(pitch_shift(sample("audio/music.ogg"), 2.0), 0.5)
			.compile()
			.expect("valid graph");
		assert_eq!(
			&compiled.processors[..],
			&[AudioProcessor::PitchShift(2.0), AudioProcessor::Gain(0.5)]
		);

		let unity = pitch_shift(sample("audio/music.ogg"), 1.0).compile().expect("valid graph");
		assert!(unity.processors.is_empty());
	}

	#[test]
	fn varispeed_compiles_as_an_exact_source_playback_rate() {
		let compiled = gain(varispeed(sample("audio/music.ogg"), 1.5), 0.5)
			.compile()
			.expect("valid graph");

		assert_eq!(
			compiled.playback_rate,
			PlaybackRate {
				numerator: 3,
				denominator: 2,
			}
		);
		assert_eq!(&compiled.processors[..], &[AudioProcessor::Gain(0.5)]);

		let unity = varispeed(sample("audio/music.ogg"), 1.0).compile().expect("valid graph");
		assert_eq!(unity.playback_rate, PlaybackRate::UNITY);
	}

	#[test]
	fn prepared_processors_keep_small_nodes_inline_and_large_state_node_local() {
		let compiled = gain(pitch_shift(sample("audio/music.ogg"), 2.0), 0.5)
			.compile()
			.expect("valid graph");
		let (_, render_plan) = compiled.into_parts();
		let prepared = render_plan.prepare();

		assert!(prepared.processors[0].is_heap());
		assert!(!prepared.processors[1].is_heap());
		assert!(!prepared.processors.spilled());
	}

	#[test]
	#[should_panic(expected = "Invalid audio graph pitch ratio")]
	fn pitch_shift_rejects_out_of_range_ratios_when_authored() {
		let _ = pitch_shift(sample("audio/music.ogg"), 2.1);
	}

	#[test]
	#[should_panic(expected = "at most one pitch-shift node")]
	fn graph_rejects_a_second_pitch_shift_when_authored() {
		let _ = pitch_shift(pitch_shift(sample("audio/music.ogg"), 0.5), 2.0);
	}

	#[test]
	#[should_panic(expected = "Invalid audio graph varispeed rate")]
	fn varispeed_rejects_out_of_range_rates_when_authored() {
		let _ = varispeed(sample("audio/music.ogg"), 4.1);
	}

	#[test]
	#[should_panic(expected = "A second varispeed node")]
	fn graph_rejects_a_second_varispeed_when_authored() {
		let _ = varispeed(varispeed(sample("audio/music.ogg"), 0.5), 2.0);
	}

	#[test]
	fn direct_sample_compiles_as_an_unprocessed_one_shot() {
		let compiled = sample("audio/impact.wav").compile().expect("valid graph");

		assert_eq!(compiled.resource_id, "audio/impact.wav");
		assert_eq!(compiled.playback_mode, SamplePlaybackMode::Once);
		assert_eq!(compiled.playback_rate, PlaybackRate::UNITY);
		assert!(compiled.processors.is_empty());
	}

	#[test]
	#[should_panic(expected = "Invalid audio graph gain")]
	fn gain_rejects_non_finite_values_when_the_graph_is_authored() {
		let _ = gain(sample("audio/music.ogg"), f32::NAN);
	}
}
