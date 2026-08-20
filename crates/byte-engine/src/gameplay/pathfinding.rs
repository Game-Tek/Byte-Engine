//! Graph search and XZ-plane navigation-mesh path finding.
//!
//! Use [`a_star`] for an arbitrary graph. Use [`NavigationMesh::find_path`] when indexed convex
//! navigation polygons should produce a string-pulled world-space path.

mod navigation_mesh;

pub use navigation_mesh::{
	string_pull, string_pull_into, NavigationMesh, NavigationMeshBuildError, NavigationPathError, NavigationPortal,
	NavigationVertexHandle, StringPullError,
};

/// Finds the lowest-cost path from `start` to `target`.
///
/// `distance` must return a non-negative, finite edge cost for adjacent nodes and
/// an admissible cost estimate for other node pairs. The returned path includes
/// both endpoints, or is empty when the target is unreachable.
pub fn a_star<T>(
	start: NodeHandle,
	target: NodeHandle,
	graph: &impl Graph<T>,
	distance: impl Fn(NodeHandle, NodeHandle) -> f32,
) -> Vec<NodeHandle> {
	let mut heap = BinaryHeap::with_capacity(64);
	a_star_with_heap(start, target, graph, distance, &mut heap)
}

/// The `FrontierEntry` struct prioritizes a pending node and detects outdated routes.
struct FrontierEntry {
	priority: f32,
	cost: f32,
	node: NodeHandle,
}

/// The `PathState` struct stores the best-known route to a visited node.
#[derive(Clone, Copy)]
struct PathState {
	cost: f32,
	predecessor: Option<NodeHandle>,
}

impl Eq for FrontierEntry {}

impl PartialEq for FrontierEntry {
	fn eq(&self, other: &Self) -> bool {
		self.priority.total_cmp(&other.priority).is_eq()
	}
}

impl PartialOrd for FrontierEntry {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for FrontierEntry {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		// BinaryHeap is a max-heap, so reverse the comparison to pop the lowest cost.
		other.priority.total_cmp(&self.priority)
	}
}

/// Finds a lowest-cost path while reusing the supplied frontier allocation.
fn a_star_with_heap<T, A: Allocator>(
	start: NodeHandle,
	target: NodeHandle,
	graph: &impl Graph<T>,
	distance: impl Fn(NodeHandle, NodeHandle) -> f32,
	frontier: &mut BinaryHeap<FrontierEntry, A>,
) -> Vec<NodeHandle> {
	// A caller may reuse a heap left populated by an earlier search.
	frontier.clear();
	frontier.push(FrontierEntry {
		priority: 0f32,
		cost: 0f32,
		node: start,
	});

	let mut paths = vec![None; graph.node_count()];
	paths[start as usize] = Some(PathState {
		cost: 0f32,
		predecessor: None,
	});

	while let Some(FrontierEntry { cost, node, .. }) = frontier.pop() {
		let Some(current_path) = paths[node as usize] else {
			continue;
		};

		// Cheaper routes leave their previous entries in the heap.
		if cost > current_path.cost {
			continue;
		}

		if node == target {
			break;
		}

		for next in graph.neighbors(node) {
			let new_cost = current_path.cost + distance(node, next);
			let next_path = &mut paths[next as usize];
			let improves_path = next_path.is_none_or(|path| new_cost < path.cost);

			if improves_path {
				*next_path = Some(PathState {
					cost: new_cost,
					predecessor: Some(node),
				});
				frontier.push(FrontierEntry {
					priority: new_cost + distance(next, target),
					cost: new_cost,
					node: next,
				});
			}
		}
	}

	if paths[target as usize].is_none() {
		return Vec::new();
	}

	// Follow predecessors backward, then reverse them into traversal order.
	let mut path: Vec<_> = std::iter::successors(Some(target), |current| {
		paths[*current as usize].and_then(|path| path.predecessor)
	})
	.collect();
	path.reverse();
	path
}

pub type NodeHandle = u32;

/// The `Graph` trait provides indexed nodes for path searches.
pub trait Graph<N> {
	fn node_count(&self) -> usize;
	fn neighbors(&self, node: NodeHandle) -> impl Iterator<Item = NodeHandle> + '_;
}

/// The `TrivialGraph` struct provides simple edge-list storage for small or reference graphs.
pub struct TrivialGraph<N> {
	nodes: Vec<N>,
	edges: Vec<NodeEdge>,
}

impl<N> Graph<N> for TrivialGraph<N> {
	fn node_count(&self) -> usize {
		self.nodes.len()
	}

