//! Audio graph assets: baked `.audiograph` files, spawned by ID and expanded into authored graphs.

use std::borrow::Cow;

use resource_management::resources::audio_graph::{AudioGraph as BakedAudioGraph, AudioGraphNode};

use super::*;

/// The `AudioGraphAsset` struct lets an application play a baked `.audiograph` file by publishing its ID.
///
/// Publish it on the world scope, `world.create(AudioGraphAsset::new("sfx/steps.audiograph"))`, and the audio worker
/// loads the file and the graphs it nests, chooses a path, and plays it. Every spawn of one ID shares that graph's
/// selectors, so a round robin advances across all of them. Stop a spawn by sending a
/// [`crate::core::message::DeleteMessage`] for its handle. To build a graph in code instead, use [`fns`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioGraphAsset {
	pub(crate) id: Cow<'static, str>,
}

impl AudioGraphAsset {
	/// Names the baked `.audiograph` file to play, by its path under the assets directory.
	pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
		Self { id: id.into() }
	}
}

impl Entity for AudioGraphAsset {}

impl AudioGraph {
	/// Builds the authored graph that the baked graph `id` describes, expanding every nested graph in place.
	///
	/// `baked` returns the baked graph of an ID, or `None` when it wasn't loaded. Each expansion gets its own selector
	/// state. Returns the cause and fix of the first problem, ready to follow a statement of what failed.
	pub(crate) fn expand<'a>(id: &'a str, baked: impl Fn(&str) -> Option<&'a BakedAudioGraph>) -> Result<Self, String> {
		let mut nodes = SmallVec::new();
		let mut chain = SmallVec::<[&str; 8]>::new();
		let output = expand_into(id, &baked, &mut chain, &mut nodes)?;
		Ok(Self { nodes, output })
	}

	/// Compiles the current selection and advances the selectors on its path, for one spawn of an asset.
	pub(crate) fn select(&mut self) -> Result<CompiledAudioGraph, String> {
		let (compiled, selector_commits) = self.compile_selection()?;
		self.commit_selectors(&selector_commits);
		Ok(compiled)
	}
}

/// Appends the nodes of the baked graph `id` to `nodes` and returns the node that stands for its output.
///
/// `chain` holds the graphs being expanded around this one, so a graph that nests itself is caught.
fn expand_into<'a>(
	id: &'a str,
	baked: &impl Fn(&str) -> Option<&'a BakedAudioGraph>,
	chain: &mut SmallVec<[&'a str; 8]>,
	nodes: &mut SmallVec<[SmallBox<AudioNode, S4>; INLINE_AUDIO_NODE_CAPACITY]>,
) -> Result<AudioNodeId, String> {
	if chain.contains(&id) {
		let path = chain
			.iter()
			.chain([&id])
			.map(|id| format!("'{id}'"))
			.collect::<Vec<_>>()
			.join(" names ");
		return Err(format!(
			"The most likely cause is that its `graph` nodes lead back to a graph that contains them: {path}. Remove one `graph` node of that chain."
		));
	}
	let graph =
		baked(id).ok_or_else(|| format!("The most likely cause is that '{id}' wasn't loaded. Bake it and spawn again."))?;
	let malformed = || format!("The most likely cause is that '{id}' was baked by an older engine. Rebake it.");
	chain.push(id);

	// The node that stands for each baked node, so later nodes find their inputs.
	let mut expanded = SmallVec::<[AudioNodeId; INLINE_AUDIO_NODE_CAPACITY]>::with_capacity(graph.nodes.len());
	for node in &graph.nodes {
		let input = |index: &u32| expanded.get(*index as usize).copied().ok_or_else(malformed);
		let selector_inputs = |inputs: &[u32]| inputs.iter().map(input).collect::<Result<SelectorInputs, _>>();
		let node = match node {
			AudioGraphNode::Graph { resource_id } => {
				let output = expand_into(resource_id, baked, chain, nodes)?;
				expanded.push(output);
				continue;
			}
			AudioGraphNode::Loop { input: index } => {
				let input = input(index)?;
				// A nested graph that already loops needs no second loop.
				if matches!(&*nodes[input.0], AudioNode::Loop { .. }) {
					expanded.push(input);
					continue;
				}
				AudioNode::Loop { input }
			}
			AudioGraphNode::Sample { resource_id } => AudioNode::Sample {
				resource_id: resource_id.clone(),
			},
			AudioGraphNode::RoundRobin { inputs } => AudioNode::RoundRobin(Box::new(RoundRobinNode {
				inputs: selector_inputs(inputs)?,
				next_index: 0,
			})),
			AudioGraphNode::Random { inputs } => AudioNode::Random(Box::new(RandomNode::new(selector_inputs(inputs)?))),
			AudioGraphNode::Gain { input: index, gain } => AudioNode::Gain {
				input: input(index)?,
				gain: *gain,
			},
			AudioGraphNode::Varispeed { input: index, rate } => AudioNode::Varispeed {
				input: input(index)?,
				rate: *rate,
			},
			AudioGraphNode::PitchShift { input: index, ratio } => AudioNode::PitchShift {
				input: input(index)?,
				ratio: *ratio,
			},
		};
		if nodes.len() == MAX_AUDIO_GRAPH_NODES {
			return Err(format!(
				"The most likely cause is that it has more than {MAX_AUDIO_GRAPH_NODES} nodes once its nested graphs expand. Nest fewer graphs or remove selector entries."
			));
		}
		nodes.push(SmallBox::new(node));
		expanded.push(AudioNodeId(nodes.len() - 1));
	}

	chain.pop();
	expanded.last().copied().ok_or_else(malformed)
}

#[cfg(test)]
mod tests;
