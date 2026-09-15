use std::{
	borrow::Cow,
	collections::{HashMap, HashSet},
};

use super::{ConcreteElement, Id, IdedElement, PathSegment};
use crate::ui::{
	Transform,
	flow::{self, FlowOutput},
	primitive::{Primitive, Primitives},
	style::{EdgeFeather, Layer},
};

/// Properties that can change placement independently of text measurements.
#[derive(PartialEq)]
enum PlacementInputs {
	Container {
		width: super::Sizing,
		height: super::Sizing,
		depth: super::Depth,
		position: super::Position,
		transform: Transform,
		hit_testable: bool,
		flow: (std::any::TypeId, FlowOutput),
	},
	Text {
		transform: Transform,
		hit_testable: bool,
	},
	/// Segment edits are paint-only; only the path's size and transform place a curve.
	Curve {
		width: super::Sizing,
		height: super::Sizing,
		transform: Transform,
		hit_testable: bool,
	},
}

/// Captures paint-independent layout inputs without copying styles or text.
/// Other primitives conservatively remeasure after edits.
fn placement_inputs(primitive: &Primitives) -> Option<PlacementInputs> {
	Some(match primitive {
		Primitives::Container(container) => PlacementInputs::Container {
			width: container.width,
			height: container.height,
			depth: container.depth,
			position: container.position,
			transform: container.transform,
			hit_testable: container.hit_testable,
			flow: flow::placement_key(&container.flow)?,
		},
		Primitives::Text(text) => PlacementInputs::Text {
			transform: text.transform,
			hit_testable: false,
		},
		Primitives::TextField(text) => PlacementInputs::Text {
			transform: text.transform,
			hit_testable: true,
		},
		Primitives::Curve(curve) => PlacementInputs::Curve {
			width: curve.path.width,
			height: curve.path.height,
			transform: curve.transform,
			hit_testable: curve.hit_width.is_some(),
		},
		_ => return None,
	})
}

/// Identifies flow replacements so geometry edits do not rescan the tree for custom callables.
fn flow_type(primitive: &Primitives) -> Option<std::any::TypeId> {
	match primitive {
		Primitives::Container(container) => Some(container.flow.callable_type_id()),
		_ => None,
	}
}

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
	/// Spare child lists keep their capacity after a scope closes.
	pub(super) children: Vec<Vec<usize>>,
	pub(super) parents: Vec<Option<usize>>,
	path_counts: HashMap<(Option<Id>, Cow<'static, str>), u32>,
	path_ids: HashMap<(usize, PathSegment), usize>,
	/// Interned paths retain their original scope ancestry even after visual reparenting.
	paths: Vec<(usize, Option<Id>)>,
	/// Reused during scope cleanup; the caller consumes these IDs before the next removal.
	removed: HashSet<Id>,
	next_id: u32,
	/// Advances on every structural or property change so consumers can retain derived state.
	revision: u64,
	/// Advances when a mutation may change element positions, sizes, or hit participation.
	pub(super) placement_revision: u64,
	/// Text edits need a size comparison before placement can be reused.
	/// Structural edits invalidate placement before these indices can be read.
	pub(super) text_changes: Vec<usize>,
	/// Advances when the set of flow types may change.
	pub(super) flow_revision: u64,
	/// Structural edits also invalidate clipping, including remounts that reuse IDs.
	pub(super) clip_revision: u64,
	/// Clipping and inherited opacity can change independently of paint color.
	pub(super) appearance_revision: u64,
}

