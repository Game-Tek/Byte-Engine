// Element ids are already well-mixed hashes, so the fast hasher is enough for every id-keyed map.
use utils::hash::{HashMap, HashSet};

use super::{Id, IdedElement, LayoutElement, engine::properties::Spares};
use crate::ui::{
	components::{container::ContainerProperties, curve::CurveSegment, path::FillRule, text::TextSettings},
	flow::{self, FlowOutput},
	primitive::Primitives,
	style::{ConcreteLayer, EdgeFeather},
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
	/// Style edits are paint-only. A path with a view box is ink a centering flow lines up, alone or in a group of
	/// paths, so its outline places it too.
	Path {
		width: super::Sizing,
		height: super::Sizing,
		view_box: Option<[f32; 2]>,
		outline: Option<(u64, u64, FillRule)>,
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
		Primitives::Text(text) => PlacementInputs::Text {
			hit_testable: text.editable,
		},
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
			view_box: path.view_box,
			outline: path.view_box.map(|_| path.content_key()),
		},
	}
}

/// Captures a container's display size, which resizes its visual box without moving placement.
fn display_inputs(primitive: &Primitives) -> Option<(super::Sizing, super::Sizing)> {
	match primitive {
		Primitives::Container(container) => Some((container.display_width, container.display_height)),
		_ => None,
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
		Primitives::Image(image) => PropertyInputs::Image {
			content: image.content_key(),
			width: image.width,
			height: image.height,
		},
		Primitives::Text(text) => PropertyInputs::Text(*text.settings()),
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
fn text_measurement_inputs(primitive: &Primitives) -> Option<(&str, f32, Option<&'static str>)> {
	match primitive {
		Primitives::Text(text) => Some((text.content(), text.settings().font_size, text.settings().font)),
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
	element: &IdedElement,
) -> Option<(
	bool,
	bool,
	[f32; 4],
	f32,
	Option<crate::ui::components::container::Sector>,
	Option<EdgeFeather>,
)> {
	let Primitives::Container(container) = &element.primitive else {
		return None;
	};
	Some((
		container.clip,
		matches!(container.depth, super::Depth::Absolute(_)),
		container.corner_radii,
		container.corner_exponent,
		container.sector,
		super::engine::first_layer_feather(element.style.layers()),
	))
}

/// The `TreeRevisions` struct numbers the tree's mutations per invalidation class, so derived state such as the
/// retained layout and render can tell which of its inputs changed.
///
/// Each class holds the number of the last mutation that may have changed it. Consumers keep a copy from when they
/// were built and compare the classes they depend on. Structural edits change every class; see [`Self::advance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct TreeRevisions {
	/// The last mutation of any kind, structural or property.
	pub(super) any: u64,
	/// Advances when a mutation may change element positions, sizes, or hit participation.
	pub(super) placement: u64,
	/// Custom flows may observe external state after edits other than visual ones: transforms and display sizes.
	pub(super) non_visual: u64,
	/// Advances when the set of flow types may change.
	pub(super) flow: u64,
	/// Structural edits also invalidate clipping, including remounts that reuse IDs.
	pub(super) clip: u64,
	/// Clipping and inherited opacity can change independently of paint color.
	pub(super) appearance: u64,
}

impl TreeRevisions {
	/// Numbers a new mutation that may change every class, as a structural edit does.
	pub(super) fn advance(&mut self) {
		let any = self.any + 1;
		*self = Self {
			any,
			placement: any,
			non_visual: any,
			flow: any,
			clip: any,
			appearance: any,
		};
	}
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
	/// Dense links follow `elements`; only external identity lookup needs hashing.
	/// Spare child lists keep their capacity after a scope closes.
	pub(super) children: Vec<Vec<usize>>,
	// usize::MAX marks a root; vector indices cannot reach it. Parent lookup keeps the sentinel private.
	pub(super) parents: Vec<usize>,
	/// The path each declared element or component scope was declared in, by its own path.
	///
	/// Scope removal follows these links, so a visually reparented element still belongs to the context that
	/// declared it. Entries outlive their elements so a context can declare again after a removal.
	declarations: HashMap<u64, u64>,
	/// Elements created in the current frame, to detect a key declared twice under the same parent.
	declared: HashSet<Id>,
	/// The rank a collection gives each of its items' scopes, by scope path. Children declared directly in a ranked
	/// scope keep rank order among their parent's ranked children; see [`Self::set_ranks`].
	ranks: HashMap<u64, u32>,
	/// Reused by [`Self::sort_ranked`]: each ranked child's slot among its siblings, and its rank and index.
	rank_slots: Vec<usize>,
	rank_order: Vec<(u32, usize)>,
	/// The compact render order of every id this tree has held. A remounted element keeps its order.
	serials: HashMap<Id, u32>,
	next_serial: u32,
	/// The last id given to an image's or a path's contents; see [`Self::add_element`].
	next_content_id: u64,
	/// Reused during scope cleanup; the caller consumes these IDs before the next removal.
	removed: HashSet<Id>,
	/// The heap buffers of removed elements, which new elements take instead of allocating.
	spares: Spares,
	/// Advances per invalidation class so consumers can retain derived state.
	pub(super) revisions: TreeRevisions,
	/// Roots whose visual transforms or display sizes changed since the last evaluation. Neither moves placement, so
	/// the evaluation only recomputes these subtrees' visual boxes.
	pub(super) visual_changes: Vec<usize>,
	/// Whether each tree index is listed in `visual_changes`, so membership needs no search.
	visual_changed: Vec<bool>,
	/// Maps each index before a scope removal compacted the elements to its index after, or `usize::MAX`.
	remap: Vec<usize>,
	/// Text edits need a size comparison before placement can be reused.
	/// Structural edits invalidate placement before these indices can be read.
	pub(super) text_changes: Vec<usize>,
	/// Reused to compare text edits without allocating for visual-only updates.
	text_before: String,
	/// Reused to compare style layers and curve segments across an edit.
	style_before: Vec<ConcreteLayer>,
	segments_before: Vec<CurveSegment>,
}

impl RetainedTree {
	pub(super) fn new() -> Self {
		// Reserve a small screen up front; larger screens grow these collections normally.
		const ELEMENT_CAPACITY: usize = 256;
		Self {
			elements: Vec::with_capacity(ELEMENT_CAPACITY),
			element_indices: HashMap::with_capacity_and_hasher(ELEMENT_CAPACITY, Default::default()),
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
		self.revisions.any
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
	) -> Option<(&mut IdedElement, &mut Spares)> {
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
		self.revisions.advance();
		self.next_content_id += 1;
		let primitive = create(self.next_content_id, &mut self.spares);
		self.elements
			.push(IdedElement::new(id, serial, self.revisions.any, primitive));

		let index = self.elements.len() - 1;
		self.parents.push(parent_index.unwrap_or(usize::MAX));
		if index == self.children.len() {
			self.children.push(Vec::new());
		}
		debug_assert!(self.children[index].is_empty());
		if let Some(parent_index) = parent_index {
			self.children[parent_index].push(index);
			// A collection's item usually arrives in order, after the ranked siblings before it; only one that doesn't
			// pays for a sort.
			if let Some(rank) = self.rank_of(index) {
				let out_of_order = self.children[parent_index]
					.iter()
					.rev()
					.skip(1)
					.find_map(|&sibling| self.rank_of(sibling))
					.is_some_and(|previous| previous > rank);
				if out_of_order {
					self.sort_ranked(parent_index);
				}
			}
		}
		Some((&mut self.elements[index], &mut self.spares))
	}

	/// Gives each scope path in `ranks` its rank, and reorders `parent`'s ranked children to match, as a collection
	/// does when its items change order. See [Collections](/docs/reference/gui#collections).
	pub(super) fn set_ranks(&mut self, parent: Id, ranks: impl IntoIterator<Item = (u64, u32)>) {
		self.ranks.extend(ranks);
		if let Some(&parent_index) = self.element_indices.get(&parent) {
			self.sort_ranked(parent_index);
		}
	}

	/// The rank of the scope the element at `index` was declared in, if a collection ranks it.
	fn rank_of(&self, index: usize) -> Option<u32> {
		let declared_in = self.declarations.get(&self.elements[index].id.get())?;
		self.ranks.get(declared_in).copied()
	}

	/// Puts the ranked children of the element at `parent_index` in rank order, in the slots ranked children already
	/// hold, so its other children keep their places. Children of one rank keep their order.
	fn sort_ranked(&mut self, parent_index: usize) {
		let (mut slots, mut order) = (std::mem::take(&mut self.rank_slots), std::mem::take(&mut self.rank_order));
		slots.clear();
		order.clear();
		for (slot, &child) in self.children[parent_index].iter().enumerate() {
			if let Some(rank) = self.rank_of(child) {
				slots.push(slot);
				order.push((rank, child));
			}
		}
		if !order.is_sorted_by_key(|&(rank, _)| rank) {
			order.sort_by_key(|&(rank, _)| rank);
			for (&slot, &(_, child)) in slots.iter().zip(&order) {
				self.children[parent_index][slot] = child;
			}
			self.revisions.advance();
		}
		(self.rank_slots, self.rank_order) = (slots, order);
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
		if self.lineage(parent_index).any(|ancestor| ancestor == child_index) {
			log::error!(
				"A UI element could not be moved under itself or one of its descendants. The most likely cause is adopting an ancestor of the adopting element."
			);
			return;
		}
		if self.parent(child_index) == Some(parent_index) {
			return;
		}
		if let Some(previous) = self.parent(child_index) {
			self.children[previous].retain(|&sibling| sibling != child_index);
		}
		self.parents[child_index] = parent_index;
		self.children[parent_index].push(child_index);
		self.revisions.advance();
	}

	/// Returns an element's parent index, or `None` for a root.
	pub(super) fn parent(&self, index: usize) -> Option<usize> {
		let parent = self.parents[index];
		(parent != usize::MAX).then_some(parent)
	}

	/// Walks from the element at `index` up through its visual ancestors, starting with the element itself.
	pub(super) fn lineage(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
		std::iter::successors(Some(index), |&index| self.parent(index))
	}

	/// Walks from `id` up through the ids of its visual ancestors, starting with `id` itself.
	///
	/// Events bubble along this chain, and hover changes compare two of them. An id the tree does not hold yields
	/// only itself.
	pub(super) fn ancestors(&self, id: Id) -> impl Iterator<Item = Id> + '_ {
		let parent = self.element_indices.get(&id).and_then(|&index| self.parent(index));
		std::iter::once(id).chain(
			parent
				.into_iter()
				.flat_map(|parent| self.lineage(parent))
				.map(|index| self.elements[index].id),
		)
	}

	/// Returns the absolute-depth container that starts the focus layer `id` is in: the nearest one among `id` and its
	/// visual ancestors, or `None` for the base layer.
	pub(super) fn layer_of(&self, id: Id) -> Option<Id> {
		let index = *self.element_indices.get(&id)?;
		self.lineage(index)
			.find(|&index| {
				matches!(
					&self.elements[index].primitive,
					Primitives::Container(container) if matches!(container.depth, super::Depth::Absolute(_))
				)
			})
			.map(|index| self.elements[index].id)
	}

	/// Returns the nearest hit-testable visual ancestor of `id`, such as the well around a text field: the surface a
	/// press on the element's surroundings lands on, which keeps the element's focus.
	pub(super) fn holder_of(&self, id: Id) -> Option<Id> {
		let parent = self.parent(*self.element_indices.get(&id)?)?;
		self.lineage(parent)
			.find(|&index| self.elements[index].primitive.hit_testable())
			.map(|index| self.elements[index].id)
	}

	/// Reports whether the visual transform or display size of the element at `index` changed since the last
	/// evaluation.
	pub(super) fn visual_changed(&self, index: usize) -> bool {
		self.visual_changed.get(index).copied().unwrap_or(false)
	}

	/// Forgets the visual changes the last evaluation consumed, keeping the storage.
	pub(super) fn clear_visual_changes(&mut self) {
		for &index in &self.visual_changes {
			self.visual_changed[index] = false;
		}
		self.visual_changes.clear();
	}

	/// Invalidates the changed node's measurement and any affected inherited appearance.
	///
	/// `update` writes the element in place with the storage removed elements left. Returns what it returned, or
	/// `None` when the tree holds no element `id`.
	pub(super) fn update_element(
		&mut self,
		id: Id,
		update: impl FnOnce(&mut IdedElement, &mut Spares) -> bool,
	) -> Option<bool> {
		let index = *self.element_indices.get(&id)?;
		let element = &mut self.elements[index];
		// Snapshot every input a revision class depends on, so the classes the edit left alone keep their numbers.
		let placement = placement_inputs(&element.primitive);
		let text_before = text_measurement_inputs(&element.primitive).map(|(content, size, font)| {
			self.text_before.clear();
			self.text_before.push_str(content);
			(size, font)
		});
		let transform = element.transform;
		let display = display_inputs(&element.primitive);
		let flow = flow_type(&element.primitive);
		let clip = clip_inputs(element);
		let opacity = element.opacity;
		let properties = property_inputs(&element.primitive);
		self.style_before.clear();
		self.style_before.extend_from_slice(element.style.layers());
		self.segments_before.clear();
		self.segments_before.extend_from_slice(curve_segments(&element.primitive));
		let before = self.revisions;
		let old_element_revision = element.revision;
		// Invalidate before application code runs, including when a callback unwinds.
		self.revisions.advance();
		element.revision = self.revisions.any;
		let updated = update(element, &mut self.spares);
		// An edit that wrote the values already present changes nothing, so every revision stays put and consumers
		// keep their retained renders.
		if properties.is_some()
			&& properties == property_inputs(&element.primitive)
			&& transform == element.transform
			&& opacity == element.opacity
			&& text_measurement_inputs(&element.primitive).map_or(true, |(content, size, font)| {
				text_before == Some((size, font)) && content == self.text_before
			}) && self.style_before.as_slice() == element.style.layers()
			&& self.segments_before.as_slice() == curve_segments(&element.primitive)
		{
			self.revisions = before;
			element.revision = old_element_revision;
			return Some(updated);
		}
		// A transform and a display size only change the visual box, so both replay the visual pass alone.
		let visual = transform != element.transform || display != display_inputs(&element.primitive);
		if visual {
			if index >= self.visual_changed.len() {
				self.visual_changed.resize(index + 1, false);
			}
			// Each root is listed once however often it is edited before the next evaluation.
			if !std::mem::replace(&mut self.visual_changed[index], true) {
				self.visual_changes.push(index);
			}
		}
		if flow == flow_type(&element.primitive) {
			self.revisions.flow = before.flow;
		}
		if placement == placement_inputs(&element.primitive) {
			self.revisions.placement = before.placement;
			if visual {
				self.revisions.non_visual = before.non_visual;
			}
			if text_measurement_inputs(&element.primitive)
				.is_some_and(|(content, size, font)| text_before != Some((size, font)) || content != self.text_before)
			{
				self.text_changes.push(index);
			}
		}
		if clip == clip_inputs(element)
			&& !matches!(&element.primitive, Primitives::Curve(curve) if curve.hit_width().is_some())
		{
			self.revisions.clip = before.clip;
			if opacity == element.opacity {
				self.revisions.appearance = before.appearance;
			}
		}
		Some(updated)
	}

	/// Marks every text changed, as editing each one's font size would, so the next layout measures them again and the
	/// next render redraws them. Call it for a change every text sees at once, such as a new text scale.
	pub(super) fn invalidate_texts(&mut self) {
		// One mutation that may move anything, as a structural edit does, so layout places the whole tree again. Each
		// text's own revision then turns away its cached measurement and its retained render entry.
		self.revisions.advance();
		for element in &mut self.elements {
			if matches!(element.primitive, Primitives::Text(_)) {
				element.revision = self.revisions.any;
			}
		}
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
			remap,
			..
		} = self;
		remap.clear();
		let mut kept = 0;
		elements.retain_mut(|element| {
			// Scope ownership follows declaration paths, never the current visual parent.
			let should_remove = is_declared_under(declarations, element.id.get(), scope);
			if should_remove {
				removed.insert(element.id);
				// The same key may be declared again in this frame once its element is gone.
				declared.remove(&element.id);
				spares.recycle(element);
				remap.push(usize::MAX);
			} else {
				remap.push(kept);
				kept += 1;
			}
			!should_remove
		});

		// A removed collection item's scope, or anything under a removed scope, needs no rank anymore.
		let Self { ranks, declarations, .. } = self;
		ranks.retain(|&path, _| !is_declared_under(declarations, path, scope));
		if self.removed.is_empty() {
			return &self.removed;
		}
		self.revisions.advance();
		// The listed indices are stale now; structural edits replay placement, which recomputes every transform.
		self.clear_visual_changes();
		self.remap_links();
		&self.removed
	}

	/// Rewrites the index links through [`Self::remap`] after a removal compacted the live elements.
	///
	/// Parents and child lists keep their order, so siblings stay in declaration order. An element whose visual parent
	/// was removed without it becomes a root.
	fn remap_links(&mut self) {
		let Self {
			remap,
			element_indices,
			parents,
			children,
			elements,
			..
		} = self;
		let live = |index: usize| Some(remap[index]).filter(|&index| index != usize::MAX);
		element_indices.retain(|_, index| match live(*index) {
			Some(new) => {
				*index = new;
				true
			}
			None => false,
		});
		// Every new index is at most its old one, so walking old indices upward only overwrites entries already read.
		for old in 0..remap.len() {
			let Some(new) = live(old) else {
				// A removed element's list keeps its capacity for later mounts.
				children[old].clear();
				continue;
			};
			let parent = parents[old];
			parents[new] = if parent == usize::MAX { usize::MAX } else { remap[parent] };
			children[old].retain_mut(|child| match live(*child) {
				Some(new) => {
					*child = new;
					true
				}
				None => false,
			});
			// Slots between `new` and `old` hold emptied lists, so the swap moves one of those out of the way.
			children.swap(new, old);
		}
		parents.truncate(elements.len());
	}

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
}
