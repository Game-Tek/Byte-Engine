use std::collections::{HashMap, HashSet};

use super::{ConcreteElement, Id, IdedElement, PathSegment};
use crate::ui::{
	primitive::{Primitive, Primitives},
	style::{EdgeFeather, Layer},
};

// Clip inheritance depends on these container properties, independently of paint color.
fn clip_inputs(primitive: &Primitives) -> Option<(bool, bool, f32, f32, Option<EdgeFeather>)> {
	let Primitives::Container(container) = primitive else {
		return None;
	};
	Some((
		container.clip,
		matches!(container.depth, super::Depth::Absolute(_)),
		container.corner_radius,
		container.corner_exponent,
		container
			.style
			.layers()
			.iter()
			.map(Layer::feather)
			.find(|feather| !feather.is_none()),
	))
}

/// The `RetainedTree` struct owns stable UI identities and the live topology used by layout.
#[derive(Default)]
pub(super) struct RetainedTree {
	pub(super) elements: Vec<IdedElement>,
	pub(super) element_indices: HashMap<Id, usize>,
	pub(super) relations: Vec<(Id, Id)>,
	/// Dense links follow `elements`; only external identity lookup needs hashing.
	pub(super) children: Vec<Vec<usize>>,
	pub(super) parents: Vec<Option<usize>>,
	path_counts: HashMap<(Option<Id>, &'static str), u32>,
	path_ids: HashMap<(usize, PathSegment), usize>,
	/// Interned paths retain their original scope ancestry even after visual reparenting.
	paths: Vec<(usize, Option<Id>)>,
	next_id: u32,
	/// Advances on every structural or property change so consumers can retain derived state.
	revision: u64,
	/// Structural edits also invalidate clipping, including remounts that reuse IDs.
	pub(super) clip_revision: u64,
	/// Clipping and inherited opacity can change independently of paint color.
	pub(super) appearance_revision: u64,
}

impl RetainedTree {
	pub(super) fn new() -> Self {
		Self {
			next_id: 1,
			paths: vec![(0, None)],
			..Self::default()
		}
	}

	pub(super) fn begin_frame(&mut self) {
		self.path_counts.clear();
	}

	/// Returns a value that changes whenever elements are added, removed, or mutated.
	///
	/// Layout and render data derived from one revision stay valid until it changes.
	pub(super) fn revision(&self) -> u64 {
		self.revision
	}

	/// Interns a structural path so mounted contexts share ancestry without copying it.
	pub(super) fn scope_path(&mut self, parent: Option<Id>, parent_path: usize, name: &'static str) -> usize {
		let count = self.path_counts.entry((parent, name)).or_insert(0);
		*count += 1;
		let key = (parent_path, PathSegment { name, ordinal: *count });
		*self.path_ids.entry(key).or_insert_with(|| {
			let index = self.paths.len();
			self.paths.push((parent_path, None));
			index
		})
	}

	/// Returns the stable element identity assigned to an interned path.
	fn id_for_path(&mut self, path: usize) -> Id {
		*self.paths[path].1.get_or_insert_with(|| {
			let id = Id::new(self.next_id).expect("UI id counter must stay non-zero");
			self.next_id += 1;
			id
		})
	}

	/// Adds a declaration once and connects it to the retained layout topology.
	pub(super) fn add_element(
		&mut self,
		parent: Option<Id>,
		parent_path: usize,
		name: &'static str,
		element: ConcreteElement,
	) -> (Id, usize) {
		let path = self.scope_path(parent, parent_path, name);
		let id = self.id_for_path(path);

		if self.element_indices.contains_key(&id) {
			return (id, path);
		}

		self.element_indices.insert(id, self.elements.len());
		self.revision += 1;
		self.clip_revision = self.revision;
		self.appearance_revision = self.revision;
		self.elements.push(IdedElement {
			id,
			element,
			path,
			revision: self.revision,
		});

		let index = self.elements.len() - 1;
		let parent_index = parent.map(|parent| self.element_indices[&parent]);
		self.parents.push(parent_index);
		self.children.push(Vec::new());
		if let Some(parent_index) = parent_index {
			self.relations.push((self.elements[parent_index].id, id));
			self.children[parent_index].push(index);
		}

		(id, path)
	}

