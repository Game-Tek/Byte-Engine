//! Eager readiness tracking for sparse dependency graphs.
//!
//! [`AvailabilityGraph`] maps application keys to compact handles. A node is
//! ready only when its own availability flag and every transitive dependency
//! are ready. Add nodes with [`AvailabilityGraph::get_or_insert`], connect them
//! with [`AvailabilityGraph::add_dependency`], then retain the returned
//! [`AvailabilityHandle`] for constant-time readiness checks.

use std::hash::Hash;

use crate::hash::{HashMap, HashMapExt as _};

const NONE: u32 = u32::MAX;
const LIVE: u8 = 1 << 0;
const AVAILABLE: u8 = 1 << 1;
const READY: u8 = 1 << 2;

/// The `AvailabilityHandle` struct identifies one live node without hashing its external key.
///
/// Retain this handle beside hot-path objects and pass it to
/// [`AvailabilityGraph::is_ready`]. A removed node invalidates its old handle,
/// even when a later insertion reuses the same storage slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AvailabilityHandle {
	index: u32,
	generation: u32,
}

/// Reports why an availability graph relationship could not be changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvailabilityGraphError {
	/// The dependent handle is stale or belongs to another graph.
	UnknownDependent,
	/// The dependency handle is stale or belongs to another graph.
	UnknownDependency,
	/// The new relationship would make readiness circular.
	Cycle,
	/// The node cannot be removed while live nodes depend on it.
	HasDependents,
}

/// The `AvailabilityGraph` struct provides constant-time readiness checks for dependency-backed objects.
///
/// The graph spends work when availability changes: each node caches its
/// transitive readiness and the number of direct dependencies that are not
/// ready. Reverse edges visit only the branch affected by an update, so reads
/// never scan dependencies.
///
/// Call [`Self::get_or_insert`] to allocate compact handles, then
/// [`Self::add_dependency`] to describe the acyclic dependency graph.
pub struct AvailabilityGraph<K> {
	handles: HashMap<K, AvailabilityHandle>,
	nodes: Vec<Node>,
	edges: Vec<Edge>,
	first_free_node: u32,
	first_free_edge: u32,
	propagation: Vec<u32>,
	traversal: Vec<u32>,
	visit_epochs: Vec<u32>,
	visit_epoch: u32,
}

#[derive(Clone, Copy)]
struct Node {
	first_dependency: u32,
	first_dependent: u32,
	unready_dependencies: u32,
	generation: u32,
	flags: u8,
}

impl Node {
	fn new(generation: u32, available: bool) -> Self {
		let mut flags = LIVE;
		if available {
			flags |= AVAILABLE | READY;
		}
		Self {
			first_dependency: NONE,
			first_dependent: NONE,
			unready_dependencies: 0,
			generation,
			flags,
		}
	}

	fn is_live(self) -> bool {
		self.flags & LIVE != 0
	}

	fn is_available(self) -> bool {
		self.flags & AVAILABLE != 0
	}

	fn is_ready(self) -> bool {
		self.flags & READY != 0
	}

	fn recompute_readiness(&mut self) -> bool {
		let was_ready = self.is_ready();
		let is_ready = self.is_available() && self.unready_dependencies == 0;
		self.flags = (self.flags & !READY) | if is_ready { READY } else { 0 };
		was_ready != is_ready
	}

	/// Applies one direct dependency transition and reports a cached-readiness transition.
	fn dependency_readiness_changed(&mut self, dependency_ready: bool) -> bool {
		self.unready_dependencies = if dependency_ready {
			self.unready_dependencies
				.checked_sub(1)
				.expect("Availability dependency count underflowed. The most likely cause is inconsistent graph propagation.")
		} else {
			self.unready_dependencies.checked_add(1).expect(
				"Availability dependency count overflowed. The most likely cause is that one node has more than u32::MAX dependencies.",
			)
		};
		self.recompute_readiness()
	}
}

#[derive(Clone, Copy)]
struct Edge {
	dependency: u32,
	dependent: u32,
	/// Next edge while walking every dependent of `dependency`.
	next_for_dependency: u32,
	/// Next edge while walking every dependency of `dependent`.
	next_for_dependent: u32,
}

impl<K: Eq + Hash> AvailabilityGraph<K> {
	/// Creates an empty graph without allocating node or edge storage.
	pub fn new() -> Self {
		Self::with_capacity(0, 0)
	}

