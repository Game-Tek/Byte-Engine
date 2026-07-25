//! Authoring-time audio graph optimization passes.

use super::{AudioGraph, AudioNode, AudioNodeId};

/// Applies each audio graph optimization pass in pipeline order.
pub(super) fn optimize(graph: &mut AudioGraph) {
	eliminate_identity_nodes(graph);
}

/// Removes nodes whose output is identical to their input.
fn eliminate_identity_nodes(graph: &mut AudioGraph) {
	while let Some((removed, replacement)) = graph.nodes.iter().enumerate().find_map(|(index, node)| {
		let replacement = match &**node {
			AudioNode::RoundRobin(node) if node.inputs.len() == 1 => node.inputs[0],
			AudioNode::Random(node) if node.inputs.len() == 1 => node.inputs[0],
			AudioNode::Loop { input }
				if graph
					.nodes
					.get(input.0)
					.is_some_and(|node| matches!(&**node, AudioNode::Loop { .. })) =>
			{
				*input
			}
			AudioNode::Gain { input, gain } if *gain == 1.0 => *input,
			AudioNode::Varispeed { input, rate } if *rate == 1.0 => *input,
			AudioNode::PitchShift { input, ratio } if *ratio == 1.0 => *input,
			_ => return None,
		};
		Some((AudioNodeId(index), replacement))
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
