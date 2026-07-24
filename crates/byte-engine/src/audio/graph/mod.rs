//! Inline-authored audio processing graphs.
//!
//! Build a graph with the functions in [`fns`], then publish it through
//! [`crate::gameplay::world::DefaultWorld::audio_graph_factory_mut`]. The
//! default audio worker validates and compiles each graph before its sample
//! resources cross to the audio thread.

use smallbox::{space::S4, SmallBox};
use smallvec::SmallVec;

use crate::core::{
	factory::{CreateMessage, Factory, Handle},
	listener::DefaultListener,
	Entity,
};

pub mod fns;

const INLINE_AUDIO_NODE_CAPACITY: usize = 8;
pub(crate) const MAX_AUDIO_GRAPH_NODES: usize = INLINE_AUDIO_NODE_CAPACITY;
pub(crate) type AudioProcessors = SmallVec<[AudioProcessor; INLINE_AUDIO_NODE_CAPACITY]>;

/// The `AudioNodeId` struct identifies one node inside an [`AudioGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AudioNodeId(usize);

/// The `AudioGraph` struct describes one source and the processing nodes that
/// route it to the default audio output.
///
/// The current graph format accepts one sample source and up to seven unary
/// processing nodes. Build it with [`fns::sample`], `loop`, and [`fns::gain`].
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
		let mut processors = SmallVec::new();

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
}

/// The `CompiledAudioGraph` struct carries validated sample and processing
/// settings from graph creation to resource loading.
#[derive(Clone)]
pub(crate) struct CompiledAudioGraph {
	pub(crate) resource_id: String,
	pub(crate) playback_mode: SamplePlaybackMode,
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
				processors: self.processors,
			},
		)
	}
}

/// The `AudioGraphRenderPlan` struct preserves validated playback and
/// processing state while the graph's sample resource loads.
pub(crate) struct AudioGraphRenderPlan {
	pub(crate) playback_mode: SamplePlaybackMode,
	pub(crate) processors: AudioProcessors,
}

/// Selects what happens when the graph's sample reaches its final frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SamplePlaybackMode {
	Once,
	Loop,
}

/// Describes one allocation-free scalar processor in a compiled graph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum AudioProcessor {
	Gain(f32),
}

impl AudioProcessor {
	/// Applies this node to one scalar sample without allocating intermediate
	/// buffers.
	pub(crate) fn process(self, sample: f32) -> f32 {
		match self {
			Self::Gain(gain) => sample * gain,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{
		fns::{gain, r#loop, sample},
		AudioProcessor, SamplePlaybackMode,
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
	fn direct_sample_compiles_as_an_unprocessed_one_shot() {
		let compiled = sample("audio/impact.wav").compile().expect("valid graph");

		assert_eq!(compiled.resource_id, "audio/impact.wav");
		assert_eq!(compiled.playback_mode, SamplePlaybackMode::Once);
		assert!(compiled.processors.is_empty());
	}

	#[test]
	#[should_panic(expected = "Invalid audio graph gain")]
	fn gain_rejects_non_finite_values_when_the_graph_is_authored() {
		let _ = gain(sample("audio/music.ogg"), f32::NAN);
	}
}
