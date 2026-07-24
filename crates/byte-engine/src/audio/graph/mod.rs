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
const INLINE_ROUND_ROBIN_INPUT_CAPACITY: usize = 4;
pub(crate) const MAX_AUDIO_GRAPH_NODES: usize = 64;
pub(crate) type AudioProcessors = SmallVec<[AudioProcessor; INLINE_AUDIO_NODE_CAPACITY]>;
pub(crate) type RuntimeAudioProcessors = SmallVec<[SmallBox<dyn RuntimeAudioProcessor + Send, S4>; INLINE_AUDIO_NODE_CAPACITY]>;
type SelectedRoundRobins = SmallVec<[AudioNodeId; MAX_AUDIO_GRAPH_NODES]>;

/// The `AudioNodeId` struct identifies one node inside an [`AudioGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AudioNodeId(usize);

/// The `AudioGraph` struct describes resource-backed sources and the nodes that
/// select and process one source for the default audio output.
///
/// Build it with [`fns::sample`], [`fns::round_robin`], `loop`, [`fns::gain`],
/// [`fns::varispeed`], and [`fns::pitch_shift`]. Next, submit the same mutable
/// graph again through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`] to advance
/// its round-robin nodes. Stop its current play through
/// [`crate::gameplay::world::DefaultWorld::delete_channel_mut`].
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

	/// Combines independent input graphs under one stateful selector node.
	fn round_robin(inputs: impl IntoIterator<Item = AudioGraph>) -> Self {
		let mut nodes = SmallVec::new();
		let mut input_ids = SmallVec::new();

		for input in inputs {
			assert!(
				nodes.len() + input.nodes.len() < MAX_AUDIO_GRAPH_NODES,
				"Audio graph is too large. Combining the round-robin inputs would exceed {MAX_AUDIO_GRAPH_NODES} nodes."
			);
			let offset = nodes.len();
			input_ids.push(AudioNodeId(input.output.0 + offset));
			for mut node in input.nodes {
				node.remap_inputs(offset);
				nodes.push(node);
			}
		}

		assert!(
			!input_ids.is_empty(),
			"Invalid audio round-robin node. No input chains were provided."
		);
		let output = AudioNodeId(nodes.len());
		nodes.push(SmallBox::new(AudioNode::RoundRobin(Box::new(RoundRobinNode {
			inputs: input_ids,
			next_index: 0,
		}))));
		Self { nodes, output }
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

	/// Validates the complete authored graph and compiles its current selection
	/// without advancing round-robin state.
	pub(crate) fn compile(&self) -> Result<CompiledAudioGraph, String> {
		self.compile_selection().map(|(compiled, _)| compiled)
	}

	/// Resolves the current path and returns the round-robin nodes that a
	/// successful factory submission must advance.
	fn compile_selection(&self) -> Result<(CompiledAudioGraph, SelectedRoundRobins), String> {
		if self.nodes.is_empty() || self.nodes.len() > MAX_AUDIO_GRAPH_NODES {
			return Err(format!(
				"Invalid audio graph. A graph must contain between 1 and {MAX_AUDIO_GRAPH_NODES} nodes."
			));
		}
		self.validate()?;

		let mut selected_round_robins = SelectedRoundRobins::new();
		let compiled = self.compile_selected(self.output, &mut selected_round_robins)?;
		Ok((compiled, selected_round_robins))
	}

	/// Commits selection state only after the compiled play has been published.
	fn advance_round_robins(&mut self, selected_round_robins: &SelectedRoundRobins) {
		for node_id in selected_round_robins {
			let AudioNode::RoundRobin(node) = &mut *self.nodes[node_id.0] else {
				unreachable!("validated round-robin selection referred to another node type");
			};
			node.next_index = (node.next_index + 1) % node.inputs.len();
		}
	}

	/// Validates every selectable path without changing round-robin state.
	fn validate(&self) -> Result<(), String> {
		if self.output.0 >= self.nodes.len() {
			return Err("Invalid audio graph output. The output refers to a node outside this graph.".to_string());
		}

		let mut cached = [None; MAX_AUDIO_GRAPH_NODES];
		let mut visiting = [false; MAX_AUDIO_GRAPH_NODES];
		self.validate_node(
			self.output,
			&mut cached[..self.nodes.len()],
			&mut visiting[..self.nodes.len()],
		)?;
		if cached[..self.nodes.len()].iter().any(Option::is_none) {
			return Err("Invalid audio graph. One or more authored nodes are not connected to the graph output.".to_string());
		}
		Ok(())
	}

	/// Validates one node and returns the path constraints inherited by nodes
	/// that consume it.
	fn validate_node(
		&self,
		node_id: AudioNodeId,
		cached: &mut [Option<NodeProperties>],
		visiting: &mut [bool],
	) -> Result<NodeProperties, String> {
		if node_id.0 >= self.nodes.len() {
			return Err("Invalid audio graph connection. A node refers to an input outside this graph.".to_string());
		}
		if let Some(properties) = cached[node_id.0] {
			return Ok(properties);
		}
		if visiting[node_id.0] {
			return Err("Invalid audio graph cycle. A node input eventually refers back to the same node.".to_string());
		}
		visiting[node_id.0] = true;

		let properties = match &*self.nodes[node_id.0] {
			AudioNode::Sample { resource_id } => {
				if resource_id.is_empty() {
					return Err("Invalid audio sample node. The sample resource ID is empty.".to_string());
				}
				NodeProperties::default()
			}
			AudioNode::RoundRobin(node) => {
				if node.inputs.is_empty() {
					return Err("Invalid audio round-robin node. No input chains were provided.".to_string());
				}
				let mut combined = NodeProperties::default();
				for input in &node.inputs {
					combined.include(self.validate_node(*input, cached, visiting)?);
				}
				combined
			}
			AudioNode::Loop { input } => self.validate_node(*input, cached, visiting)?,
			AudioNode::Gain { input, gain } => {
				if !gain.is_finite() || *gain < 0.0 {
					return Err("Invalid audio gain node. Its gain is not finite or is negative.".to_string());
				}
				self.validate_node(*input, cached, visiting)?
			}
			AudioNode::Varispeed { input, rate } => {
				if !rate.is_finite() || !(0.25..=4.0).contains(rate) {
					return Err("Invalid audio varispeed node. Its rate is not finite or is outside 0.25..=4.0.".to_string());
				}
				let mut inherited = self.validate_node(*input, cached, visiting)?;
				if inherited.has_varispeed {
					return Err(
						"Invalid audio graph. At least one selectable path contains more than one varispeed node.".to_string(),
					);
				}
				inherited.has_varispeed = true;
				inherited
			}
			AudioNode::PitchShift { input, ratio } => {
				if !ratio.is_finite() || !(0.5..=2.0).contains(ratio) {
					return Err("Invalid audio pitch-shift node. Its ratio is not finite or is outside 0.5..=2.0.".to_string());
				}
				let mut inherited = self.validate_node(*input, cached, visiting)?;
				if inherited.has_pitch_shift {
					return Err(
						"Invalid audio graph. At least one selectable path contains more than one pitch-shift node."
							.to_string(),
					);
				}
				inherited.has_pitch_shift = true;
				inherited
			}
		};

		visiting[node_id.0] = false;
		cached[node_id.0] = Some(properties);
		Ok(properties)
	}

	/// Compiles only the branch selected for this submission and records the
	/// selectors that must advance after compilation succeeds.
	fn compile_selected(
		&self,
		node_id: AudioNodeId,
		selected_round_robins: &mut SelectedRoundRobins,
	) -> Result<CompiledAudioGraph, String> {
		let node = self
			.nodes
			.get(node_id.0)
			.ok_or_else(|| "Invalid audio graph connection. A selected node is outside this graph.".to_string())?;

		match &**node {
			AudioNode::Sample { resource_id } => Ok(CompiledAudioGraph {
				resource_id: resource_id.clone(),
				playback_mode: SamplePlaybackMode::Once,
				playback_rate: PlaybackRate::UNITY,
				processors: SmallVec::new(),
			}),
			AudioNode::RoundRobin(node) => {
				let selected = node.inputs[node.next_index % node.inputs.len()];
				selected_round_robins.push(node_id);
				self.compile_selected(selected, selected_round_robins)
			}
			AudioNode::Loop { input } => {
				let mut compiled = self.compile_selected(*input, selected_round_robins)?;
				compiled.playback_mode = SamplePlaybackMode::Loop;
				Ok(compiled)
			}
			AudioNode::Gain { input, gain } => {
				let mut compiled = self.compile_selected(*input, selected_round_robins)?;
				compiled.processors.push(AudioProcessor::Gain(*gain));
				Ok(compiled)
			}
			AudioNode::Varispeed { input, rate } => {
				let mut compiled = self.compile_selected(*input, selected_round_robins)?;
				compiled.playback_rate = PlaybackRate::from_rate(*rate);
				Ok(compiled)
			}
			AudioNode::PitchShift { input, ratio } => {
				let mut compiled = self.compile_selected(*input, selected_round_robins)?;
				if *ratio != 1.0 {
					compiled.processors.push(AudioProcessor::PitchShift(*ratio));
				}
				Ok(compiled)
			}
		}
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
	/// This advances each round-robin node on the selected path. Keep the graph
	/// and submit it again to play its next selection.
	///
	/// Send the returned handle through
	/// [`crate::gameplay::world::DefaultWorld::delete_channel_mut`] to stop the
	/// graph.
	pub fn create(&mut self, graph: &mut AudioGraph) -> Handle {
		let (compiled, selected_round_robins) = compile_for_factory(graph);
		let handle = self.compiled_graphs.create(compiled);
		graph.advance_round_robins(&selected_round_robins);
		handle
	}

	/// Compiles and publishes a replacement with an existing lifecycle handle.
	///
	/// This advances each round-robin node on the selected path.
	///
	/// Use this to replace a graph while preserving the identity returned by
	/// [`Self::create`].
	pub fn derive(&mut self, handle: Handle, graph: &mut AudioGraph) {
		let (compiled, selected_round_robins) = compile_for_factory(graph);
		self.compiled_graphs.derive(handle, compiled);
		graph.advance_round_robins(&selected_round_robins);
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
fn compile_for_factory(graph: &AudioGraph) -> (CompiledAudioGraph, SelectedRoundRobins) {
	graph
		.compile_selection()
		.unwrap_or_else(|error| panic!("Audio graph was not created. The authored graph is invalid: {error}"))
}

/// Stores one authored node in the graph's inline node list.
#[derive(Debug, Clone, PartialEq)]
enum AudioNode {
	Sample { resource_id: String },
	RoundRobin(Box<RoundRobinNode>),
	Loop { input: AudioNodeId },
	Gain { input: AudioNodeId, gain: f32 },
	Varispeed { input: AudioNodeId, rate: f32 },
	PitchShift { input: AudioNodeId, ratio: f32 },
}

impl AudioNode {
	/// Moves every input connection by the offset assigned while graphs are
	/// merged under a round-robin node.
	fn remap_inputs(&mut self, offset: usize) {
		match self {
			Self::Sample { .. } => {}
			Self::RoundRobin(node) => {
				for input in &mut node.inputs {
					input.0 += offset;
				}
			}
			Self::Loop { input }
			| Self::Gain { input, .. }
			| Self::Varispeed { input, .. }
			| Self::PitchShift { input, .. } => input.0 += offset,
		}
	}
}

/// The `RoundRobinNode` struct keeps branch connections and per-instance
/// selection state for an authored round-robin node.
#[derive(Debug, Clone, PartialEq)]
struct RoundRobinNode {
	inputs: SmallVec<[AudioNodeId; INLINE_ROUND_ROBIN_INPUT_CAPACITY]>,
	next_index: usize,
}

/// The `NodeProperties` struct summarizes constraints present on any selectable
/// path that can reach one authored node.
#[derive(Debug, Clone, Copy, Default)]
struct NodeProperties {
	has_varispeed: bool,
	has_pitch_shift: bool,
}

impl NodeProperties {
	fn include(&mut self, other: Self) {
		self.has_varispeed |= other.has_varispeed;
		self.has_pitch_shift |= other.has_pitch_shift;
	}
}

/// The `CompiledAudioGraph` struct carries validated sample and processing
/// settings from graph creation to resource loading.
#[derive(Debug, Clone)]
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
		fns::{gain, pitch_shift, r#loop, round_robin, sample, varispeed},
		AudioGraph, AudioGraphFactory, AudioNode, AudioNodeId, AudioProcessor, CompiledAudioGraph, PlaybackRate,
		SamplePlaybackMode,
	};
	use crate::core::listener::Listener;

	fn compile_submission(graph: &mut AudioGraph) -> CompiledAudioGraph {
		let (compiled, selected_round_robins) = graph.compile_selection().expect("valid graph");
		graph.advance_round_robins(&selected_round_robins);
		compiled
	}

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
	fn factory_submissions_cycle_through_complete_input_chains() {
		let mut graph = gain(
			round_robin([
				gain(r#loop(pitch_shift(sample("audio/a.wav"), 0.5)), 0.5),
				varispeed(sample("audio/b.wav"), 1.5),
				pitch_shift(sample("audio/c.wav"), 2.0),
			]),
			0.25,
		);
		let mut factory = AudioGraphFactory::new();
		let mut listener = factory.listener();

		let first_handle = factory.create(&mut graph);
		let _second_handle = factory.create(&mut graph);
		let _third_handle = factory.create(&mut graph);
		let _fourth_handle = factory.create(&mut graph);
		factory.derive(first_handle, &mut graph);
		let first = listener.read().expect("first selection");
		let second = listener.read().expect("second selection");
		let third = listener.read().expect("third selection");
		let fourth = listener.read().expect("wrapped selection");
		let replacement = listener.read().expect("derived selection");

		assert_eq!(first.data().resource_id, "audio/a.wav");
		assert_eq!(first.data().playback_mode, SamplePlaybackMode::Loop);
		assert_eq!(
			&first.data().processors[..],
			&[
				AudioProcessor::PitchShift(0.5),
				AudioProcessor::Gain(0.5),
				AudioProcessor::Gain(0.25)
			]
		);
		assert_eq!(second.data().resource_id, "audio/b.wav");
		assert_eq!(second.data().playback_rate.numerator, 3);
		assert_eq!(second.data().playback_rate.denominator, 2);
		assert_eq!(&second.data().processors[..], &[AudioProcessor::Gain(0.25)]);
		assert_eq!(third.data().resource_id, "audio/c.wav");
		assert_eq!(
			&third.data().processors[..],
			&[AudioProcessor::PitchShift(2.0), AudioProcessor::Gain(0.25)]
		);
		assert_eq!(fourth.data().resource_id, "audio/a.wav");
		assert_eq!(replacement.handle(), &first_handle);
		assert_eq!(replacement.data().resource_id, "audio/b.wav");
	}

	#[test]
	fn nested_round_robins_advance_only_on_the_selected_path() {
		let mut graph = round_robin([
			round_robin([sample("audio/a.wav"), sample("audio/b.wav")]),
			sample("audio/c.wav"),
		]);

		let sequence = (0..6).map(|_| compile_submission(&mut graph).resource_id).collect::<Vec<_>>();

		assert_eq!(
			sequence,
			[
				"audio/a.wav",
				"audio/c.wav",
				"audio/b.wav",
				"audio/c.wav",
				"audio/a.wav",
				"audio/c.wav"
			]
		);
	}

	#[test]
	fn cloned_graphs_keep_independent_round_robin_cursors() {
		let mut original = round_robin([sample("audio/a.wav"), sample("audio/b.wav")]);
		assert_eq!(compile_submission(&mut original).resource_id, "audio/a.wav");
		let mut cloned = original.clone();

		assert_eq!(compile_submission(&mut original).resource_id, "audio/b.wav");
		assert_eq!(compile_submission(&mut original).resource_id, "audio/a.wav");
		assert_eq!(compile_submission(&mut cloned).resource_id, "audio/b.wav");
		assert_eq!(compile_submission(&mut cloned).resource_id, "audio/a.wav");
	}

	#[test]
	fn one_input_round_robin_selects_the_same_chain_each_time() {
		let mut graph = round_robin([gain(sample("audio/a.wav"), 0.5)]);

		for _ in 0..3 {
			let compiled = compile_submission(&mut graph);
			assert_eq!(compiled.resource_id, "audio/a.wav");
			assert_eq!(&compiled.processors[..], &[AudioProcessor::Gain(0.5)]);
		}
	}

	#[test]
	#[should_panic(expected = "No input chains were provided")]
	fn round_robin_rejects_empty_inputs() {
		let _ = round_robin([]);
	}

	#[test]
	fn round_robin_accepts_the_node_limit_and_rejects_larger_graphs() {
		let inputs = (0..63).map(|index| sample(format!("audio/{index}.wav")));
		let maximum = round_robin(inputs);
		assert_eq!(maximum.nodes.len(), 64);
		assert_eq!(maximum.compile().unwrap().resource_id, "audio/0.wav");

		let too_many = std::panic::catch_unwind(|| round_robin((0..64).map(|index| sample(format!("audio/{index}.wav")))));
		assert!(too_many.is_err());
	}

	#[test]
	fn invalid_unselected_branch_does_not_advance_the_cursor() {
		let mut graph = round_robin([sample("audio/a.wav"), sample("")]);
		assert!(graph.compile().unwrap_err().contains("resource ID is empty"));

		let AudioNode::Sample { resource_id } = &mut *graph.nodes[1] else {
			panic!("second input must remain a sample node");
		};
		*resource_id = "audio/b.wav".to_string();

		assert_eq!(compile_submission(&mut graph).resource_id, "audio/a.wav");
		assert_eq!(compile_submission(&mut graph).resource_id, "audio/b.wav");
	}

	#[test]
	fn compiler_rejects_cycles_and_disconnected_round_robin_inputs() {
		let mut cyclic = gain(sample("audio/a.wav"), 0.5);
		let AudioNode::Gain { input, .. } = &mut *cyclic.nodes[cyclic.output.0] else {
			panic!("graph output must be a gain node");
		};
		*input = cyclic.output;
		assert!(cyclic.compile().unwrap_err().contains("cycle"));

		let mut disconnected = round_robin([sample("audio/a.wav"), sample("audio/b.wav")]);
		let AudioNode::RoundRobin(node) = &mut *disconnected.nodes[disconnected.output.0] else {
			panic!("graph output must be a round-robin node");
		};
		node.inputs.pop();
		assert!(disconnected.compile().unwrap_err().contains("not connected"));

		let mut outside = gain(sample("audio/a.wav"), 0.5);
		let AudioNode::Gain { input, .. } = &mut *outside.nodes[outside.output.0] else {
			panic!("graph output must be a gain node");
		};
		*input = AudioNodeId(usize::MAX);
		assert!(outside.compile().unwrap_err().contains("outside this graph"));
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