	/// Creates an empty graph with storage for the expected nodes and relationships.
	pub fn with_capacity(node_capacity: usize, edge_capacity: usize) -> Self {
		Self {
			handles: HashMap::with_capacity(node_capacity),
			nodes: Vec::with_capacity(node_capacity),
			edges: Vec::with_capacity(edge_capacity),
			first_free_node: NONE,
			first_free_edge: NONE,
			propagation: Vec::with_capacity(node_capacity),
			traversal: Vec::with_capacity(node_capacity),
			visit_epochs: Vec::with_capacity(node_capacity),
			visit_epoch: 0,
		}
	}

	/// Returns the number of live nodes addressable by external keys.
	pub fn len(&self) -> usize {
		self.handles.len()
	}

	/// Returns whether the graph contains no live nodes.
	pub fn is_empty(&self) -> bool {
		self.handles.is_empty()
	}

	/// Returns the compact handle for `key`, if the key is registered.
	pub fn handle(&self, key: &K) -> Option<AvailabilityHandle> {
		self.handles.get(key).copied()
	}

	/// Returns an existing handle or inserts `key` with the supplied initial availability.
	///
	/// `available` is a default: it does not overwrite an existing node. Next,
	/// pass returned handles to [`Self::add_dependency`] before publishing them
	/// to readers.
	pub fn get_or_insert(&mut self, key: K, available: bool) -> AvailabilityHandle {
		if let Some(handle) = self.handles.get(&key) {
			return *handle;
		}

		let handle = self.allocate_node(available);
		self.handles.insert(key, handle);
		handle
	}

	/// Returns the cached transitive readiness for a compact handle.
	///
	/// This is the hot-path operation: it performs one indexed generation check
	/// and one flag read. Unknown or stale handles are not ready.
	pub fn is_ready(&self, handle: AvailabilityHandle) -> bool {
		self.live_node_index(handle).is_some_and(|index| self.nodes[index].is_ready())
	}

	/// Returns the cached transitive readiness for an external key.
	pub fn is_key_ready(&self, key: &K) -> bool {
		self.handle(key).is_some_and(|handle| self.is_ready(handle))
	}

	/// Replaces one node's own availability and eagerly updates every affected dependent.
	///
	/// Returns `false` when `handle` is stale or belongs to another graph.
	pub fn set_available(&mut self, handle: AvailabilityHandle, available: bool) -> bool {
		let Some(index) = self.live_node_index(handle) else {
			return false;
		};
		let node = &mut self.nodes[index];
		if node.is_available() == available {
			return true;
		}

		node.flags = (node.flags & !AVAILABLE) | if available { AVAILABLE } else { 0 };
		if node.recompute_readiness() {
			self.propagate_readiness(index as u32);
		}
		true
	}

	/// Replaces one keyed node's own availability and eagerly updates every affected dependent.
	///
	/// Returns `false` when `key` is not registered.
	pub fn set_key_available(&mut self, key: &K, available: bool) -> bool {
		let Some(handle) = self.handle(key) else {
			return false;
		};
		self.set_available(handle, available)
	}

	/// Makes `dependent` wait for the complete readiness of `dependency`.
	///
	/// Returns `Ok(false)` when the relationship already exists. Relationship
	/// insertion performs cycle detection and immediately updates the dependent
	/// branch when the dependency is not ready.
	pub fn add_dependency(
		&mut self,
		dependent: AvailabilityHandle,
		dependency: AvailabilityHandle,
	) -> Result<bool, AvailabilityGraphError> {
		let dependent_index = self
			.live_node_index(dependent)
			.ok_or(AvailabilityGraphError::UnknownDependent)? as u32;
		let dependency_index = self
			.live_node_index(dependency)
			.ok_or(AvailabilityGraphError::UnknownDependency)? as u32;

		if dependent_index == dependency_index || self.has_path(dependent_index, dependency_index) {
			return Err(AvailabilityGraphError::Cycle);
		}
		if self.relationship_exists(dependent_index, dependency_index) {
			return Ok(false);
		}

		self.insert_edge(dependency_index, dependent_index);
		if !self.nodes[dependency_index as usize].is_ready() {
			let dependent = &mut self.nodes[dependent_index as usize];
			dependent.unready_dependencies = dependent.unready_dependencies.checked_add(1).expect(
				"Availability dependency count overflowed. The most likely cause is that one node has more than u32::MAX dependencies.",
			);
			if dependent.recompute_readiness() {
				self.propagate_readiness(dependent_index);
			}
		}
		Ok(true)
	}