	fn neighbors(&self, node: NodeHandle) -> impl Iterator<Item = NodeHandle> + '_ {
		self.edges
			.iter()
			.filter_map(move |&(a, b)| (a == node).then_some(b).or((b == node).then_some(a)))
	}
}

impl<N> TrivialGraph<N> {
	pub fn new() -> Self {
		Self {
			nodes: Vec::with_capacity(64),
			edges: Vec::with_capacity(64),
		}
	}

	pub fn push(&mut self, value: N) -> NodeHandle {
		let i = self.nodes.len();
		self.nodes.push(value);
		i as _
	}

	pub fn push_connected(&mut self, x: NodeHandle, value: N) -> NodeHandle {
		let i = self.push(value);
		self.edges.push((x, i));
		i as _
	}

	pub fn connect(&mut self, x: NodeEdge) {
		if self.edges.iter().any(|&y| Self::do_edges_match(x, y)) {
			return;
		}

		self.edges.push(x);
	}

	pub fn get(&self, node: NodeHandle) -> &N {
		&self.nodes[node as usize]
	}

	pub fn node_iterator<'a>(&'a self, node: NodeHandle) -> impl NodeIterator<N> + 'a {
		struct GrahNodeIterator<'a, T> {
			graph: &'a TrivialGraph<T>,
			node: NodeHandle,
		}

		impl<'a, T> GrahNodeIterator<'a, T> {
			fn new(graph: &'a TrivialGraph<T>, node: NodeHandle) -> Self {
				Self { graph, node }
			}
		}

		impl<'a, T> NodeIterator<T> for GrahNodeIterator<'a, T> {
			fn cost(&self) -> f32 {
				0f32
			}

			fn handle(&self) -> NodeHandle {
				self.node
			}

			fn value(&self) -> &T {
				self.graph.get(self.node)
			}

			fn neighbors(&self) -> impl Iterator<Item = Self> {
				self.graph.neighbors(self.node).map(|e| Self::new(self.graph, e))
			}
		}

		impl<'a, T> PartialEq for GrahNodeIterator<'a, T> {
			fn eq(&self, other: &Self) -> bool {
				self.node == other.node
			}
		}

		impl<'a, T> Eq for GrahNodeIterator<'a, T> {}

		GrahNodeIterator::new(self, node)
	}

	fn do_edges_match((a, b): NodeEdge, (x, y): NodeEdge) -> bool {
		(a == x && b == y) || (b == x && a == y)
	}
}

/// The `BitMatrixGraph` struct provides dense graph storage with contiguous bit-packed rows.
///
/// Use [`BitMatrixGraph::with_capacity`] when you know the maximum node count. The fixed
/// capacity keeps rows stable as nodes are added and lets [`Graph::neighbors`] scan each
/// row without pointer indirection.
pub struct BitMatrixGraph<N> {
	nodes: Vec<N>,
	adjacency: Vec<u64>,
	capacity: usize,
	words_per_row: usize,
}

impl<N> BitMatrixGraph<N> {
	/// Creates a graph that can hold up to `capacity` nodes.
	pub fn with_capacity(capacity: usize) -> Self {
		let words_per_row = capacity.div_ceil(u64::BITS as usize);
		Self {
			nodes: Vec::with_capacity(capacity),
			adjacency: vec![0; capacity * words_per_row],
			capacity,
			words_per_row,
		}
	}

	/// Adds an unconnected node and returns its handle.
	pub fn push(&mut self, value: N) -> NodeHandle {
		assert!(
			self.nodes.len() < self.capacity,
			"bit matrix graph capacity exceeded. The graph has reached its fixed node capacity"
		);

		let handle = self.nodes.len() as NodeHandle;
		self.nodes.push(value);
		handle
	}

	/// Adds a node connected to `node` and returns its handle.
	pub fn push_connected(&mut self, node: NodeHandle, value: N) -> NodeHandle {
		let added = self.push(value);
		self.connect((node, added));
		added
	}

	/// Connects both endpoints of an undirected edge.
	pub fn connect(&mut self, (a, b): NodeEdge) {
		assert!(
			(a as usize) < self.nodes.len() && (b as usize) < self.nodes.len(),
			"invalid graph edge. One or both node handles are outside the graph"
		);

		self.set_adjacent(a, b);
		self.set_adjacent(b, a);
	}

	fn set_adjacent(&mut self, from: NodeHandle, to: NodeHandle) {
		let bit = to as usize;
		let word = from as usize * self.words_per_row + bit / u64::BITS as usize;
		self.adjacency[word] |= 1 << (bit % u64::BITS as usize);
	}
}

