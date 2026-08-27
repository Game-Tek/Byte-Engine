//! Audio graph authoring and publication.

use super::*;
use crate::core::message_bus::MessageScope;

/// The `AudioGraph` struct describes resource-backed sources and the nodes that
/// select and process one source for the default audio output.
///
/// Build it with [`fns::sample`], [`fns::round_robin`], [`fns::random`], `loop`,
/// [`fns::gain`], [`fns::varispeed`], [`fns::pitch_shift`], and
/// [`fns::custom`]. Next, submit the same mutable graph again through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory`] to advance
/// its selector nodes. Stop its current play through
/// [`crate::gameplay::world::DefaultWorld::delete_channel`].
#[must_use = "Audio graphs do not play until they are published through the world's audio graph factory"]
#[derive(Debug, Clone, PartialEq)]
pub struct AudioGraph {
	pub(super) nodes: SmallVec<[SmallBox<AudioNode, S4>; INLINE_AUDIO_NODE_CAPACITY]>,
	pub(super) output: AudioNodeId,
}

impl AudioGraph {
	/// Creates a graph whose output is one resource-backed sample node.
	pub(super) fn sample(resource_id: impl Into<String>) -> Self {
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
	pub(super) fn round_robin(inputs: impl IntoIterator<Item = AudioGraph>) -> Self {
		let (mut nodes, input_ids) = Self::combine_selector_inputs(inputs, "round-robin");
		if input_ids.len() == 1 {
			let mut graph = Self {
				nodes,
				output: input_ids[0],
			};
			graph.optimize();
			return graph;
		}

		assert!(
			nodes.len() < MAX_AUDIO_GRAPH_NODES,
			"Audio graph is too large. Combining the round-robin inputs would exceed {MAX_AUDIO_GRAPH_NODES} nodes."
		);
		let output = AudioNodeId(nodes.len());
		nodes.push(SmallBox::new(AudioNode::RoundRobin(Box::new(RoundRobinNode {
			inputs: input_ids,
			next_index: 0,
		}))));
		let mut graph = Self { nodes, output };
		graph.optimize();
		graph
	}

	/// Combines independent input graphs under one non-repeating random selector.
	pub(super) fn random(inputs: impl IntoIterator<Item = AudioGraph>) -> Self {
		let (mut nodes, input_ids) = Self::combine_selector_inputs(inputs, "random");
		if input_ids.len() == 1 {
			let mut graph = Self {
				nodes,
				output: input_ids[0],
			};
			graph.optimize();
			return graph;
		}

		assert!(
			nodes.len() < MAX_AUDIO_GRAPH_NODES,
			"Audio graph is too large. Combining the random inputs would exceed {MAX_AUDIO_GRAPH_NODES} nodes."
		);
		let output = AudioNodeId(nodes.len());
		nodes.push(SmallBox::new(AudioNode::Random(Box::new(RandomNode::new(input_ids)))));
		let mut graph = Self { nodes, output };
		graph.optimize();
		graph
	}

	/// Remaps independent graph inputs into one selector-ready node list.
	fn combine_selector_inputs(
		inputs: impl IntoIterator<Item = AudioGraph>,
		selector_name: &str,
	) -> (
		SmallVec<[SmallBox<AudioNode, S4>; INLINE_AUDIO_NODE_CAPACITY]>,
		SelectorInputs,
	) {
		let mut nodes = SmallVec::new();
		let mut input_ids = SmallVec::new();

		for input in inputs {
			assert!(
				nodes.len() + input.nodes.len() <= MAX_AUDIO_GRAPH_NODES,
				"Audio graph is too large. Combining the {selector_name} inputs would exceed {MAX_AUDIO_GRAPH_NODES} nodes."
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
			"Invalid audio {selector_name} node. No input chains were provided."
		);
		(nodes, input_ids)
	}

	/// Applies the authoring-graph optimization pipeline in place.
	fn optimize(&mut self) {
		optimization::optimize(self);
	}

	/// Appends a loop node and makes it the graph output.
	pub(super) fn looping(mut self) -> Self {
		if matches!(&*self.nodes[self.output.0], AudioNode::Loop { .. }) {
			return self;
		}
		let input = self.output;
		self.push(AudioNode::Loop { input });
		self
	}

	/// Appends a gain node and makes it the graph output.
	pub(super) fn with_gain(mut self, gain: f32) -> Self {
		assert!(
			gain.is_finite() && gain >= 0.0,
			"Invalid audio graph gain. The gain must be finite and non-negative."
		);
		if gain == 1.0 {
			return self;
		}
		let input = self.output;
		self.push(AudioNode::Gain { input, gain });
		self
	}

	/// Appends a varispeed node that changes playback speed and pitch together.
	pub(super) fn with_varispeed(mut self, rate: f32) -> Self {
		assert!(
			rate.is_finite() && (0.25..=4.0).contains(&rate),
			"Invalid audio graph varispeed rate. The provided rate is not finite or is outside 0.25..=4.0."
		);
		if rate == 1.0 {
			return self;
		}

		assert!(
			!self.nodes.iter().any(|node| matches!(&**node, AudioNode::Varispeed { .. })),
			"Invalid audio graph varispeed. A second varispeed node was added to the graph."
		);
		let input = self.output;
		self.push(AudioNode::Varispeed { input, rate });
		self
	}

	/// Appends a duration-preserving pitch-shift node.
	pub(super) fn with_pitch_shift(mut self, ratio: f32) -> Self {
		assert!(
			ratio.is_finite() && (0.5..=2.0).contains(&ratio),
			"Invalid audio graph pitch ratio. The ratio must be finite and between 0.5 and 2.0."
		);
		if ratio == 1.0 {
			return self;
		}

		assert!(
			!self.nodes.iter().any(|node| matches!(&**node, AudioNode::PitchShift { .. })),
			"Invalid audio graph pitch shift. A graph can contain at most one pitch-shift node."
		);
		let input = self.output;
		self.push(AudioNode::PitchShift { input, ratio });
		self
	}

	/// Appends a user-provided block processor with per-playback closure state.
	pub(super) fn with_custom<F>(mut self, function: F) -> Self
	where
		F: FnMut(AudioGraphTime, &mut [f32]) + Clone + Send + Sync + 'static,
	{
		let input = self.output;
		self.push(AudioNode::Custom(input, CustomAudioFunction::new(function)));
		self
	}

	pub(super) fn push(&mut self, node: AudioNode) {
		assert!(
			self.nodes.len() < MAX_AUDIO_GRAPH_NODES,
			"Audio graph is too large. A graph can contain at most {MAX_AUDIO_GRAPH_NODES} nodes."
		);
		self.output = AudioNodeId(self.nodes.len());
		self.nodes.push(SmallBox::new(node));
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
	/// Creates a standalone empty graph factory.
	///
	/// Use this constructor for isolated tests. Applications normally use
	/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory`] instead.
	pub fn new() -> Self {
		Self {
			compiled_graphs: Factory::new(),
		}
	}

	/// Creates a graph factory that publishes through a shared message scope.
	///
	/// Use this during application setup so audio graph creation participates in
	/// unified message diagnostics. Next, install the audio worker's listener
	/// before calling [`Self::create`].
	pub(crate) fn in_scope(scope: &MessageScope) -> Self {
		Self {
			compiled_graphs: scope.factory(),
		}
	}

	/// Compiles and publishes a graph with a new lifecycle handle.
	///
	/// This commits each selector node on the selected path. Keep the graph and
	/// submit it again to play its next selection.
	///
	/// Send the returned handle through
	/// [`crate::gameplay::world::DefaultWorld::delete_channel`] to stop the
	/// graph.
	pub fn create(&self, graph: &mut AudioGraph) -> Handle {
		let (compiled, selector_commits) = compile_for_factory(graph);
		let handle = self.compiled_graphs.create(compiled);
		graph.commit_selectors(&selector_commits);
		handle
	}

	/// Compiles and publishes a replacement with an existing lifecycle handle.
	///
	/// This commits each selector node on the selected path.
	///
	/// Use this to replace a graph while preserving the identity returned by
	/// [`Self::create`].
	pub fn derive(&self, handle: Handle, graph: &mut AudioGraph) {
		let (compiled, selector_commits) = compile_for_factory(graph);
		self.compiled_graphs.derive(handle, compiled);
		graph.commit_selectors(&selector_commits);
	}

	pub(crate) fn listener(&self) -> DefaultListener<CreateMessage<CompiledAudioGraph>> {
		self.compiled_graphs.listener()
	}
}

/// Compiles an authored graph on its creating thread before the factory sends
/// any work to the audio worker.
fn compile_for_factory(graph: &mut AudioGraph) -> (CompiledAudioGraph, SelectorCommits) {
	graph.optimize();
	graph
		.compile_selection()
		.unwrap_or_else(|error| panic!("Audio graph was not created. The authored graph is invalid: {error}"))
}

#[cfg(test)]
mod tests {
	use super::AudioGraphFactory;
	use crate::{
		audio::graph::fns,
		core::{
			listener::Listener,
			message_bus::{MessageBus, MessageBusConfig},
		},
	};

	/// Verifies that scoped audio factories share one lazily registered creation route.
	#[test]
	fn scoped_factories_share_compiled_graph_creations() {
		let bus = MessageBus::new(MessageBusConfig::new(1, 8, 1024)).expect("valid audio test bus");
		let scope = bus.new_scope("audio-test");
		let producer = AudioGraphFactory::in_scope(&scope);
		let observer = AudioGraphFactory::in_scope(&scope);
		let mut listener = observer.listener();
		let mut graph = fns::sample("audio/test.wav");

		let handle = producer.create(&mut graph);
		let message = listener.read().expect("scoped audio graph creation");

		assert_eq!(message.handle(), handle);
		assert_eq!(scope.topics().len(), 1);
	}
}