	/// Removes every direct dependency from `dependent` and eagerly updates its branch.
	///
	/// Use this before replacing a node's dependency set. Returns
	/// `UnknownDependent` when the handle is stale.
	pub fn clear_dependencies(&mut self, dependent: AvailabilityHandle) -> Result<(), AvailabilityGraphError> {
		let dependent_index = self
			.live_node_index(dependent)
			.ok_or(AvailabilityGraphError::UnknownDependent)? as u32;

		// Each incoming edge is also unlinked from its dependency's outgoing list.
		// Readiness is recomputed once after the complete replacement boundary.
		let mut edge_index = self.nodes[dependent_index as usize].first_dependency;
		while edge_index != NONE {
			let removed_edge = edge_index;
			let edge = self.edges[edge_index as usize];
			self.unlink_dependent_edge(edge.dependency, edge_index);
			edge_index = edge.next_for_dependent;
			self.free_edge(removed_edge);
		}

		let dependent = &mut self.nodes[dependent_index as usize];
		dependent.first_dependency = NONE;
		dependent.unready_dependencies = 0;
		if dependent.recompute_readiness() {
			self.propagate_readiness(dependent_index);
		}
		Ok(())
	}

	/// Removes a node that has no live dependents and invalidates its compact handle.
	///
	/// Removing a dependency while live nodes still require it is rejected so
	/// those nodes cannot silently become ready. Remove dependent leaves first.
	pub fn remove(&mut self, key: &K) -> Result<bool, AvailabilityGraphError> {
		let Some(handle) = self.handle(key) else {
			return Ok(false);
		};
		let index = self
			.live_node_index(handle)
			.expect("Availability key referenced a stale node. The most likely cause is inconsistent graph storage.")
			as u32;
		if self.nodes[index as usize].first_dependent != NONE {
			return Err(AvailabilityGraphError::HasDependents);
		}

		// Incoming edges are detached from both intrusive lists before the node slot
		// becomes reusable, so no live update can observe a recycled index.
		let mut edge_index = self.nodes[index as usize].first_dependency;
		while edge_index != NONE {
			let removed_edge = edge_index;
			let edge = self.edges[edge_index as usize];
			self.unlink_dependent_edge(edge.dependency, edge_index);
			edge_index = edge.next_for_dependent;
			self.free_edge(removed_edge);
		}

		self.handles.remove(key);
		let node = &mut self.nodes[index as usize];
		node.first_dependency = self.first_free_node;
		node.first_dependent = NONE;
		node.unready_dependencies = 0;
		node.generation = node.generation.wrapping_add(1);
		node.flags = 0;
		self.first_free_node = index;
		Ok(true)
	}

	/// Allocates or recycles one node slot while preserving stale-handle rejection.
	fn allocate_node(&mut self, available: bool) -> AvailabilityHandle {
		if self.first_free_node != NONE {
			let index = self.first_free_node;
			let generation = self.nodes[index as usize].generation;
			self.first_free_node = self.nodes[index as usize].first_dependency;
			self.nodes[index as usize] = Node::new(generation, available);
			return AvailabilityHandle { index, generation };
		}

		let index = u32::try_from(self.nodes.len()).expect(
			"Availability node capacity exceeded. The most likely cause is that more than u32::MAX nodes were inserted.",
		);
		self.nodes.push(Node::new(0, available));
		self.visit_epochs.push(0);
		AvailabilityHandle { index, generation: 0 }
	}

	fn live_node_index(&self, handle: AvailabilityHandle) -> Option<usize> {
		let index = handle.index as usize;
		let node = self.nodes.get(index)?;
		(node.is_live() && node.generation == handle.generation).then_some(index)
	}

	/// Adds one edge to both intrusive adjacency lists.
	fn insert_edge(&mut self, dependency: u32, dependent: u32) {
		let edge = Edge {
			dependency,
			dependent,
			next_for_dependency: self.nodes[dependency as usize].first_dependent,
			next_for_dependent: self.nodes[dependent as usize].first_dependency,
		};
		let edge_index = self.allocate_edge(edge);
		self.nodes[dependency as usize].first_dependent = edge_index;
		self.nodes[dependent as usize].first_dependency = edge_index;
	}

	fn allocate_edge(&mut self, edge: Edge) -> u32 {
		if self.first_free_edge != NONE {
			let index = self.first_free_edge;
			self.first_free_edge = self.edges[index as usize].next_for_dependency;
			self.edges[index as usize] = edge;
			return index;
		}

		let index = u32::try_from(self.edges.len()).expect(
			"Availability edge capacity exceeded. The most likely cause is that more than u32::MAX relationships were inserted.",
		);
		self.edges.push(edge);
		index
	}

	fn free_edge(&mut self, edge_index: u32) {
		let edge = &mut self.edges[edge_index as usize];
		edge.next_for_dependency = self.first_free_edge;
		self.first_free_edge = edge_index;
	}

