// Element ids are already well-mixed hashes, so the fast hasher is enough for every id-keyed map.
use utils::hash::{HashMap, HashSet};

use super::{ConcreteElement, Id, IdedElement, LayoutElement, engine::properties::Spares};
use crate::ui::{
	components::{
		container::ContainerProperties, curve::CurveSegment, path::FillRule, text::TextSettings, text_field::TextFieldSettings,
	},
	flow::{self, FlowOutput},
	primitive::{Primitive, Primitives},
	style::{ConcreteLayer, EdgeFeather, Layer},
};

/// The path of the root context. Nothing is declared above it, so it can never be removed.
pub(super) const ROOT_PATH: u64 = 0;

/// Reports whether `path` is `ancestor` or was declared somewhere under it, following `declarations`.
fn is_declared_under(declarations: &HashMap<u64, u64>, mut path: u64, ancestor: u64) -> bool {
	loop {
		if path == ancestor {
			return true;
		}
		match declarations.get(&path) {
			Some(&declared_in) if path != ROOT_PATH => path = declared_in,
			_ => return false,
		}
	}
}

/// Properties that can change placement independently of text measurements.
#[derive(PartialEq)]
enum PlacementInputs {
	Image {
		width: super::Sizing,
		height: super::Sizing,
	},
	Shape(crate::ui::primitive::Shapes),
	Container {
		width: super::Sizing,
		height: super::Sizing,
		depth: super::Depth,
		position: super::Position,
		hit_testable: bool,
		flow: (std::any::TypeId, Option<FlowOutput>),
	},
	Text {
		hit_testable: bool,
	},
	/// Segment edits are paint-only; only the path's size places a curve.
	Curve {
		width: super::Sizing,
		height: super::Sizing,
		hit_testable: bool,
	},
	/// Outline and style edits are paint-only; only the path's size places it.
	Path {
		width: super::Sizing,
		height: super::Sizing,
	},
}

/// Captures paint-independent layout inputs without copying styles or text.
/// Visual transforms never participate in flow placement.
fn placement_inputs(primitive: &Primitives) -> PlacementInputs {
	match primitive {
		Primitives::Container(container) => PlacementInputs::Container {
			width: container.width,
			height: container.height,
			depth: container.depth,
			position: container.position,
			hit_testable: container.hit_testable,
			flow: (
				container.flow.callable_type_id(),
				flow::placement_key(&container.flow).map(|key| key.1),
			),
		},
		Primitives::Text(_) => PlacementInputs::Text { hit_testable: false },
		Primitives::TextField(_) => PlacementInputs::Text { hit_testable: true },
		Primitives::Curve(curve) => PlacementInputs::Curve {
			width: curve.path.width,
			height: curve.path.height,
			hit_testable: curve.hit_width.is_some(),
		},
		Primitives::Image(image) => PlacementInputs::Image {
			width: image.width,
			height: image.height,
		},
		Primitives::Path(path) => PlacementInputs::Path {
			width: path.path.width,
			height: path.path.height,
		},
		Primitives::Shape(shape) => PlacementInputs::Shape(shape.outline()),
	}
}

/// The fixed-size properties that, with style, content, transform, and opacity, fully describe a primitive.
#[derive(PartialEq)]
enum PropertyInputs {
	Container(ContainerProperties),
	Image {
		content: (u64, u64, u32, u32),
		width: super::Sizing,
		height: super::Sizing,
	},
	Text(TextSettings),
	TextField(TextFieldSettings),
	Curve {
		width: super::Sizing,
		height: super::Sizing,
		hit_width: Option<f32>,
	},
	Path {
		content: (u64, u64, FillRule),
		width: super::Sizing,
		height: super::Sizing,
		view_box: Option<[f32; 2]>,
	},
}

