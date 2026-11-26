//! Graph search, navigation meshes, and XZ-plane path finding.
//!
//! Declare where agents walk with [`NavigationArea`] components on the world and let a [`Navigation`] system build
//! their [`NavigationMesh`]; then call [`NavigationMesh::find_path`] on it for a string-pulled world-space path, and
//! [`Navigation::steer`] to keep each [`NavigationAgent`] moving along it clear of the others.
//! Build a mesh directly with [`NavigationMesh::from_areas`] or [`NavigationMesh::new`] when the polygons don't come
//! from the world. [`PathSearch`] runs the A* search under [`NavigationMesh::find_path`] on any [`Graph`].

mod area_mesh;
mod navigation;
mod navigation_mesh;
mod steering;

pub use navigation::{Navigation, NavigationAgent, NavigationArea, NavigationSteering};
pub use navigation_mesh::{
	NAVIGATION_TOLERANCE, NavigationMesh, NavigationMeshBuildError, NavigationPathError, NavigationPortal,
	NavigationVertexHandle, StringPullError, string_pull, string_pull_into,
};

/// The `PathSearch` struct retains A* working storage so repeated searches do not allocate.
///
/// Keep one per system that searches every tick. A search may run on any graph; storage grows to the largest graph
/// seen. `P` is what a search carries along its route, such as the point a navigation mesh path enters each polygon
/// at.
pub struct PathSearch<P> {
	frontier: BinaryHeap<FrontierEntry>,
	paths: Vec<Option<PathState<P>>>,
}

impl<P> Default for PathSearch<P> {
	fn default() -> Self {
		Self {
			frontier: BinaryHeap::new(),
			paths: Vec::new(),
		}
	}
}

impl<P: Copy> PathSearch<P> {
	/// Finds the lowest-cost path from `start`, carrying `payload`, to `target` and writes it into `path`.
	///
	/// `step` gives the cost of going from a node, with what the route carries there, to a neighbor, and what the
	/// route carries into the neighbor; `heuristic` estimates the cost left from a node with what it carries, and
	/// must not overestimate it for the path to be the cheapest, such as when a step's cost depends on where the
	/// route crosses into a polygon. `path` is replaced. It includes both endpoints, or is left empty when the target
	/// is unreachable; the return value reports reachability.
	pub fn find_with(
		&mut self,
		start: NodeHandle,
		payload: P,
		target: NodeHandle,
		graph: &impl Graph,
		step: impl Fn(NodeHandle, P, NodeHandle) -> (f32, P),
		heuristic: impl Fn(NodeHandle, P) -> f32,
		path: &mut Vec<NodeHandle>,
	) -> bool {
		path.clear();
		let Self { frontier, paths } = self;
		frontier.clear();
		frontier.push(FrontierEntry {
			priority: 0f32,
			cost: 0f32,
			node: start,
		});
		paths.clear();
		paths.resize(graph.node_count(), None);
		paths[start as usize] = Some(PathState {
			cost: 0f32,
			predecessor: None,
			payload,
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
				let (step_cost, payload) = step(node, current_path.payload, next);
				let new_cost = current_path.cost + step_cost;
				let next_path = &mut paths[next as usize];
				let improves_path = next_path.is_none_or(|path| new_cost < path.cost);

				if improves_path {
					*next_path = Some(PathState {
						cost: new_cost,
						predecessor: Some(node),
						payload,
					});
					frontier.push(FrontierEntry {
						priority: new_cost + heuristic(next, payload),
						cost: new_cost,
						node: next,
					});
				}
			}
		}

		if paths[target as usize].is_none() {
			return false;
		}

		// Follow predecessors backward, then reverse them into traversal order.
		path.extend(std::iter::successors(Some(target), |current| {
			paths[*current as usize].and_then(|path| path.predecessor)
		}));
		path.reverse();
		true
	}
}

/// The `FrontierEntry` struct prioritizes a pending node and detects outdated routes.
struct FrontierEntry {
	priority: f32,
	cost: f32,
	node: NodeHandle,
}

/// The `PathState` struct stores the best-known route to a visited node, with what the route carries there.
struct PathState<P> {
	cost: f32,
	predecessor: Option<NodeHandle>,
	payload: P,
}

impl<P: Copy> Clone for PathState<P> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<P: Copy> Copy for PathState<P> {}

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

pub type NodeHandle = u32;

/// The `Graph` trait provides indexed nodes for path searches.
pub trait Graph {
	fn node_count(&self) -> usize;
	fn neighbors(&self, node: NodeHandle) -> impl Iterator<Item = NodeHandle> + '_;
}

use std::collections::BinaryHeap;