	/// Propagates one cached-readiness transition through only its reverse branch.
	fn propagate_readiness(&mut self, root: u32) {
		self.propagation.clear();
		self.propagation.push(root);

		while let Some(dependency_index) = self.propagation.pop() {
			let dependency_ready = self.nodes[dependency_index as usize].is_ready();
			let mut edge_index = self.nodes[dependency_index as usize].first_dependent;
			while edge_index != NONE {
				let edge = self.edges[edge_index as usize];
				self.propagate_edge(edge.dependent, dependency_ready);
				edge_index = edge.next_for_dependency;
			}
		}
	}

	fn propagate_edge(&mut self, dependent: u32, dependency_ready: bool) {
		if self.nodes[dependent as usize].dependency_readiness_changed(dependency_ready) {
			self.propagation.push(dependent);
		}
	}

	fn relationship_exists(&self, dependent: u32, dependency: u32) -> bool {
		// Dependency lists are normally short (for visibility materials, at most
		// the authored texture slots), even when one resource has many dependents.
		let mut edge_index = self.nodes[dependent as usize].first_dependency;
		while edge_index != NONE {
			let edge = self.edges[edge_index as usize];
			if edge.dependency == dependency {
				return true;
			}
			edge_index = edge.next_for_dependent;
		}
		false
	}

	/// Returns whether reverse edges already connect `start` to `target`.
	fn has_path(&mut self, start: u32, target: u32) -> bool {
		self.visit_epoch = self.visit_epoch.wrapping_add(1);
		if self.visit_epoch == 0 {
			self.visit_epochs.fill(0);
			self.visit_epoch = 1;
		}
		self.traversal.clear();
		self.traversal.push(start);

		while let Some(node_index) = self.traversal.pop() {
			if node_index == target {
				return true;
			}
			let visit = &mut self.visit_epochs[node_index as usize];
			if *visit == self.visit_epoch {
				continue;
			}
			*visit = self.visit_epoch;

			let mut edge_index = self.nodes[node_index as usize].first_dependent;
			while edge_index != NONE {
				let edge = self.edges[edge_index as usize];
				self.traversal.push(edge.dependent);
				edge_index = edge.next_for_dependency;
			}
		}
		false
	}

	/// Removes one edge from a dependency's outgoing list.
	fn unlink_dependent_edge(&mut self, dependency: u32, target_edge: u32) {
		let mut edge_index = self.nodes[dependency as usize].first_dependent;
		let mut previous = NONE;
		while edge_index != NONE {
			let next = self.edges[edge_index as usize].next_for_dependency;
			if edge_index == target_edge {
				self.replace_dependent_link(dependency, previous, next);
				return;
			}
			previous = edge_index;
			edge_index = next;
		}
		unreachable!(
			"Availability edge was not linked from its dependency. The most likely cause is inconsistent graph storage."
		);
	}

	fn replace_dependent_link(&mut self, dependency: u32, previous: u32, next: u32) {
		if previous == NONE {
			self.nodes[dependency as usize].first_dependent = next;
		} else {
			self.edges[previous as usize].next_for_dependency = next;
		}
	}
}

impl<K: Eq + Hash> Default for AvailabilityGraph<K> {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::{AvailabilityGraph, AvailabilityGraphError};

	#[test]
	fn readiness_requires_the_node_and_every_transitive_dependency() {
		let mut graph = AvailabilityGraph::new();
		let texture = graph.get_or_insert("texture", false);
		let material = graph.get_or_insert("material", true);
		let object = graph.get_or_insert("object", true);

		graph.add_dependency(material, texture).unwrap();
		graph.add_dependency(object, material).unwrap();

		assert!(!graph.is_ready(texture));
		assert!(!graph.is_ready(material));
		assert!(!graph.is_ready(object));

		assert!(graph.set_available(texture, true));
		assert!(graph.is_ready(texture));
		assert!(graph.is_ready(material));
		assert!(graph.is_ready(object));
	}

	#[test]
	fn availability_changes_update_only_connected_branches() {
		let mut graph = AvailabilityGraph::new();
		let shared = graph.get_or_insert("shared", true);
		let first = graph.get_or_insert("first", true);
		let second = graph.get_or_insert("second", true);
		let independent = graph.get_or_insert("independent", true);
		graph.add_dependency(first, shared).unwrap();
		graph.add_dependency(second, shared).unwrap();

		graph.set_available(shared, false);

		assert!(!graph.is_ready(first));
		assert!(!graph.is_ready(second));
		assert!(graph.is_ready(independent));
	}