impl<N> Graph<N> for BitMatrixGraph<N> {
	fn node_count(&self) -> usize {
		self.nodes.len()
	}

	fn neighbors(&self, node: NodeHandle) -> impl Iterator<Item = NodeHandle> + '_ {
		let row_start = node as usize * self.words_per_row;
		let row = &self.adjacency[row_start..row_start + self.words_per_row];

		// Extract set bits a word at a time instead of testing every possible edge.
		row.iter().copied().enumerate().flat_map(|(word_index, mut word)| {
			std::iter::from_fn(move || {
				if word == 0 {
					return None;
				}

				let bit = word.trailing_zeros() as usize;
				word &= word - 1;
				Some((word_index * u64::BITS as usize + bit) as NodeHandle)
			})
		})
	}
}

pub type NodeEdge = (NodeHandle, NodeHandle);

pub trait NodeIterator<T>: Eq {
	fn handle(&self) -> NodeHandle;
	fn cost(&self) -> f32;
	fn value(&self) -> &T;
	fn neighbors(&self) -> impl Iterator<Item = Self>;
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The `TestGraph` trait gives each graph implementation the same test setup API.
	trait TestGraph<N>: Graph<N> {
		fn push(&mut self, value: N) -> NodeHandle;
		fn connect(&mut self, edge: NodeEdge);

		fn push_connected(&mut self, node: NodeHandle, value: N) -> NodeHandle {
			let added = self.push(value);
			self.connect((node, added));
			added
		}
	}

	impl<N> TestGraph<N> for TrivialGraph<N> {
		fn push(&mut self, value: N) -> NodeHandle {
			TrivialGraph::push(self, value)
		}

		fn connect(&mut self, edge: NodeEdge) {
			TrivialGraph::connect(self, edge);
		}
	}

	impl<N> TestGraph<N> for BitMatrixGraph<N> {
		fn push(&mut self, value: N) -> NodeHandle {
			BitMatrixGraph::push(self, value)
		}

		fn connect(&mut self, edge: NodeEdge) {
			BitMatrixGraph::connect(self, edge);
		}
	}

	fn assert_finds_a_direct_path(mut graph: impl TestGraph<u8>) {
		let start = graph.push(0);
		let end = graph.push_connected(start, 1);

		let path = a_star(start, end, &graph, |_, _| 1f32);

		assert_eq!(path.as_slice(), [start, end]);
	}

	fn assert_chooses_the_lower_cost_path(mut graph: impl TestGraph<u8>) {
		let start = graph.push(0);
		let middle = graph.push_connected(start, 1);
		let end = graph.push_connected(middle, 2);
		graph.connect((start, end));

		let path = a_star(start, end, &graph, |from, to| {
			if (from == start && to == end) || (from == end && to == start) {
				10f32
			} else if from == to {
				0f32
			} else {
				1f32
			}
		});

		assert_eq!(path.as_slice(), [start, middle, end]);
	}

	fn assert_returns_an_empty_path_when_target_is_unreachable(mut graph: impl TestGraph<u8>) {
		let start = graph.push(0);
		let end = graph.push(1);

		let path = a_star(start, end, &graph, |_, _| 1f32);

		assert!(path.is_empty());
	}

	#[test]
	fn finds_a_direct_path() {
		assert_finds_a_direct_path(TrivialGraph::new());
		assert_finds_a_direct_path(BitMatrixGraph::with_capacity(2));
	}

	#[test]
	fn chooses_the_lower_cost_path() {
		assert_chooses_the_lower_cost_path(TrivialGraph::new());
		assert_chooses_the_lower_cost_path(BitMatrixGraph::with_capacity(3));
	}

	#[test]
	fn returns_an_empty_path_when_target_is_unreachable() {
		assert_returns_an_empty_path_when_target_is_unreachable(TrivialGraph::new());
		assert_returns_an_empty_path_when_target_is_unreachable(BitMatrixGraph::with_capacity(2));
	}

	#[test]
	fn bit_matrix_graph_reports_each_neighbor_once() {
		let mut graph = BitMatrixGraph::with_capacity(130);
		let start = graph.push(0);
		let neighbors: Vec<_> = (1..=65).map(|value| graph.push_connected(start, value)).collect();

		assert_eq!(graph.neighbors(start).collect::<Vec<_>>(), neighbors);
	}
}

use std::{alloc::Allocator, collections::BinaryHeap};
