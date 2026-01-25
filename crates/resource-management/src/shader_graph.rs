use std::collections::{HashMap, HashSet};

/// Graph structure for tracking dependencies between shader nodes.
#[derive(Clone, Debug)]
pub struct Graph {
	pub set: HashMap<besl::NodeReference, Vec<besl::NodeReference>>,
}

impl Graph {
	pub fn new() -> Self {
		Graph {
			set: HashMap::with_capacity(1024),
		}
	}

	pub fn add(&mut self, from: besl::NodeReference, to: besl::NodeReference) {
		self.set.entry(from).or_insert(Vec::new()).push(to);
	}
}

/// Performs a topological sort on the graph to determine the order in which nodes should be emitted.
pub fn topological_sort(graph: &Graph) -> Vec<besl::NodeReference> {
	let mut visited = HashSet::new();
	let mut stack = Vec::new();

	for (node, _) in graph.set.iter() {
		if !visited.contains(node) {
			topological_sort_impl(node.clone(), graph, &mut visited, &mut stack);
		}
	}

	fn topological_sort_impl(node: besl::NodeReference, graph: &Graph, visited: &mut HashSet<besl::NodeReference>, stack: &mut Vec<besl::NodeReference>) {
		visited.insert(node.clone());

		if let Some(neighbours) = graph.set.get(&node) {
			for neighbour in neighbours {
				if !visited.contains(neighbour) {
					topological_sort_impl(neighbour.clone(), graph, visited, stack);
				}
			}
		}

		stack.push(node);
	}

	stack
}