/// Captures the properties an edit is compared on, or `None` when a custom flow cannot be compared.
fn property_inputs(primitive: &Primitives) -> Option<PropertyInputs> {
	Some(match primitive {
		Primitives::Container(container) => PropertyInputs::Container(container.properties()?),
		Primitives::Shape(shape) => PropertyInputs::Container(shape.settings.properties()?),
		Primitives::Image(image) => PropertyInputs::Image {
			content: image.content_key(),
			width: image.width,
			height: image.height,
		},
		Primitives::Text(text) => PropertyInputs::Text(*text.settings()),
		Primitives::TextField(text_field) => PropertyInputs::TextField(*text_field.settings()),
		Primitives::Curve(curve) => PropertyInputs::Curve {
			width: curve.path.width,
			height: curve.path.height,
			hit_width: curve.hit_width,
		},
		Primitives::Path(path) => PropertyInputs::Path {
			content: path.content_key(),
			width: path.path.width,
			height: path.path.height,
			view_box: path.view_box,
		},
	})
}

/// Returns the curve segments of a primitive, which are compared by content.
fn curve_segments(primitive: &Primitives) -> &[CurveSegment] {
	match primitive {
		Primitives::Curve(curve) => &curve.path.segments,
		_ => &[],
	}
}

/// Returns only the text inputs that affect intrinsic size.
fn text_measurement_inputs(primitive: &Primitives) -> Option<(&str, f32)> {
	match primitive {
		Primitives::Text(text) => Some((text.content(), text.settings().font_size)),
		Primitives::TextField(text) => Some((text.content(), text.settings().font_size)),
		_ => None,
	}
}

/// Identifies flow replacements so geometry edits do not rescan the tree for custom callables.
fn flow_type(primitive: &Primitives) -> Option<std::any::TypeId> {
	match primitive {
		Primitives::Container(container) => Some(container.flow.callable_type_id()),
		_ => None,
	}
}

// Clip inheritance depends on these container properties, independently of paint color.
/// The inputs whose change rebuilds clipping and hit geometry. A sector belongs here: it reshapes what the
/// pointer can hit without moving the element.
fn clip_inputs(
	primitive: &Primitives,
) -> Option<(
	bool,
	bool,
	f32,
	f32,
	Option<crate::ui::components::container::Sector>,
	Option<EdgeFeather>,
)> {
	let Primitives::Container(container) = primitive else {
		return None;
	};
	Some((
		container.clip,
		matches!(container.depth, super::Depth::Absolute(_)),
		container.corner_radius,
		container.corner_exponent,
		container.sector,
		container
			.style
			.layers()
			.iter()
			.map(Layer::feather)
			.find(|feather| !feather.is_none()),
	))
}