	/// Moves an element under another parent as its last child.
	///
	/// The element keeps its id, path, and properties. Returns false when either
	/// id is unknown or `parent` is the element itself or one of its descendants.
	pub(super) fn reparent(&mut self, child: Id, parent: Id) -> bool {
		let (Some(&child_index), Some(&parent_index)) = (self.element_indices.get(&child), self.element_indices.get(&parent))
		else {
			return false;
		};
		let mut ancestor = Some(parent_index);
		while let Some(current) = ancestor {
			if current == child_index {
				return false;
			}
			ancestor = self.parents[current];
		}
		if self.parents[child_index] == Some(parent_index) {
			return true;
		}
		if let Some(previous) = self.parents[child_index] {
			self.children[previous].retain(|&sibling| sibling != child_index);
			self.relations.retain(|&(_, candidate)| candidate != child);
		}
		self.parents[child_index] = Some(parent_index);
		self.children[parent_index].push(child_index);
		self.relations.push((parent, child));
		self.revision += 1;
		self.clip_revision = self.revision;
		self.appearance_revision = self.revision;
		true
	}

	/// Invalidates the changed node's measurement and any affected inherited appearance.
	pub(super) fn update_element(&mut self, id: Id, update: impl FnOnce(&mut Primitives) -> bool) -> bool {
		let Some(&index) = self.element_indices.get(&id) else {
			return false;
		};
		let element = &mut self.elements[index];
		let primitive = &mut element.element.primitive;
		let clip = clip_inputs(primitive);
		let opacity = primitive.visual().opacity;
		let old_clip_revision = self.clip_revision;
		let old_appearance_revision = self.appearance_revision;
		// Invalidate before application code runs, including when a callback unwinds.
		self.revision += 1;
		element.revision = self.revision;
		self.clip_revision = self.revision;
		self.appearance_revision = self.revision;
		let updated = update(primitive);
		if clip == clip_inputs(primitive) {
			self.clip_revision = old_clip_revision;
			if opacity == primitive.visual().opacity {
				self.appearance_revision = old_appearance_revision;
			}
		}
		updated
	}

	pub(super) fn element(&self, id: Id) -> Option<&IdedElement> {
		let index = *self.element_indices.get(&id)?;
		self.elements.get(index)
	}

	/// Removes the scope and returns its identities for runtime cleanup.
	pub(super) fn remove_scope(&mut self, scope: usize) -> HashSet<Id> {
		if scope == 0 {
			return HashSet::new();
		}

		let mut removed = HashSet::new();
		self.elements.retain(|element| {
			// Scope ownership follows declaration paths, never the current visual parent.
			let mut path = element.path;
			while path != 0 && path != scope {
				path = self.paths[path].0;
			}
			let should_remove = path == scope;
			if should_remove {
				removed.insert(element.id);
			}
			!should_remove
		});

		if removed.is_empty() {
			return HashSet::new();
		}
		self.revision += 1;
		self.clip_revision = self.revision;
		self.appearance_revision = self.revision;

		self.relations
			.retain(|(parent, child)| !removed.contains(parent) && !removed.contains(child));
		self.rebuild_element_indices();
		removed
	}

	/// Restores index-based links after removal compacts the live elements.
	pub(super) fn rebuild_element_indices(&mut self) {
		self.element_indices.clear();
		for (index, element) in self.elements.iter().enumerate() {
			self.element_indices.insert(element.id, index);
		}
		self.parents.clear();
		self.parents.resize(self.elements.len(), None);
		self.children.resize_with(self.elements.len(), Vec::new);
		for children in &mut self.children {
			children.clear();
		}
		for &(parent, child) in &self.relations {
			let parent = self.element_indices[&parent];
			let child = self.element_indices[&child];
			self.parents[child] = Some(parent);
			self.children[parent].push(child);
		}
	}
}