impl RetainedTree {
	pub(super) fn new() -> Self {
		// Reserve a small screen up front; larger screens grow these collections normally.
		const ELEMENT_CAPACITY: usize = 256;
		let mut paths = Vec::with_capacity(ELEMENT_CAPACITY);
		paths.push((0, None));
		Self {
			next_id: 1,
			elements: Vec::with_capacity(ELEMENT_CAPACITY),
			element_indices: HashMap::with_capacity(ELEMENT_CAPACITY),
			relations: Vec::with_capacity(ELEMENT_CAPACITY),
			children: Vec::with_capacity(ELEMENT_CAPACITY),
			parents: Vec::with_capacity(ELEMENT_CAPACITY),
			path_counts: HashMap::with_capacity(ELEMENT_CAPACITY),
			path_ids: HashMap::with_capacity(ELEMENT_CAPACITY),
			paths,
			removed: HashSet::with_capacity(ELEMENT_CAPACITY),
			text_changes: Vec::with_capacity(ELEMENT_CAPACITY),
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
	///
	/// Names are declared once per element, so an owned name is cloned only at that time.
	pub(super) fn scope_path(&mut self, parent: Option<Id>, parent_path: usize, name: Cow<'static, str>) -> usize {
		let count = self.path_counts.entry((parent, name.clone())).or_insert(0);
		*count += 1;
		let key = (parent_path, PathSegment { name, ordinal: *count });
		*self.path_ids.entry(key).or_insert_with(|| {
			let index = self.paths.len();
			self.paths.push((parent_path, None));
			index
		})
	}

	/// Reports whether `path` is `ancestor` or was declared somewhere under it.
	///
	/// Declaration ancestry is what scope removal follows, so a visually reparented
	/// element still belongs to the context that declared it.
	pub(super) fn path_is_under(&self, mut path: usize, ancestor: usize) -> bool {
		while path != 0 && path != ancestor {
			path = self.paths[path].0;
		}
		path == ancestor
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
		name: Cow<'static, str>,
		element: ConcreteElement,
	) -> (Id, usize) {
		let path = self.scope_path(parent, parent_path, name);
		let id = self.id_for_path(path);

		if self.element_indices.contains_key(&id) {
			return (id, path);
		}

		self.element_indices.insert(id, self.elements.len());
		self.revision += 1;
		self.placement_revision = self.revision;
		self.flow_revision = self.revision;
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
		if index == self.children.len() {
			self.children.push(Vec::new());
		}
		debug_assert!(self.children[index].is_empty());
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
		self.placement_revision = self.revision;
		self.flow_revision = self.revision;
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
		let placement = placement_inputs(primitive);
		let flow = flow_type(primitive);
		let clip = clip_inputs(primitive);
		let opacity = primitive.visual().opacity;
		let old_clip_revision = self.clip_revision;
		let old_appearance_revision = self.appearance_revision;
		let old_placement_revision = self.placement_revision;
		let old_flow_revision = self.flow_revision;
		// Invalidate before application code runs, including when a callback unwinds.
		self.revision += 1;
		element.revision = self.revision;
		self.placement_revision = self.revision;
		self.flow_revision = self.revision;
		self.clip_revision = self.revision;
		self.appearance_revision = self.revision;
		let updated = update(primitive);
		if flow == flow_type(primitive) {
			self.flow_revision = old_flow_revision;
		}
		if placement.is_some() && placement == placement_inputs(primitive) {
			self.placement_revision = old_placement_revision;
			if matches!(placement, Some(PlacementInputs::Text { .. })) {
				self.text_changes.push(index);
			}
		}
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

	/// Removes the scope and lends its identities to runtime cleanup without reallocating the set.
	pub(super) fn remove_scope(&mut self, scope: usize) -> &HashSet<Id> {
		self.removed.clear();
		if scope == 0 {
			return &self.removed;
		}

		let Self {
			elements,
			paths,
			removed,
			..
		} = self;
		elements.retain(|element| {
			// Scope ownership follows declaration paths, never the current visual parent.
			let mut path = element.path;
			while path != 0 && path != scope {
				path = paths[path].0;
			}
			let should_remove = path == scope;
			if should_remove {
				removed.insert(element.id);
			}
			!should_remove
		});

		if self.removed.is_empty() {
			return &self.removed;
		}
		self.revision += 1;
		self.placement_revision = self.revision;
		self.flow_revision = self.revision;
		self.clip_revision = self.revision;
		self.appearance_revision = self.revision;

		self.relations
			.retain(|(parent, child)| !self.removed.contains(parent) && !self.removed.contains(child));
		self.rebuild_element_indices();
		&self.removed
	}

	/// Restores index-based links after removal compacts the live elements.
	pub(super) fn rebuild_element_indices(&mut self) {
		self.element_indices.clear();
		for (index, element) in self.elements.iter().enumerate() {
			self.element_indices.insert(element.id, index);
		}
		self.parents.clear();
		self.parents.resize(self.elements.len(), None);
		// Keep cleared child lists for later mounts, including indices beyond the live tree.
		self.children
			.resize_with(self.children.len().max(self.elements.len()), Vec::new);
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
