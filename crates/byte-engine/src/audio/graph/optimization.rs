//! Authoring-time audio graph optimization passes.

use super::{AudioGraph, AudioNode, AudioNodeId};

/// Applies each audio graph optimization pass in pipeline order.
pub(super) fn optimize(graph: &mut AudioGraph) {
	eliminate_single_input_selector_nodes(graph);
}

/// Removes selectors whose only possible choice is their sole input.
fn eliminate_single_input_selector_nodes(graph: &mut AudioGraph) {
	while let Some((removed, replacement)) = graph.nodes.iter().enumerate().find_map(|(index, node)| {
		let inputs = match &**node {
			AudioNode::RoundRobin(node) => &node.inputs,
			AudioNode::Random(node) => &node.inputs,
			_ => return None,
		};
		(inputs.len() == 1).then_some((AudioNodeId(index), inputs[0]))
	}) {
		debug_assert_ne!(removed, replacement);
		graph.nodes.remove(removed.0);

		for node in &mut graph.nodes {
			node.reconnect_after_removal(removed, replacement);
		}
		reconnect_id_after_removal(&mut graph.output, removed, replacement);
	}
}

/// Reconnects one node ID and accounts for compaction after a node is removed.
fn reconnect_id_after_removal(id: &mut AudioNodeId, removed: AudioNodeId, replacement: AudioNodeId) {
	if *id == removed {
		*id = replacement;
	}
	if id.0 > removed.0 {
		id.0 -= 1;
	}
}

impl AudioNode {
	/// Reconnects every input after an intermediate node is removed.
	fn reconnect_after_removal(&mut self, removed: AudioNodeId, replacement: AudioNodeId) {
		match self {
			Self::Sample { .. } => {}
			Self::RoundRobin(node) => {
				for input in &mut node.inputs {
					reconnect_id_after_removal(input, removed, replacement);
				}
			}
			Self::Random(node) => {
				for input in &mut node.inputs {
					reconnect_id_after_removal(input, removed, replacement);
				}
			}
			Self::Loop { input }
			| Self::Gain { input, .. }
			| Self::Varispeed { input, .. }
			| Self::PitchShift { input, .. } => reconnect_id_after_removal(input, removed, replacement),
		}
	}
}