	#[test]
	fn duplicate_relationships_do_not_count_twice() {
		let mut graph = AvailabilityGraph::new();
		let dependency = graph.get_or_insert("dependency", false);
		let dependent = graph.get_or_insert("dependent", true);

		assert_eq!(graph.add_dependency(dependent, dependency), Ok(true));
		assert_eq!(graph.add_dependency(dependent, dependency), Ok(false));
		graph.set_available(dependency, true);

		assert!(graph.is_ready(dependent));
	}

	#[test]
	fn relationships_must_remain_acyclic() {
		let mut graph = AvailabilityGraph::new();
		let root = graph.get_or_insert("root", true);
		let middle = graph.get_or_insert("middle", true);
		let leaf = graph.get_or_insert("leaf", true);
		graph.add_dependency(middle, root).unwrap();
		graph.add_dependency(leaf, middle).unwrap();

		assert_eq!(graph.add_dependency(root, leaf), Err(AvailabilityGraphError::Cycle));
		assert_eq!(graph.add_dependency(root, root), Err(AvailabilityGraphError::Cycle));
	}

	#[test]
	fn insertion_uses_availability_only_as_a_default() {
		let mut graph = AvailabilityGraph::new();
		let first = graph.get_or_insert("resource", true);
		let existing = graph.get_or_insert("resource", false);

		assert_eq!(existing, first);
		assert!(graph.is_ready(existing));
	}

	#[test]
	fn removing_a_leaf_detaches_edges_and_invalidates_its_handle() {
		let mut graph = AvailabilityGraph::new();
		let dependency = graph.get_or_insert("dependency", false);
		let stale = graph.get_or_insert("old object", true);
		graph.add_dependency(stale, dependency).unwrap();

		assert_eq!(graph.remove(&"old object"), Ok(true));
		assert!(!graph.is_ready(stale));

		let replacement = graph.get_or_insert("new object", true);
		assert_ne!(replacement, stale);
		graph.set_available(dependency, true);
		assert!(graph.is_ready(replacement));
	}

	#[test]
	fn removing_a_live_dependency_is_rejected() {
		let mut graph = AvailabilityGraph::new();
		let dependency = graph.get_or_insert("dependency", true);
		let dependent = graph.get_or_insert("dependent", true);
		graph.add_dependency(dependent, dependency).unwrap();

		assert_eq!(graph.remove(&"dependency"), Err(AvailabilityGraphError::HasDependents));
		assert!(graph.is_ready(dependency));
		assert!(graph.is_ready(dependent));
	}

	#[test]
	fn replacing_dependencies_drops_the_old_branch() {
		let mut graph = AvailabilityGraph::new();
		let old_dependency = graph.get_or_insert("old dependency", false);
		let new_dependency = graph.get_or_insert("new dependency", true);
		let dependent = graph.get_or_insert("dependent", true);
		graph.add_dependency(dependent, old_dependency).unwrap();
		assert!(!graph.is_ready(dependent));

		graph.clear_dependencies(dependent).unwrap();
		graph.add_dependency(dependent, new_dependency).unwrap();

		assert!(graph.is_ready(dependent));
		graph.set_available(old_dependency, true);
		assert!(graph.is_ready(dependent));
	}

	#[test]
	fn eager_updates_match_a_topological_reference_model() {
		const NODE_COUNT: usize = 48;
		let mut graph = AvailabilityGraph::with_capacity(NODE_COUNT, NODE_COUNT * 3);
		let mut available = [false; NODE_COUNT];
		let handles = (0..NODE_COUNT)
			.map(|index| graph.get_or_insert(index, false))
			.collect::<Vec<_>>();
		let dependencies = (0..NODE_COUNT)
			.map(|dependent| {
				(0..dependent)
					.filter(|dependency| (dependent + dependency * 3) % 11 == 0)
					.collect::<Vec<_>>()
			})
			.collect::<Vec<_>>();

		for (dependent, dependencies) in dependencies.iter().enumerate() {
			for dependency in dependencies {
				graph.add_dependency(handles[dependent], handles[*dependency]).unwrap();
			}
		}

		for step in 0..NODE_COUNT * 5 {
			let changed = (step * 17 + 5) % NODE_COUNT;
			available[changed] = !available[changed];
			graph.set_available(handles[changed], available[changed]);

			let mut expected_ready = [false; NODE_COUNT];
			for index in 0..NODE_COUNT {
				expected_ready[index] =
					available[index] && dependencies[index].iter().all(|dependency| expected_ready[*dependency]);
			}
			for index in 0..NODE_COUNT {
				assert_eq!(graph.is_ready(handles[index]), expected_ready[index]);
			}
		}
	}
}