/// The `RetainedTree` struct owns the live UI elements and the topology layout walks.
///
/// Components write to it while their task is polled: declarations and edits reach it through the poll state the
/// engine lends them; see [`super::engine::properties`]. Element ids are computed by the contexts (see
/// [`super::context::ElementKey`]), so the tree only records what it needs to follow declaration ancestry and to keep
/// a compact render order.
#[derive(Default)]
pub(super) struct RetainedTree {
	pub(super) elements: Vec<IdedElement>,
	pub(super) element_indices: HashMap<Id, usize>,
	pub(super) relations: Vec<(Id, Id)>,
	/// Dense links follow `elements`; only external identity lookup needs hashing.
	/// Spare child lists keep their capacity after a scope closes.
	pub(super) children: Vec<Vec<usize>>,
	pub(super) parents: Vec<Option<usize>>,
	/// The path each declared element or component scope was declared in, by its own path.
	///
	/// Scope removal follows these links, so a visually reparented element still belongs to the context that
	/// declared it. Entries outlive their elements so a context can declare again after a removal.
	declarations: HashMap<u64, u64>,
	/// Elements created in the current frame, to detect a key declared twice under the same parent.
	declared: HashSet<Id>,
	/// The compact render order of every id this tree has held. A remounted element keeps its order.
	serials: HashMap<Id, u32>,
	next_serial: u32,
	/// The last id given to an image's or a path's contents; see [`Self::add_element`].
	next_content_id: u64,
	/// Reused during scope cleanup; the caller consumes these IDs before the next removal.
	removed: HashSet<Id>,
	/// The heap buffers of removed elements, which new elements take instead of allocating.
	spares: Spares,
	/// Advances on every structural or property change so consumers can retain derived state.
	revision: u64,
	/// Advances when a mutation may change element positions, sizes, or hit participation.
	pub(super) placement_revision: u64,
	/// Roots whose visual transforms changed since the last evaluation.
	pub(super) transform_changes: Vec<usize>,
	/// Custom flows may observe external state after edits other than visual transforms.
	pub(super) non_transform_revision: u64,
	/// Text edits need a size comparison before placement can be reused.
	/// Structural edits invalidate placement before these indices can be read.
	pub(super) text_changes: Vec<usize>,
	/// Reused to compare text edits without allocating for visual-only updates.
	text_before: String,
	/// Reused to compare style layers and curve segments across an edit.
	style_before: Vec<ConcreteLayer>,
	segments_before: Vec<CurveSegment>,
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
		Self {
			elements: Vec::with_capacity(ELEMENT_CAPACITY),
			element_indices: HashMap::with_capacity_and_hasher(ELEMENT_CAPACITY, Default::default()),
			relations: Vec::with_capacity(ELEMENT_CAPACITY),
			children: Vec::with_capacity(ELEMENT_CAPACITY),
			parents: Vec::with_capacity(ELEMENT_CAPACITY),
			declarations: HashMap::with_capacity_and_hasher(ELEMENT_CAPACITY, Default::default()),
			declared: HashSet::with_capacity_and_hasher(ELEMENT_CAPACITY, Default::default()),
			serials: HashMap::with_capacity_and_hasher(ELEMENT_CAPACITY, Default::default()),
			removed: HashSet::with_capacity_and_hasher(ELEMENT_CAPACITY, Default::default()),
			text_changes: Vec::with_capacity(ELEMENT_CAPACITY),
			..Self::default()
		}
	}

	pub(super) fn begin_frame(&mut self) {
		self.declared.clear();
	}

	/// Returns a value that changes whenever elements are added, removed, or mutated.
	///
	/// Layout and render data derived from one revision stay valid until it changes.
	pub(super) fn revision(&self) -> u64 {
		self.revision
	}

	/// Records that the component scope `path` was declared in `declared_in`, so removing an ancestor ends it.
	pub(super) fn declare_scope(&mut self, path: u64, declared_in: u64) {
		self.declarations.insert(path, declared_in);
	}

	/// Reports whether `path` is `ancestor` or was declared somewhere under it.
	///
	/// Declaration ancestry is what scope removal follows, so a visually reparented
	/// element still belongs to the context that declared it.
	pub(super) fn path_is_under(&self, path: u64, ancestor: u64) -> bool {
		is_declared_under(&self.declarations, path, ancestor)
	}

	/// Adds a declaration once and connects it to the retained layout topology.
	///
	/// A declaration of an id the tree already holds keeps the existing element and its properties. Declaring the
	/// same id twice in one frame, or under a parent the tree does not hold, is logged and ignored.
	///
	/// `create` builds the element from a fresh content id and the storage removed elements left, and runs only when
	/// the declaration creates it. Returns the new element with that storage, so its initial properties are written in
	/// place; a new element needs no change tracking, since creating it invalidated everything it affects.
	pub(super) fn add_element(
		&mut self,
		parent: Option<Id>,
		declared_in: u64,
		id: Id,
		create: impl FnOnce(u64, &mut Spares) -> Primitives,
	) -> Option<(&mut Primitives, &mut Spares)> {
		let parent_index = match parent.map(|parent| self.element_indices.get(&parent).copied()) {
			Some(Some(index)) => Some(index),
			Some(None) => {
				// Writes from one task land in order, so this parent was removed before its child was declared.
				log::error!(
					"A UI element was declared under a parent that no longer exists. The most likely cause is declaring an element from a context whose element was removed."
				);
				return None;
			}
			None => None,
		};
		if !self.declared.insert(id) {
			log::error!(
				"A UI element key was declared twice under the same parent in one frame. The most likely cause is declaring siblings in a loop with one key; give each one a distinct key, such as `(\"row\", index)`."
			);
			debug_assert!(false, "UI element {id} was declared twice under the same parent in one frame");
			return None;
		}
		self.declarations.insert(id.get(), declared_in);
		let std::collections::hash_map::Entry::Vacant(entry) = self.element_indices.entry(id) else {
			return None;
		};
		entry.insert(self.elements.len());

		let next_serial = &mut self.next_serial;
		let serial = *self.serials.entry(id).or_insert_with(|| {
			*next_serial += 1;
			*next_serial
		});
		self.revision += 1;
		self.non_transform_revision = self.revision;
		self.placement_revision = self.revision;
		self.flow_revision = self.revision;
		self.clip_revision = self.revision;
		self.appearance_revision = self.revision;
		self.next_content_id += 1;
		self.elements.push(IdedElement {
			id,
			element: ConcreteElement {
				primitive: create(self.next_content_id, &mut self.spares),
			},
			serial,
			revision: self.revision,
		});

		let index = self.elements.len() - 1;
		self.parents.push(parent_index);
		if index == self.children.len() {
			self.children.push(Vec::new());
		}
		debug_assert!(self.children[index].is_empty());
		if let Some(parent_index) = parent_index {
			self.relations.push((self.elements[parent_index].id, id));
			self.children[parent_index].push(index);
		}
		Some((&mut self.elements[index].element.primitive, &mut self.spares))
	}

	/// Moves an element under another parent as its last child.
	///
	/// The element keeps its id, declaration path, and properties. An unknown id, or a `parent` that is the element
	/// itself or one of its descendants, is logged and changes nothing.
	pub(super) fn reparent(&mut self, child: Id, parent: Id) {
		let (Some(&child_index), Some(&parent_index)) = (self.element_indices.get(&child), self.element_indices.get(&parent))
		else {
			log::error!(
				"A UI element could not be reparented because it or its new parent does not exist. The most likely cause is an element that was removed before the reparent was applied."
			);
			return;
		};
		let mut ancestor = Some(parent_index);
		while let Some(current) = ancestor {
			if current == child_index {
				log::error!(
					"A UI element could not be moved under itself or one of its descendants. The most likely cause is adopting an ancestor of the adopting element."
				);
				return;
			}
			ancestor = self.parents[current];
		}
		if self.parents[child_index] == Some(parent_index) {
			return;
		}
		if let Some(previous) = self.parents[child_index] {
			self.children[previous].retain(|&sibling| sibling != child_index);
			self.relations.retain(|&(_, candidate)| candidate != child);
		}
		self.parents[child_index] = Some(parent_index);
		self.children[parent_index].push(child_index);
		self.relations.push((parent, child));
		self.revision += 1;
		self.non_transform_revision = self.revision;
		self.placement_revision = self.revision;
		self.flow_revision = self.revision;
		self.clip_revision = self.revision;
		self.appearance_revision = self.revision;
	}

	/// Invalidates the changed node's measurement and any affected inherited appearance.
	///
	/// `update` writes the element in place with the storage removed elements left. Returns what it returned, or
	/// `None` when the tree holds no element `id`.
	pub(super) fn update_element(&mut self, id: Id, update: impl FnOnce(&mut Primitives, &mut Spares) -> bool) -> Option<bool> {
		let index = *self.element_indices.get(&id)?;
		let element = &mut self.elements[index];
		let primitive = &mut element.element.primitive;
		let placement = placement_inputs(primitive);
		let text_before = text_measurement_inputs(primitive).map(|(content, size)| {
			self.text_before.clear();
			self.text_before.push_str(content);
			size
		});
		let transform = *primitive.transform();
		let flow = flow_type(primitive);
		let clip = clip_inputs(primitive);
		let opacity = primitive.visual().opacity;
		let properties = property_inputs(primitive);
		self.style_before.clear();
		self.style_before.extend_from_slice(primitive.style().layers());
		self.segments_before.clear();
		self.segments_before.extend_from_slice(curve_segments(primitive));
		let old_revision = self.revision;
		let old_element_revision = element.revision;
		let old_clip_revision = self.clip_revision;
		let old_appearance_revision = self.appearance_revision;
		let old_placement_revision = self.placement_revision;
		let old_non_transform_revision = self.non_transform_revision;
		let old_flow_revision = self.flow_revision;
		// Invalidate before application code runs, including when a callback unwinds.
		self.revision += 1;
		self.non_transform_revision = self.revision;
		element.revision = self.revision;
		self.placement_revision = self.revision;
		self.flow_revision = self.revision;
		self.clip_revision = self.revision;
		self.appearance_revision = self.revision;
		let updated = update(primitive, &mut self.spares);
		// An edit that wrote the values already present changes nothing, so every revision stays put and consumers
		// keep their retained renders.
		if properties.is_some()
			&& properties == property_inputs(primitive)
			&& transform == *primitive.transform()
			&& opacity == primitive.visual().opacity
			&& text_measurement_inputs(primitive).map_or(true, |(content, size)| {
				text_before == Some(size) && content == self.text_before
			}) && self.style_before.as_slice() == primitive.style().layers()
			&& self.segments_before.as_slice() == curve_segments(primitive)
		{
			self.revision = old_revision;
			element.revision = old_element_revision;
			self.non_transform_revision = old_non_transform_revision;
			self.placement_revision = old_placement_revision;
			self.flow_revision = old_flow_revision;
			self.clip_revision = old_clip_revision;
			self.appearance_revision = old_appearance_revision;
			return Some(updated);
		}
		if transform != *primitive.transform() && !self.transform_changes.contains(&index) {
			self.transform_changes.push(index);
		}
		if flow == flow_type(primitive) {
			self.flow_revision = old_flow_revision;
		}
		if placement == placement_inputs(primitive) {
			self.placement_revision = old_placement_revision;
			if transform != *primitive.transform() {
				self.non_transform_revision = old_non_transform_revision;
			}
			if text_measurement_inputs(primitive)
				.is_some_and(|(content, size)| text_before != Some(size) || content != self.text_before)
			{
				self.text_changes.push(index);
			}
		}
		if clip == clip_inputs(primitive) && !matches!(primitive, Primitives::Curve(curve) if curve.hit_width().is_some()) {
			self.clip_revision = old_clip_revision;
			if opacity == primitive.visual().opacity {
				self.appearance_revision = old_appearance_revision;
			}
		}
		Some(updated)
	}

	pub(super) fn element(&self, id: Id) -> Option<&IdedElement> {
		let index = *self.element_indices.get(&id)?;
		self.elements.get(index)
	}

	/// Removes the scope and lends its identities to runtime cleanup without reallocating the set.
	pub(super) fn remove_scope(&mut self, scope: u64) -> &HashSet<Id> {
		self.removed.clear();
		if scope == ROOT_PATH {
			return &self.removed;
		}

		let Self {
			elements,
			declarations,
			removed,
			declared,
			spares,
			..
		} = self;
		elements.retain_mut(|element| {
			// Scope ownership follows declaration paths, never the current visual parent.
			let should_remove = is_declared_under(declarations, element.id.get(), scope);
			if should_remove {
				removed.insert(element.id);
				// The same key may be declared again in this frame once its element is gone.
				declared.remove(&element.id);
				spares.recycle(&mut element.element.primitive);
			}
			!should_remove
		});

		if self.removed.is_empty() {
			return &self.removed;
		}
		self.revision += 1;
		self.non_transform_revision = self.revision;
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
	/// Resolves a laid-out element to its tree index without hashing.
	///
	/// A snapshot older than a structural edit can carry a stale index, so the
	/// carried index is trusted only when the element there still has the same ID.
	pub(super) fn index_of(&self, element: &LayoutElement) -> Option<usize> {
		match self.elements.get(element.index) {
			Some(candidate) if candidate.id == element.id => Some(element.index),
			_ => self.element_indices.get(&element.id).copied(),
		}
	}

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
