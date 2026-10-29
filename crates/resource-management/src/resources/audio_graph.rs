//! Baked audio graphs: the nodes of an `.audiograph` file, ready for the audio worker to expand and play.

/// The `AudioGraph` struct lets the audio worker play an `.audiograph` file without parsing text.
///
/// Bake the file, then spawn it by ID through the engine's audio system, which requests this model, expands the
/// graphs its [`AudioGraphNode::Graph`] nodes name, and plays one selected path per spawn.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct AudioGraph {
	/// The graph's nodes. Every node's inputs come before it, and the last node is the output.
	pub nodes: Vec<AudioGraphNode>,
}

/// The most nodes an audio graph holds after identity nodes are removed and nested graphs are expanded.
pub const MAX_NODES: usize = 64;

super::impl_direct_resource!(AudioGraph, "AudioGraph", references = audio_graph_references);

/// The `AudioGraphNode` enum names one function of a baked audio graph and the earlier nodes it reads.
///
/// Inputs are indices into [`AudioGraph::nodes`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum AudioGraphNode {
	/// Plays the `Audio` resource once.
	Sample { resource_id: String },
	/// Stands for the output of another baked audio graph, expanded when the graph loads.
	Graph { resource_id: String },
	/// Plays the next input each time the graph is spawned.
	RoundRobin { inputs: Vec<u32> },
	/// Plays a random input each time the graph is spawned, never the same one twice in a row.
	Random { inputs: Vec<u32> },
	/// Repeats the input until the spawn is deleted.
	Loop { input: u32 },
	/// Multiplies the input's signal.
	Gain { input: u32, gain: f32 },
	/// Changes the input's speed and pitch together.
	Varispeed { input: u32, rate: f32 },
	/// Changes the input's pitch and keeps its duration.
	PitchShift { input: u32, ratio: f32 },
}

/// Lists the samples and nested graphs the graph names, in node order.
fn audio_graph_references(graph: &AudioGraph) -> Vec<&str> {
	graph
		.nodes
		.iter()
		.filter_map(|node| match node {
			AudioGraphNode::Sample { resource_id } | AudioGraphNode::Graph { resource_id } => Some(resource_id.as_str()),
			_ => None,
		})
		.collect()
}
