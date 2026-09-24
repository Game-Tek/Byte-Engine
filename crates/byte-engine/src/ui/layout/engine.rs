//! UI retained tree evaluation, interaction state, and render snapshots.

/// The [`Engine`] struct owns UI evaluation state, text shaping, and pointer
/// interaction across viewports.
///
/// Create an engine with [`Self::new`] or [`Self::with_context`], mount the root
/// component with [`Self::mount`], then call [`Self::evaluate`] and
/// [`Self::render`] for each frame.
/// See the [GUI guide](/docs/develop/gui)
/// for component, event, focus, and render-pass integration.
pub struct Engine<C = ()> {
	viewports: Vec<VirtualViewport>,
	state: EngineState,
	cursor_position: UiPoint,
	is_clicking: bool,
	clicks: Vec<bool>,
	scrolls: Vec<UiVector>,
	/// Released sources waiting for the next layout to resolve their drop targets.
	drops: Vec<DragDrop>,
	/// The captured position the source was last told about.
	dragged: Option<UiPoint>,
	/// The surface under the pointer after the last evaluation, for enter and exit events.
	hovered: Option<Id>,
	/// The ancestor chains of the previous and the new hover target, reused by every hover change.
	hover_chains: [Vec<Id>; 2],
	/// Buffers of render entries that dropped off the lists, which later entries reuse.
	render_spares: Spares,
	key_states: HashMap<Key, bool>,
	key_presses: VecDeque<Key>,
	text_edits: VecDeque<TextEdit>,
	text_system: TextSystem,
	/// The runtime, retained tree, and application context, lent to every task poll; see [`UiPoll`].
	core: UiPoll<C>,
	retained_layout: Option<RetainedLayout>,
	retained_render: Option<RetainedRender>,
	visual_state: Vec<VisualState>,
	visual_state_key: Option<(u64, u64, Size)>,
	measurements: Vec<super::Measurement>,
	rendered_revisions: Vec<u64>,
	/// What each rendered element last looked like, by tree index, so the next render can report damage.
	rendered_footprints: Vec<Option<Footprint>>,
	footprint_scratch: Vec<Option<Footprint>>,
	/// Depth and snapshot offset of each visible element, kept so ordering never allocates.
	depth_order: Vec<(u32, u32)>,
	/// Flow placement stays in layout units while snapshots expose transformed surfaces.
	placement: Vec<LayoutElement>,
	hit_curves: HashMap<Id, crate::ui::components::curve::FlattenedCurve>,
	transforms: Vec<Affine2>,
	transform_work: Vec<usize>,
	placement_indices: Vec<usize>,
	/// Placed elements inside the subtrees moved since the retained layout, parents before children.
	dirty: Vec<usize>,
	/// The layout revision that last moved each tree index, so a render walk can skip the rest.
	dirty_stamps: Vec<u64>,
	/// Each tree index's entry in the retained hit index, or `u32::MAX` when it has none.
	hit_offsets: Vec<u32>,
	/// The revision the next changed render receives. Each engine numbers its own renders.
	next_render_revision: u64,
}

/// The `RetainedLayout` struct keeps the last computed layout so unchanged trees skip evaluation.
struct RetainedLayout {
	/// Last evaluated mutation; `revision` advances only when snapshot geometry changes.
	tree_revision: u64,
	placement_revision: u64,
	non_transform_revision: u64,
	flow_revision: u64,
	has_custom_flows: bool,
	clip_revision: u64,
	revision: u64,
	/// The layout this one differs from by visual transforms inside the engine's dirty roots alone.
	transformed_from: Option<u64>,
	size: Size,
	elements: Vec<LayoutElement>,
	relations: Vec<(Id, Id)>,
	acceleration: MouseClickAcceleration,
}

/// Lends a host the retained layout together with the engine's cursor.
///
/// It takes the two fields rather than the engine, so input routing can hit-test while it keeps changing the
/// engine's other state.
fn snapshot_of<'a>(retained: &'a RetainedLayout, cursor: &'a mut Option<Id>) -> Snapshot<'a> {
	Snapshot {
		elements: &retained.elements,
		relations: &retained.relations,
		acceleration: &retained.acceleration,
		cursor,
		size: retained.size,
	}
}

/// The `RetainedRender` struct keeps the last render so unchanged trees report the same revision.
struct RetainedRender {
	tree_revision: u64,
	placement_revision: u64,
	clip_revision: u64,
	appearance_revision: u64,
	visible: Vec<LayoutElement>,
	layout_revision: u64,
	size: Size,
	render: Render,
}

/// The `Footprint` struct records everything that decides one element's pixels.
///
/// Two equal footprints draw identically, so a render only damages elements whose
/// footprint changed, appeared, or disappeared since the previous render.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Footprint {
	id: u32,
	/// The node revision covers style, content, and every other primitive property.
	revision: u64,
	/// Visible rectangle after the element's own outset and its clip.
	rect: Option<Geometry>,
	clip: Option<Geometry>,
	mask: Option<ClipMask>,
	rotation: Option<Rotation>,
	opacity: f32,
	scale: [f32; 2],
}

/// Damage lists longer than this collapse into their union so consumers stay bounded.
pub const MAX_DAMAGE_RECTS: usize = 8;

/// Layout distance a captured pointer must travel before a press becomes a drag.
const DRAG_THRESHOLD: f32 = 6.0;

impl<C> Drop for Engine<C> {
	fn drop(&mut self) {
		// Tasks go first: their mounted scopes send cleanup to a channel nobody drains anymore.
		self.core.runtime.wakes.ready.lock().clear();
		drop(std::mem::take(&mut self.core.runtime.tasks));
	}
}

pub(super) struct EngineState {
	element_ids: HashSet<Id>,
	cursor: Option<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerState {
	/// Where the pointer is in layout units of the frame being evaluated, so it compares directly with
	/// [`Geometry`](crate::ui::layout::Geometry).
	pub position: UiPoint,
	pub pressed: bool,
}

impl Default for PointerState {
	fn default() -> Self {
		Self {
			position: UiPoint::zero(),
			pressed: false,
		}
	}
}

/// Maps normalized window coordinates, -1 to 1 with y up, onto a layout of `size` with y down.
fn normalized_to_layout(position: UiPoint, size: Size) -> UiPoint {
	UiPoint::new((position.x + 1.0) * 0.5 * size.x(), (1.0 - position.y) * 0.5 * size.y())
}

// Isolate clipping, evaluation, future, and runtime mechanics from the engine facade.
mod clipping;
mod commands;
mod evaluation_context;
mod futures;
#[cfg(test)]
mod invalidation_tests;
pub(super) mod properties;
mod runtime;

use clipping::*;
use commands::UiCommand;
use commands::apply;
pub use evaluation_context::*;
pub use futures::*;
use properties::Spares;
pub use properties::{ElementKind, Properties, Setup};
pub use runtime::*;

impl EngineState {
	fn new() -> Self {
		Self {
			element_ids: HashSet::new(),
			cursor: None,
		}
	}

	fn set_element_ids(&mut self, element_ids: impl IntoIterator<Item = Id>) {
		self.element_ids.clear();
		self.element_ids.extend(element_ids);
		self.cursor = self.cursor.filter(|id| self.element_ids.contains(id));
	}

	fn contains_element(&self, id: Id) -> bool {
		self.element_ids.contains(&id)
	}

	pub(super) fn set_cursor(&mut self, cursor: Option<Id>) -> Option<Id> {
		self.cursor = cursor.filter(|id| self.element_ids.contains(id));
		self.cursor
	}

	fn cursor(&self) -> Option<Id> {
		self.cursor
	}
}

impl Default for Engine<()> {
	fn default() -> Self {
		Self::new()
	}
}

impl Engine<()> {
	/// Creates a UI engine without application-specific shared context.
	///
	/// Next, call [`Self::mount`] with the root asynchronous component.
	pub fn new() -> Self {
		Self::with_context(())
	}
}

impl<C: 'static> Engine<C> {
	/// Creates a UI engine with shared application context available to components.
	///
	/// Next, call [`Self::mount`] with the root component, then begin the per-frame
	/// [`Self::evaluate`] and [`Self::render`] sequence.
	/// Measures text with the font at `path` instead of a system font.
	///
	/// Give the UI render pass the same file, so layout and drawing agree on glyph sizes.
	pub fn with_font(mut self, path: impl Into<std::path::PathBuf>) -> Self {
		self.text_system = TextSystem::with_font(Some(path.into()));
		self
	}

	pub fn with_context(ctx: C) -> Self {
		let (sender, commands) = std::sync::mpsc::channel();
		Self {
			viewports: Vec::new(),
			state: EngineState::new(),
			cursor_position: UiPoint::zero(),
			is_clicking: false,
			clicks: Vec::new(),
			scrolls: Vec::new(),
			drops: Vec::new(),
			dragged: None,
			hovered: None,
			hover_chains: [Vec::new(), Vec::new()],
			render_spares: Spares::default(),
			key_states: HashMap::new(),
			key_presses: VecDeque::new(),
			text_edits: VecDeque::new(),
			text_system: TextSystem::new(),
			core: UiPoll {
				runtime: Runtime::new(),
				ctx,
				current: None,
				tree: RetainedTree::new(),
				commands,
				sender,
			},
			retained_layout: None,
			retained_render: None,
			visual_state: Vec::new(),
			visual_state_key: None,
			measurements: Vec::new(),
			rendered_revisions: Vec::new(),
			rendered_footprints: Vec::new(),
			footprint_scratch: Vec::new(),
			depth_order: Vec::new(),
			placement: Vec::new(),
			hit_curves: HashMap::new(),
			transforms: Vec::new(),
			transform_work: Vec::new(),
			dirty: Vec::new(),
			dirty_stamps: Vec::new(),
			hit_offsets: Vec::new(),
			placement_indices: Vec::new(),
			next_render_revision: 1,
		}
	}

	/// Sets the waker that runs the tick in which this engine evaluates.
	///
	/// A component woken from another thread, such as by a worker that finished loading, wakes this waker so the
	/// host runs the tick that polls it. Pass `GraphicsApplication::waker` converted into a [`Waker`].
	pub fn set_waker(&mut self, waker: Waker) {
		*self.core.runtime.wakes.host.lock() = Some(waker);
	}

	/// Returns when this engine next needs an evaluation, or `None` when it only reacts to events.
	///
	/// The engine needs one now while a component waits for a frame, which includes every running animation, or a
	/// woken component waits to be polled. Otherwise the earliest UI timer decides. Hand the answer to the host
	/// that drives the engine, such as `GraphicsApplication::schedule_tick`.
	pub fn next_tick(&self) -> Option<std::time::Instant> {
		let runtime = &self.core.runtime;
		if runtime.needs_tick() {
			return Some(std::time::Instant::now());
		}
		runtime.next_deadline()
	}

	/// Returns the application context components read through [`Context::with`].
	pub fn ctx(&self) -> &C {
		&self.core.ctx
	}

	/// Returns the application context for the host to change between evaluations.
	pub fn ctx_mut(&mut self) -> &mut C {
		&mut self.core.ctx
	}

	pub(crate) fn add_viewport(&mut self, viewport: VirtualViewport) {
		self.viewports.push(viewport);
	}

	/// Mounts the root asynchronous component into the retained UI tree.
	///
	/// Pass an async function or closure, such as `async |ctx| { ... }`. Next, call [`Self::evaluate`] once per frame
	/// after updating pointer, key, and text input state.
	pub fn mount<F>(&mut self, root: F)
	where
		F: AsyncFnOnce(&mut EvaluationContext<C>) + 'static,
	{
		let ctx = EvaluationContext::<C>::new_root();
		// Store an owning future while preserving the borrowed component interface.
		let future = Box::pin(async move {
			let mut ctx = ctx;
			root(&mut ctx).await;
		});
		self.core.runtime.spawn(ScopeId::ROOT, ROOT_PATH, future);
	}

	/// Evaluates mounted UI tasks and returns a snapshot of the resulting layout.
	///
	/// A changed viewport size first ends the interaction in progress as
	/// [`Self::cancel`] does. The snapshot borrows the engine, so inspect it or move the cursor with it, then drop
	/// it. Next, call [`Self::render`] and submit the returned render data through [`crate::ui::UiRenderPass`].
	pub fn evaluate(&mut self, size: Size, frame_allocator: &bumpalo::Bump) -> Snapshot<'_> {
		// Layout distances and the held source's place change with the viewport.
		if self.retained_layout.as_ref().is_some_and(|retained| retained.size != size) {
			self.cancel();
		}
		self.sync_pointer_state(size);
		self.route_drag();
		self.route_drops();
		self.core.runtime.begin_frame();
		self.core.tree.begin_frame();
		poll_ready_tasks(&mut self.core);

		self.build_layout(size, frame_allocator);
		self.route_input_events();
		self.route_key_input_events();
		self.route_text_edit_events();

		poll_ready_tasks(&mut self.core);
		let retained = self
			.retained_layout
			.as_ref()
			.expect("UI layout was not retained. Evaluation did not prepare its snapshot.");
		snapshot_of(retained, &mut self.state.cursor)
	}

	// Visual transforms reuse placement for every flow. Other edits can replay custom
	// flows because their placement may depend on captured application state.
	fn build_layout(&mut self, size: Size, frame_allocator: &bumpalo::Bump) {
		let tree = &mut self.core.tree;
		let revision = tree.revision();
		let unchanged = self
			.retained_layout
			.as_ref()
			.is_some_and(|retained| retained.tree_revision == revision && retained.size == size);
		if !unchanged {
			let placement_unchanged = self.retained_layout.as_ref().is_some_and(|retained| {
				retained.placement_revision == tree.placement_revision
					&& (!retained.has_custom_flows || retained.non_transform_revision == tree.non_transform_revision)
					&& retained.size == size
					// Only edited text needs checking. Keep the refreshed measurement if a
					// changed size falls through to full placement, so it is not measured twice.
					&& tree.text_changes.iter().all(|&index| {
						let cached = &mut self.measurements[index];
						let Some((_, available, previous_size)) = *cached else {
							return false;
						};
						super::measure_element(&tree.elements[index], available, &mut self.text_system, cached) == previous_size
					})
			});
			let transforms_changed = !tree.transform_changes.is_empty();
			// Unchanged placement reuses the retained elements. They are moved out while the rest of the retained
			// layout is updated, and moved back once this evaluation no longer reads them.
			let previous = self
				.retained_layout
				.as_mut()
				.filter(|_| placement_unchanged && !transforms_changed)
				.map(|retained| std::mem::take(&mut retained.elements));
			let mut placed = Vec::new_in(frame_allocator);
			self.dirty.clear();
			if placement_unchanged && transforms_changed {
				placed.extend_from_slice(&self.retained_layout.as_ref().unwrap().elements);
				// Recompute from retained placement, never from already transformed bounds.
				// An edited ancestor owns the whole subtree, including nested edited roots.
				for &index in &tree.transform_changes {
					let mut ancestor = tree.parents[index];
					let mut covered = false;
					while let Some(parent) = ancestor {
						covered |= tree.transform_changes.contains(&parent);
						ancestor = tree.parents[parent];
					}
					if !covered {
						update_visual_subtree(
							index,
							&tree,
							&self.placement,
							&self.placement_indices,
							&mut self.transforms,
							&mut placed,
							&mut self.transform_work,
							&mut self.dirty,
						);
					}
				}
			} else if !placement_unchanged {
				placed = layout_elements(&tree, size, &mut self.text_system, &mut self.measurements, frame_allocator);
				self.placement.clear();
				self.placement.extend_from_slice(&placed);
				self.transforms.resize(tree.elements.len(), Affine2::identity());
				self.placement_indices.clear();
				self.placement_indices.resize(tree.elements.len(), usize::MAX);
				for (offset, element) in self.placement.iter().enumerate() {
					self.placement_indices[element.index] = offset;
				}
				for index in 0..tree.elements.len() {
					if tree.parents[index].is_none() {
						update_visual_subtree(
							index,
							&tree,
							&self.placement,
							&self.placement_indices,
							&mut self.transforms,
							&mut placed,
							&mut self.transform_work,
							&mut self.dirty,
						);
					}
				}
				self.dirty.clear();
			}
			let elements = previous.as_deref().unwrap_or(placed.as_slice());
			let has_custom_flows = self.retained_layout.as_ref()
				.filter(|retained| retained.flow_revision == tree.flow_revision)
				.map_or_else(|| tree.elements.iter().any(|element| {
					matches!(&element.element.primitive, Primitives::Container(container) if crate::ui::flow::placement_key(&container.flow).is_none())
				}), |retained| retained.has_custom_flows);
			let geometry_unchanged = self.retained_layout.as_ref().is_some_and(|retained| {
				retained.size == size
					&& retained.clip_revision == tree.clip_revision
					&& !transforms_changed
					&& (placement_unchanged || retained.elements.as_slice() == elements)
			});
			if !geometry_unchanged {
				// Resizing can replay a stateful flow without a tree mutation. Give each
				// changed geometry its own revision so older snapshots keep distinct cache keys.
				let layout_revision = self.retained_layout.as_ref().map_or(1, |retained| retained.revision + 1);
				// Only the transformed subtrees moved, so appearance outside them is still current.
				let transformed_from = self
					.retained_layout
					.as_ref()
					.filter(|_| placement_unchanged && transforms_changed)
					.map(|retained| retained.revision);
				Self::prepare_appearance(
					&mut self.visual_state,
					&mut self.visual_state_key,
					&self.dirty,
					&self.placement_indices,
					elements,
					tree,
					layout_revision,
					size,
					transformed_from,
				);
				// Moved subtrees patch the retained hit index in place while nothing else holds it
				// and the topology is stable; anything else rebuilds the index from every element.
				let refreshed = transformed_from.is_some()
					&& self.hit_offsets.len() == tree.elements.len()
					&& self.retained_layout.as_mut().is_some_and(|retained| {
						retained.clip_revision == tree.clip_revision
							&& refresh_hit_entries(
								&self.dirty,
								elements,
								&tree,
								&self.visual_state,
								&self.placement_indices,
								&self.hit_offsets,
								&mut self.hit_curves,
								&mut retained.acceleration,
								frame_allocator,
							)
					});
				let hit = (!refreshed)
					.then(|| clipped_hit_elements(elements, &tree, &self.visual_state, &mut self.hit_curves, frame_allocator));
				// A stable topology keeps IDs and layout order, so update only changed bounds.
				// Structural edits also advance clip_revision, including removal and remount of the same ID.
				if let Some(previous) = self
					.retained_layout
					.as_ref()
					.filter(|retained| retained.clip_revision == tree.clip_revision)
				{
					let runtime = &mut self.core.runtime;
					debug_assert_eq!(previous.elements.len(), elements.len());
					for (previous, element) in previous.elements.iter().zip(elements.iter()) {
						debug_assert_eq!(previous.id, element.id);
						if previous.position != element.position || previous.size != element.size {
							runtime
								.geometry
								.insert(element.id, Geometry::new(element.position, element.size));
						}
					}
				} else {
					self.state.set_element_ids(elements.iter().map(|element| element.id));
					self.core.runtime.update_geometry(elements);
				}
				let retained = self.retained_layout.get_or_insert_with(|| RetainedLayout {
					tree_revision: revision,
					placement_revision: tree.placement_revision,
					non_transform_revision: tree.non_transform_revision,
					flow_revision: tree.flow_revision,
					has_custom_flows,
					clip_revision: tree.clip_revision,
					revision: layout_revision,
					transformed_from,
					size,
					elements: Vec::new(),
					relations: Vec::new(),
					acceleration: MouseClickAcceleration::default(),
				});
				retained.revision = layout_revision;
				retained.transformed_from = transformed_from;
				retained.clip_revision = tree.clip_revision;
				retained.size = size;
				if !placement_unchanged || transforms_changed {
					retained.elements.clear();
					retained.elements.extend_from_slice(elements);
				}
				retained.relations.clear();
				retained.relations.extend_from_slice(&tree.relations);
				if let Some(hit) = hit {
					retained.acceleration.update(&hit.elements, &hit.curves, &hit.points);
					self.hit_offsets.clear();
					self.hit_offsets.resize(tree.elements.len(), u32::MAX);
					for (offset, element) in hit.elements.iter().enumerate() {
						if let Some(index) = tree.index_of(element) {
							self.hit_offsets[index] = offset as u32;
						}
					}
				}
			}
			let retained = self
				.retained_layout
				.as_mut()
				.expect("UI layout was not retained. Evaluation did not prepare its snapshot.");
			retained.tree_revision = revision;
			retained.placement_revision = tree.placement_revision;
			retained.non_transform_revision = tree.non_transform_revision;
			retained.flow_revision = tree.flow_revision;
			retained.has_custom_flows = has_custom_flows;
			if let Some(previous) = previous {
				retained.elements = previous;
			}
			tree.text_changes.clear();
			tree.transform_changes.clear();
		}
	}

	/// Reuses inherited appearance only for the same tree inputs and snapshot geometry.
	///
	/// `transformed_from` names the layout this geometry differs from by visual transforms
	/// alone, within [`Self::dirty`]; appearance prepared for that layout is then refreshed
	/// for those elements only, since state inherits strictly from the parent.
	/// Refreshes inherited clipping and opacity for `elements`, or keeps them while the tree's appearance and the
	/// layout are unchanged. Takes the engine's fields apart so callers can keep borrowing the tree.
	#[allow(clippy::too_many_arguments)]
	fn prepare_appearance(
		visual_state: &mut Vec<VisualState>,
		visual_state_key: &mut Option<(u64, u64, Size)>,
		dirty: &[usize],
		placement_indices: &[usize],
		elements: &[LayoutElement],
		tree: &RetainedTree,
		layout_revision: u64,
		size: Size,
		transformed_from: Option<u64>,
	) {
		let key = (tree.appearance_revision, layout_revision, size);
		if *visual_state_key == Some(key) {
			return;
		}
		let previous = transformed_from.map(|revision| (tree.appearance_revision, revision, size));
		if previous.is_some() && *visual_state_key == previous && visual_state.len() == tree.elements.len() {
			for &index in dirty {
				let element = &elements[placement_indices[index]];
				visual_state[index] = inherited_visual_state(element, index, tree, visual_state);
			}
		} else {
			prepare_visual_state(elements, tree, visual_state);
		}
		*visual_state_key = Some(key);
	}

	fn sync_pointer_state(&mut self, size: Size) {
		self.core.runtime.pointer = PointerState {
			position: normalized_to_layout(self.cursor_position, size),
			pressed: self.is_clicking,
		};
	}

	/// Hit-tests this frame's pointer once, then delivers hover changes, clicks, and scrolls.
	fn route_input_events(&mut self) {
		let held = self.core.runtime.drag.capture().map(|capture| capture.source);
		let position = self.cursor_position;
		let retained = self
			.retained_layout
			.as_ref()
			.expect("UI layout was not retained. Evaluation did not prepare its snapshot.");
		let mut snapshot = snapshot_of(retained, &mut self.state.cursor);
		let hovered = snapshot.hover(position, held);
		// Every click and scroll this frame lands on the same position, and a hit also moves the cursor there.
		let pressed = self.clicks.contains(&true) || !self.scrolls.is_empty();
		let target = if pressed { snapshot.click(position) } else { None };

		self.route_hover(hovered, held);
		while let Some(click) = self.clicks.pop() {
			if click && let Some(target) = target {
				self.core.runtime.push_event(UiEvent {
					target,
					kind: Events::Actuated,
					delta: None,
					source: None,
				});
			}
		}

		while let Some(delta) = self.scrolls.pop() {
			if let Some(target) = target {
				self.route_bubbling_event(target, Events::Scrolled, Some(delta), None);
			}
		}
	}

	/// Tells surfaces the pointer entered or left them, from this frame's geometry.
	///
	/// A held source is skipped so the surface beneath a dragged item is the one
	/// that hears about the pointer, and the capture records it as its drop preview.
	fn route_hover(&mut self, hovered: Option<Id>, held: Option<Id>) {
		if held.is_some() {
			self.core.runtime.drag.set_over(hovered);
		}
		let previous = std::mem::replace(&mut self.hovered, hovered);
		if previous == hovered {
			return;
		}
		let tree = &self.core.tree;
		let ancestors = |start: Option<Id>, chain: &mut Vec<Id>| {
			chain.clear();
			let mut current = start;
			while let Some(id) = current {
				chain.push(id);
				current = tree
					.element_indices
					.get(&id)
					.and_then(|&index| tree.parents[index])
					.map(|parent| tree.elements[parent].id);
			}
		};
		let [exited, entered] = &mut self.hover_chains;
		ancestors(previous, exited);
		ancestors(hovered, entered);
		let runtime = &mut self.core.runtime;
		// A surface containing both the old and the new target keeps the pointer and hears nothing.
		for &target in exited.iter().filter(|id| !entered.contains(id)) {
			runtime.push_event(drag_event(target, Events::PointerExited));
		}
		for &target in entered.iter().filter(|id| !exited.contains(id)) {
			runtime.push_event(drag_event(target, Events::PointerEntered));
		}
	}

	/// Delivers queued drops before components poll, so a component sees the drop
	/// and the cleared capture together.
	///
	/// Targets come from the previous frame's hit geometry, which the caller
	/// measured the release against. The target is any surface under the release
	/// point other than the released source, which hears about the end after it.
	fn route_drops(&mut self) {
		// Taken for the walk and put back emptied, so the list keeps its storage for the next release.
		let mut drops = std::mem::take(&mut self.drops);
		for drop in drops.drain(..) {
			let target = self.retained_layout.as_ref().and_then(|retained| {
				let point = crate::ui::flow::Location::new(drop.position.x, drop.position.y);
				retained
					.acceleration
					.query_excluding(point, Some(drop.source.get()))
					.and_then(Id::new)
			});
			if let Some(target) = target {
				self.route_bubbling_event(target, Events::Dropped, None, Some(drop.source));
			}
			self.core.runtime.push_event(UiEvent {
				target: drop.source,
				kind: Events::DragEnded,
				delta: None,
				source: target,
			});
		}
		self.drops = drops;
	}

	/// Tells a dragged source where it is, once per evaluation and only after it moved.
	fn route_drag(&mut self) {
		let runtime = &mut self.core.runtime;
		let Some(capture) = runtime.drag.capture().filter(|capture| capture.dragging) else {
			return;
		};
		if self.dragged == Some(capture.position) {
			return;
		}
		self.dragged = Some(capture.position);
		runtime.push_event(UiEvent {
			target: capture.source,
			kind: Events::Dragged,
			delta: Some(UiVector::new(
				capture.position.x - capture.origin.x,
				capture.position.y - capture.origin.y,
			)),
			source: None,
		});
	}

	/// Converts normalized window coordinates to the last evaluated frame's layout units.
	fn layout_point(&self, position: UiPoint) -> Option<UiPoint> {
		let size = self.retained_layout.as_ref()?.size;
		Some(normalized_to_layout(position, size))
	}

	/// Returns the frontmost surface at normalized window coordinates in the
	/// last evaluated frame, without running layout.
	pub fn hit(&self, position: UiPoint) -> Option<Id> {
		let point = self.layout_point(position)?;
		let retained = self.retained_layout.as_ref()?;
		retained
			.acceleration
			.query(crate::ui::flow::Location::new(point.x, point.y))
			.and_then(Id::new)
	}

	/// Delivers one event to a target and then to each of its ancestors.
	fn route_bubbling_event(&mut self, target: Id, kind: Events, delta: Option<UiVector>, source: Option<Id>) {
		let tree = &self.core.tree;
		let mut current = Some(target);

		while let Some(target) = current {
			self.core.runtime.push_event(UiEvent {
				target,
				kind,
				delta,
				source,
			});
			current = tree
				.element_indices
				.get(&target)
				.and_then(|&index| tree.parents[index])
				.map(|parent| tree.elements[parent].id);
		}
	}

	fn route_key_input_events(&mut self) {
		while let Some(key) = self.key_presses.pop_front() {
			let target = {
				let state = &self.state;
				self.core.runtime.focused_target(|target| state.contains_element(target))
			};

			if let Some(target) = target {
				self.core.runtime.push_key_event(UiKeyEvent { target, key });
			}
		}
	}

	fn route_text_edit_events(&mut self) {
		while let Some(edit) = self.text_edits.pop_front() {
			let target = {
				let state = &self.state;
				self.core.runtime.focused_target(|target| state.contains_element(target))
			};

			if let Some(target) = target {
				self.core.runtime.push_text_edit_event(UiTextEditEvent { target, edit });
			}
		}
	}

	/// Builds render data from the layout the last [`Self::evaluate`] produced.
	///
	/// Next, give the returned data to [`crate::ui::UiRenderPass`] for GPU drawing.
	/// The render is retained by the engine: while the tree, its layout, and the
	/// viewport are unchanged, the same render with the same [`Render::revision`]
	/// is returned again. Cloning shares its contents; publish a clone only when
	/// the revision changed, since a retained clone forces the next build to
	/// allocate fresh buffers instead of reusing the engine's.
	pub fn render(&mut self) -> &Render {
		let layout = self.retained_layout.as_mut().expect(
			"UI render has no layout to draw. The most likely cause is calling render before the first evaluate.",
		);
		let (layout_revision, size) = (layout.revision, layout.size);
		let tree_revision = self.core.tree.revision();
		let retained = self.retained_render.as_ref().is_some_and(|retained| {
			retained.tree_revision == tree_revision && retained.layout_revision == layout_revision && retained.size == size
		});
		if !retained {
			// The elements are moved out for the build, which reads them while it changes the engine's other state.
			let elements = std::mem::take(&mut layout.elements);
			self.retained_render = Some(self.build_render(&elements, layout_revision, size));
			if let Some(layout) = self.retained_layout.as_mut() {
				layout.elements = elements;
			}
		}
		&self
			.retained_render
			.as_ref()
			.expect("UI render must be retained after a rebuild. The most likely cause is a rebuild that returned early.")
			.render
	}

	// Rebuild visible draw data while preserving its layout order and owned render buffers.
	#[allow(clippy::too_many_lines)]
	fn build_render(&mut self, layout_elements: &[LayoutElement], layout_revision: u64, size: Size) -> RetainedRender {
		let tree = &self.core.tree;
		let mut visibility_unchanged = self.retained_render.as_ref().is_some_and(|retained| {
			retained.clip_revision == tree.clip_revision
				&& retained.layout_revision == layout_revision
				&& retained.size == size
		});
		// Geometry that moved by visual transforms alone, with appearance and clipping as
		// the retained render saw them, leaves every entry outside the dirty subtrees as it is.
		let incremental = self
			.retained_render
			.as_ref()
			.zip(self.retained_layout.as_ref())
			.is_some_and(|(render, layout)| {
				layout.transformed_from == Some(render.layout_revision)
					&& layout.revision == layout_revision
					&& render.clip_revision == tree.clip_revision
					&& render.appearance_revision == tree.appearance_revision
					&& render.size == size
					&& self.transforms.len() == tree.elements.len()
					// Once most of the tree moved, patching costs more than the plain rebuild.
					&& self.dirty.len() * 4 < tree.elements.len()
			});
		// Damage is relative to the previous render only while the viewport is the same size.
		let damage_base = self
			.retained_render
			.as_ref()
			.filter(|retained| retained.size == size)
			.map(|retained| retained.render.revision);
		let render_revision = RenderRevision(self.next_render_revision);
		self.next_render_revision += 1;
		// Reuse the engine-owned contents in place, allocation included. A consumer still holding the previous render
		// keeps it, and every entry is then written into fresh contents.
		let placement_unchanged = self
			.retained_render
			.as_ref()
			.is_some_and(|retained| retained.placement_revision == tree.placement_revision);
		let (mut shared, mut visible) = match self.retained_render.take() {
			Some(retained) => (retained.render.contents, retained.visible),
			None => (
				std::sync::Arc::new(RenderContents::empty(render_revision, Default::default())),
				Vec::new(),
			),
		};
		let reclaimed = std::sync::Arc::get_mut(&mut shared).is_some();
		if !reclaimed {
			shared = std::sync::Arc::new(RenderContents::empty(
				shared.surface_revision,
				std::sync::Arc::clone(&shared.surface_ids),
			));
		}
		let contents = std::sync::Arc::get_mut(&mut shared).expect("The render contents were just reclaimed or made.");
		// Keep live identities independently of culling so sinks retain hidden canvas data.
		// Placement edits are a conservative boundary for refreshing membership.
		if !placement_unchanged {
			// Refilled in place while no consumer shares the list, so a changed screen keeps its storage.
			let ids = std::sync::Arc::make_mut(&mut contents.surface_ids);
			ids.clear();
			ids.extend(tree.elements.iter().map(|element| element.serial));
			ids.sort_unstable();
			contents.surface_revision = render_revision;
		}
		let RenderContents {
			elements,
			curve_elements,
			path_elements,
			image_elements,
			text_elements,
			damage,
			..
		} = contents;
		damage.clear();
		let spares = &mut self.render_spares;
		// Entries can only be kept where they are when the previous lists were reclaimed.
		let incremental = incremental && reclaimed;
		// Rewrite the live prefix while reusing each entry's owned buffers. Entries left
		// beyond that prefix are dropped after the walk, so removed content cannot escape.
		let (mut element_count, mut curve_count, mut path_count, mut text_count, mut image_count) = (0, 0, 0, 0, 0);
		let previous_footprints = std::mem::take(&mut self.rendered_footprints);
		let mut next_footprints = std::mem::take(&mut self.footprint_scratch);
		next_footprints.clear();
		next_footprints.resize(tree.elements.len(), None);
		self.rendered_revisions.resize(tree.elements.len(), 0);
		// Input callbacks can change appearance after layout. The cache key includes those changes.
		Self::prepare_appearance(
			&mut self.visual_state,
			&mut self.visual_state_key,
			&self.dirty,
			&self.placement_indices,
			layout_elements,
			tree,
			layout_revision,
			size,
			None,
		);
		let stamp = layout_revision;
		if incremental {
			self.dirty_stamps.resize(tree.elements.len(), 0);
			// Depth is a placement property, so order holds; only membership can flip, and a
			// flip shifts entry slots, which the full walk below then rewrites.
			let mut flipped = false;
			for &index in &self.dirty {
				self.dirty_stamps[index] = stamp;
				let offset = self.placement_indices[index];
				let element = layout_elements[offset];
				let now_visible = self.visual_state[index]
					.clip
					.apply(geometry_from_layout_element(&element))
					.is_some();
				let slot = self.depth_order.binary_search(&(element.position.z(), offset as u32)).ok();
				if slot.is_some() != now_visible {
					flipped = true;
				} else if let Some(slot) = slot {
					visible[slot] = element;
				}
			}
			visibility_unchanged = !flipped;
		}
		let incremental = incremental && visibility_unchanged;
		if !visibility_unchanged {
			// Stable depth order is shared by every primitive list. Keep it with visibility
			// so paint-only rebuilds neither sort nor allocate scratch for larger render entries.
			// Offsets are unique, so an unstable sort of (depth, offset) keeps layout order
			// at equal depth without the scratch buffer a stable element sort would allocate.
			self.depth_order.clear();
			self.depth_order
				.extend(layout_elements.iter().enumerate().filter_map(|(offset, element)| {
					let visible = tree.index_of(element).is_some_and(|index| {
						self.visual_state[index]
							.clip
							.apply(geometry_from_layout_element(element))
							.is_some()
					});
					visible.then(|| (element.position.z(), offset as u32))
				}));
			self.depth_order.sort_unstable();
			visible.clear();
			visible.extend(self.depth_order.iter().map(|&(_, offset)| layout_elements[offset as usize]));
		}
		for element in &visible {
			let Some(index) = tree.index_of(element) else {
				continue;
			};
			let retained_element = &tree.elements[index];
			let local_unchanged = self.rendered_revisions[index] == retained_element.revision;
			if incremental && local_unchanged && self.dirty_stamps[index] != stamp {
				// Neither its geometry nor its content changed: the retained entry and footprint hold.
				next_footprints[index] = previous_footprints.get(index).copied().flatten();
				match &retained_element.element.primitive {
					Primitives::Container(_) | Primitives::Shape(_) => element_count += 1,
					Primitives::Curve(_) => curve_count += 1,
					Primitives::Path(_) => path_count += 1,
					Primitives::Image(_) => image_count += 1,
					Primitives::Text(_) | Primitives::TextField(_) => text_count += 1,
				}
				continue;
			}
			self.rendered_revisions[index] = retained_element.revision;
			let state = self.visual_state[index];
			let clip = state.clip.as_rect();
			let clip_mask = state.mask;
			let rotation = self
				.transforms
				.get(index)
				.map(|transform| transform.rotation)
				.filter(|rotation| !rotation.is_identity());
			let opacity = effective_opacity(index, &tree, &mut self.visual_state);
			let style = retained_element.element.primitive.style();
			// Curves stroke outward from their path; rectangles stroke inward.
			let outset = match &retained_element.element.primitive {
				Primitives::Curve(_) => {
					style
						.layers()
						.iter()
						.map(|layer| match layer.kind() {
							LayerKind::Stroke { width } if width.is_finite() && width > 0.0 => width,
							_ => 0.0,
						})
						.fold(0.0f32, f32::max)
						* state.scale[0].max(state.scale[1])
						* 0.5
				}
				// An outer shadow paints past the box, so its reach joins the damaged area.
				_ => {
					style
						.layers()
						.iter()
						.map(|layer| match layer.kind() {
							LayerKind::Shadow(shadow) => shadow.outset(),
							_ => 0.0,
						})
						.fold(0.0f32, f32::max)
						* state.scale[0].max(state.scale[1])
				}
			};
			let footprint = Footprint {
				id: retained_element.serial,
				revision: retained_element.revision,
				rect: {
					let rect = geometry_from_layout_element(element).expanded(outset);
					let rect = match clip {
						Some(clip) => rect.intersect(clip),
						None => Some(rect),
					};
					// Damage covers where the pixels land, which a turn moves away from the placement.
					rect.map(|rect| rotation.map_or(rect, |rotation| rotation.geometry(rect)))
				},
				clip,
				mask: clip_mask,
				rotation,
				opacity,
				scale: state.scale,
			};
			match previous_footprints.get(index).copied().flatten() {
				Some(previous) if previous == footprint => {}
				Some(previous) => {
					damage.extend(previous.rect);
					damage.extend(footprint.rect);
				}
				None => damage.extend(footprint.rect),
			}
			next_footprints[index] = Some(footprint);
			// Only layered geometry retains a style copy; images and text borrow what they need.
			let mut push_rectangle = |spares: &mut Spares, corner_radius, corner_exponent, sector| {
				if let Some(entry) = elements
					.get_mut(element_count)
					.filter(|entry| local_unchanged && entry.id == retained_element.serial)
				{
					entry.position = element.position;
					entry.size = element.size;
					entry.clip = clip;
					entry.clip_mask = clip_mask;
					entry.rotation = rotation;
					entry.opacity = opacity;
					element_count += 1;
					return;
				}
				let mut layers = elements
					.get_mut(element_count)
					.map(|entry| std::mem::take(&mut entry.style.layers))
					.unwrap_or_default();
				spares.fit_layers(&mut layers, style.layers.len());
				layers.clone_from(&style.layers);
				let rendered = RenderElement {
					id: retained_element.serial,
					position: element.position,
					size: element.size,
					clip,
					clip_mask,
					rotation,
					style: ConcreteStyle { layers },
					opacity,
					backdrop_blur_radius: style
						.layers()
						.iter()
						.find(|layer| matches!(layer.kind(), LayerKind::Fill) && layer.backdrop_blur_radius() > 0.0)
						.map_or(0.0, |layer| layer.backdrop_blur_radius()),
					corner_radius,
					corner_exponent,
					sector,
				};
				if element_count < elements.len() {
					elements[element_count] = rendered;
				} else {
					elements.push(rendered);
				}
				element_count += 1;
			};
			let mut push_text = |spares: &mut Spares, content: &str, font_size| {
				if let Some(entry) = text_elements
					.get_mut(text_count)
					.filter(|entry| local_unchanged && entry.id == retained_element.serial)
				{
					entry.position = element.position;
					entry.size = element.size;
					entry.clip = clip;
					entry.clip_mask = clip_mask;
					entry.rotation = rotation;
					entry.opacity = opacity;
					entry.scale = state.scale[0].min(state.scale[1]);
					text_count += 1;
					return;
				}
				let mut retained_content = text_elements
					.get_mut(text_count)
					.map(|entry| std::mem::take(&mut entry.content))
					.unwrap_or_else(|| spares.string());
				retained_content.clear();
				retained_content.push_str(content);
				let rendered = RenderTextElement {
					id: retained_element.serial,
					position: element.position,
					size: element.size,
					clip,
					clip_mask,
					rotation,
					color: match style.layers().first().map(|layer| &layer.color) {
						Some(Color::Value(rgba)) => *rgba,
						_ => RGBA::white(),
					},
					opacity,
					font_size,
					scale: state.scale[0].min(state.scale[1]),
					content: retained_content,
				};
				if text_count < text_elements.len() {
					text_elements[text_count] = rendered;
				} else {
					text_elements.push(rendered);
				}
				text_count += 1;
			};

			match &retained_element.element.primitive {
				Primitives::Container(container) => {
					push_rectangle(spares, container.corner_radius, container.corner_exponent, container.sector)
				}
				Primitives::Shape(shape) => {
					push_rectangle(spares, shape.settings.corner_radius, shape.settings.corner_exponent, None)
				}
				Primitives::Curve(curve) => {
					if let Some(entry) = curve_elements
						.get_mut(curve_count)
						.filter(|entry| local_unchanged && entry.id == retained_element.serial)
					{
						entry.position = element.position;
						entry.size = element.size;
						entry.clip = clip;
						entry.clip_mask = clip_mask;
						entry.rotation = rotation;
						entry.opacity = opacity;
						entry.scale = state.scale;
						curve_count += 1;
						continue;
					}
					let (mut layers, mut segments) = curve_elements
						.get_mut(curve_count)
						.map(|entry| (std::mem::take(&mut entry.style.layers), std::mem::take(&mut entry.segments)))
						.unwrap_or_else(|| (SmallVec::new(), spares.segments()));
					spares.fit_layers(&mut layers, style.layers.len());
					layers.clone_from(&style.layers);
					segments.clear();
					segments.extend_from_slice(curve.path().segments());
					let rendered = RenderCurveElement {
						id: retained_element.serial,
						position: element.position,
						size: element.size,
						clip,
						clip_mask,
						rotation,
						style: ConcreteStyle { layers },
						opacity,
						scale: state.scale,
						segments,
					};
					if curve_count < curve_elements.len() {
						curve_elements[curve_count] = rendered;
					} else {
						curve_elements.push(rendered);
					}
					curve_count += 1;
				}
				Primitives::Path(path) => {
					if let Some(entry) = path_elements
						.get_mut(path_count)
						.filter(|entry| local_unchanged && entry.id == retained_element.serial)
					{
						entry.position = element.position;
						entry.size = element.size;
						entry.clip = clip;
						entry.clip_mask = clip_mask;
						entry.rotation = rotation;
						entry.opacity = opacity;
						entry.scale = state.scale;
						path_count += 1;
						continue;
					}
					let mut layers = path_elements
						.get_mut(path_count)
						.map(|entry| std::mem::take(&mut entry.style.layers))
						.unwrap_or_default();
					spares.fit_layers(&mut layers, style.layers.len());
					layers.clone_from(&style.layers);
					// The outline is shared with every draw list entry that shows it, and only
					// copied here when the element changed.
					let segments = path_elements
						.get(path_count)
						.filter(|entry| entry.path_id == path.id() && entry.version == path.version())
						.map(|entry| std::sync::Arc::clone(&entry.segments))
						.unwrap_or_else(|| std::sync::Arc::from(path.path().segments()));
					let rendered = RenderPathElement {
						id: retained_element.serial,
						path_id: path.id(),
						version: path.version(),
						fill_rule: path.fill_rule,
						view_box: path.view_box,
						position: element.position,
						size: element.size,
						clip,
						clip_mask,
						rotation,
						style: ConcreteStyle { layers },
						opacity,
						scale: state.scale,
						segments,
					};
					if path_count < path_elements.len() {
						path_elements[path_count] = rendered;
					} else {
						path_elements.push(rendered);
					}
					path_count += 1;
				}
				Primitives::Image(image) => {
					let rendered = RenderImageElement {
						id: retained_element.serial,
						image_id: image.id(),
						version: image.version(),
						source_width: image.width_pixels(),
						source_height: image.height_pixels(),
						pixels: std::sync::Arc::clone(image.pixels()),
						position: element.position,
						size: element.size,
						clip,
						clip_mask,
						rotation,
						opacity,
					};
					if image_count < image_elements.len() {
						image_elements[image_count] = rendered;
					} else {
						image_elements.push(rendered);
					}
					image_count += 1;
				}
				Primitives::Text(text) => push_text(spares, text.content(), text.settings().font_size),
				Primitives::TextField(text_field) => push_text(spares, text_field.content(), text_field.settings().font_size),
			}
		}

		// Entries past the live prefix leave their buffers to the entries of later renders.
		for entry in elements.drain(element_count..) {
			spares.keep_layers(entry.style.layers);
		}
		for entry in curve_elements.drain(curve_count..) {
			spares.keep_layers(entry.style.layers);
			spares.keep_segments(entry.segments);
		}
		for entry in path_elements.drain(path_count..) {
			spares.keep_layers(entry.style.layers);
		}
		for entry in text_elements.drain(text_count..) {
			spares.keep_string(entry.content);
		}
		image_elements.truncate(image_count);

		// Elements that were drawn last time and are now culled, removed, or truncated leave a hole.
		for (index, previous) in previous_footprints.iter().enumerate() {
			if let Some(previous) = previous
				&& next_footprints.get(index).copied().flatten().is_none()
			{
				damage.extend(previous.rect);
			}
		}
		self.footprint_scratch = previous_footprints;
		self.rendered_footprints = next_footprints;
		match damage_base {
			Some(_) => collapse_damage(damage),
			None => damage.clear(),
		}
		contents.revision = render_revision;
		contents.viewport_size = size;
		contents.damage_base = damage_base;

		RetainedRender {
			tree_revision: tree.revision(),
			placement_revision: tree.placement_revision,
			clip_revision: tree.clip_revision,
			appearance_revision: tree.appearance_revision,
			layout_revision: layout_revision,
			size: size,
			visible,
			render: Render { contents: shared },
		}
	}

	pub fn set_cursor_position(&mut self, v: UiPoint) {
		self.cursor_position = v;
	}

	pub fn cursor(&self) -> Option<Id> {
		self.state.cursor()
	}

	pub fn set_cursor(&mut self, cursor: Option<Id>) -> Option<Id> {
		self.state.set_cursor(cursor)
	}

	pub fn clear_cursor(&mut self) {
		self.state.set_cursor(None);
	}

	pub fn update_click_state(&mut self, v: bool) {
		self.is_clicking = v;
		self.clicks.push(v);
	}

	pub fn update_scroll_state(&mut self, delta: UiVector) {
		self.scrolls.push(delta);
	}

	pub fn update_key_state(&mut self, key: Key, pressed: bool) {
		let was_pressed = self.key_states.insert(key, pressed).unwrap_or(false);
		if pressed && !was_pressed {
			self.key_presses.push_back(key);
		}
	}

	pub fn input_character(&mut self, character: char) {
		if character != '\0' {
			self.text_edits.push_back(TextEdit::Inserted(character));
		}
	}

	pub fn delete_text_backward(&mut self) {
		if let Some(character) = self.focused_text_field_last_char() {
			self.text_edits.push_back(TextEdit::Deleted(character));
		}
	}

	/// Grabs the surface under normalized window coordinates for a pointer gesture.
	///
	/// The surface comes from the last evaluated frame and receives
	/// [`Events::Grabbed`]. Returns `false` when nothing is there or another
	/// source is held. Next, forward pointer motion through [`Self::drag_to`].
	pub fn press(&mut self, position: UiPoint) -> bool {
		let Some(source) = self.hit(position) else {
			return false;
		};
		let Some(point) = self.layout_point(position) else {
			return false;
		};
		let runtime = &mut self.core.runtime;
		if !runtime.drag.press(source, point) {
			return false;
		}
		self.dragged = None;
		runtime.push_event(drag_event(source, Events::Grabbed));
		true
	}

	/// Moves the captured pointer and reports whether a source is held.
	///
	/// The gesture activates once the pointer travels the drag threshold and stays
	/// active if it returns. The source then gets [`Events::Dragged`] from the
	/// next [`Self::evaluate`]. Next, call [`Self::release`] when the pointer is released.
	pub fn drag_to(&mut self, position: UiPoint) -> bool {
		let Some(point) = self.layout_point(position) else {
			return false;
		};
		self.core.runtime.drag.move_to(point)
	}

	/// Releases the captured source and returns a drop only after activation.
	///
	/// The release position participates in threshold detection. A click clears
	/// capture without yielding a drop. A drop is also delivered as
	/// [`Events::Dropped`] to the surface under the release position by the next
	/// [`Self::evaluate`], so either the caller or a component can apply it. The
	/// source gets [`Events::DragEnded`] either way.
	pub fn release(&mut self, position: UiPoint) -> Option<DragDrop> {
		let point = self.layout_point(position)?;
		let runtime = &mut self.core.runtime;
		let held = runtime.drag.capture()?.source;
		let dropped = runtime.drag.release(point);
		match dropped {
			Some(dropped) => self.drops.push(dropped),
			None => runtime.push_event(drag_event(held, Events::DragEnded)),
		}
		dropped
	}

	/// Returns the captured drag gesture, if a source is held.
	pub fn drag(&self) -> Option<DragCapture> {
		self.core.runtime.drag.capture()
	}

	/// Returns the surface under the pointer after the last evaluation, skipping a held source.
	pub fn hovered(&self) -> Option<Id> {
		self.hovered
	}

	/// Ends the interaction in progress and returns the source of a cancelled drag.
	///
	/// Queued clicks, scrolls, drops, key presses, and text edits are discarded,
	/// held keys and the pointer are released, and a held source is restored
	/// without a drop. The source gets [`Events::DragEnded`]. Call this when the
	/// window loses focus or the user cancels; a changed viewport size calls it
	/// from [`Self::evaluate`].
	pub fn cancel(&mut self) -> Option<Id> {
		self.is_clicking = false;
		self.clicks.clear();
		self.scrolls.clear();
		self.drops.clear();
		self.key_states.clear();
		self.key_presses.clear();
		self.text_edits.clear();
		let runtime = &mut self.core.runtime;
		let source = runtime.drag.cancel()?;
		runtime.push_event(drag_event(source, Events::DragEnded));
		Some(source)
	}

	/// Returns the compact id render data carries for the element `id`.
	#[cfg(test)]
	pub(crate) fn render_id(&self, id: Id) -> u32 {
		self.core.tree.element(id).expect("the element is in the tree").serial
	}

	fn focused_text_field_last_char(&mut self) -> Option<char> {
		let target = {
			let state = &self.state;
			self.core.runtime.focused_target(|target| state.contains_element(target))?
		};
		let element = self.core.tree.element(target)?;
		let Primitives::TextField(text_field) = &element.element.primitive else {
			return None;
		};
		text_field.content().chars().last()
	}
}

/// Drops empty rectangles and bounds the damage list at [`MAX_DAMAGE_RECTS`] by taking the union.
fn collapse_damage(damage: &mut Vec<Geometry>) {
	damage.retain(|rect| !rect.is_empty());
	// An element that changed in place pushes its rectangle twice.
	damage.dedup();
	if damage.len() > MAX_DAMAGE_RECTS {
		let union = damage
			.iter()
			.copied()
			.reduce(Geometry::union)
			.expect("Damage list is non-empty. The most likely cause is a length check that changed.");
		damage.clear();
		damage.push(union);
	}
}

/// The `RenderRevision` struct identifies the content of one [`Render`].
///
/// Revisions are unique within one [`Engine`]. Two renders from the same engine
/// with equal revisions describe identical visuals, so consumers keep the revision they last adopted
/// and skip work while it repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderRevision(u64);

/// The `Render` struct preserves the visual data derived from a snapshot so UI primitives can be submitted to the renderer.
///
/// Its contents are shared, so cloning a render costs a reference count instead of
/// copying every element. The engine reuses the buffers of a render nobody else
/// holds; a consumer that retains a clone keeps it intact while the next one is built.
#[derive(Clone)]
pub struct Render {
	contents: std::sync::Arc<RenderContents>,
}

impl std::ops::Deref for Render {
	type Target = RenderContents;

	fn deref(&self) -> &Self::Target {
		&self.contents
	}
}

/// The `RenderContents` struct holds the primitive lists a [`Render`] shares.
pub struct RenderContents {
	pub(crate) surface_revision: RenderRevision,
	pub(crate) surface_ids: std::sync::Arc<Vec<u32>>,
	/// The viewport defines layout units independently of the root visual transform.
	pub(crate) viewport_size: Size,
	elements: Vec<RenderElement>,
	curve_elements: Vec<RenderCurveElement>,
	path_elements: Vec<RenderPathElement>,
	image_elements: Vec<RenderImageElement>,
	text_elements: Vec<RenderTextElement>,
	revision: RenderRevision,
	/// Layout-unit rectangles whose pixels differ from `damage_base`; at most [`MAX_DAMAGE_RECTS`].
	damage: Vec<Geometry>,
	/// The render this damage is relative to; `None` when everything changed, such as after a viewport resize.
	damage_base: Option<RenderRevision>,
}

impl RenderContents {
	/// Makes contents with no entries and the surface list `surface_ids`, which a render build then fills.
	fn empty(surface_revision: RenderRevision, surface_ids: std::sync::Arc<Vec<u32>>) -> Self {
		Self {
			surface_revision,
			surface_ids,
			viewport_size: Size::new(0, 0),
			elements: Vec::new(),
			curve_elements: Vec::new(),
			path_elements: Vec::new(),
			image_elements: Vec::new(),
			text_elements: Vec::new(),
			revision: surface_revision,
			damage: Vec::new(),
			damage_base: None,
		}
	}
}

impl Render {
	/// Identifies this render's content; unchanged UI keeps the same revision across frames.
	pub fn revision(&self) -> RenderRevision {
		self.revision
	}

	/// Returns the base revision and the layout-unit regions that differ from it.
	///
	/// A consumer holding pixels for the base revision only needs to redraw inside these
	/// rectangles. `None` means everything changed. Rectangles cover element bounds plus any
	/// outward stroke and are already intersected with the element clip; consumers add their own
	/// pixel-space margins such as anti-aliasing, glyph padding, and blur kernels.
	pub fn damage(&self) -> Option<(RenderRevision, &[Geometry])> {
		self.damage_base.map(|base| (base, self.damage.as_slice()))
	}

	#[cfg(test)]
	pub(crate) fn root(&self) -> &RenderElement {
		self.elements.iter().find(|e| e.id == 1).unwrap()
	}

	pub(crate) fn size(&self) -> usize {
		self.elements.len()
			+ self.curve_elements.len()
			+ self.path_elements.len()
			+ self.image_elements.len()
			+ self.text_elements.len()
	}

	pub(crate) fn elements(&self) -> impl Iterator<Item = &RenderElement> {
		self.elements.iter()
	}

	pub(crate) fn texts(&self) -> impl Iterator<Item = &RenderTextElement> {
		self.text_elements.iter()
	}

	pub(crate) fn curves(&self) -> impl Iterator<Item = &RenderCurveElement> {
		self.curve_elements.iter()
	}

	pub(crate) fn paths(&self) -> impl Iterator<Item = &RenderPathElement> {
		self.path_elements.iter()
	}

	pub(crate) fn images(&self) -> impl Iterator<Item = &RenderImageElement> {
		self.image_elements.iter()
	}
}

/// The `VirtualViewport` struct reserves a stable identity for a virtual UI output region.
pub(crate) struct VirtualViewport(Id);

#[derive(Debug, Clone, PartialEq)]
pub struct UiEvent {
	pub target: Id,
	pub kind: Events,
	/// The scroll delta of an [`Events::Scrolled`] event, or the offset from the
	/// press point of an [`Events::Dragged`] event.
	pub delta: Option<UiVector>,
	/// The released drag source of an [`Events::Dropped`] event, or the surface
	/// dropped on of an [`Events::DragEnded`] event.
	pub source: Option<Id>,
}

/// Builds an event addressed to a drag source without a payload.
fn drag_event(source: Id, kind: Events) -> UiEvent {
	UiEvent {
		target: source,
		kind,
		delta: None,
		source: None,
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiKeyEvent {
	pub target: Id,
	pub key: Key,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiTextEditEvent {
	pub target: Id,
	pub edit: TextEdit,
}

#[cfg(test)]
mod tests {
	use std::{sync::mpsc, time::Duration};

	use super::*;
	use crate::ui::{
		Depth, animate,
		components::{
			container::Container,
			curve::{CurvePath, CurveSegment},
			shape::Shape,
			text_field::TextField,
		},
		flow::{self, Location3},
		layout::{
			Geometry, Sizing,
			context::{ContainerContext, Context, ElementContext},
		},
		primitive::TextEdit,
		spring,
		style::{ConcreteLayer, ConcreteStyle, EdgeFeather, Layer, LayerKind},
	};

	/// The `DropCounter` struct reports its drop on a channel, so a test can see the engine release the context it owns.
	struct DropCounter(mpsc::Sender<()>);

	impl Drop for DropCounter {
		fn drop(&mut self) {
			let _ = self.0.send(());
		}
	}

	/// Converts layout units in a 128 by 128 frame to normalized window coordinates exactly.
	fn window(x: f32, y: f32) -> UiPoint {
		UiPoint::new(x / 64.0 - 1.0, 1.0 - y / 64.0)
	}

	/// Mounts a 20 by 20 source at the origin and records the drag it sees on every frame.
	fn drag_observer() -> Engine<Vec<Option<DragCapture>>> {
		let mut engine = Engine::with_context(Vec::new());
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.hit_testable(false)).await;
			root.element("source")
				.container(|c| c.absolute_position(0, 0).size(20.into()))
				.await;
			loop {
				let drag = ctx.drag().await;
				ctx.with(|observed| observed.push(drag)).await;
				ctx.render().await;
			}
		});
		engine
	}

	#[test]
	fn press_activates_after_the_threshold_and_release_yields_the_drop_once() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = drag_observer();
		// Nothing can be grabbed before a frame supplies hit geometry.
		assert!(!engine.press(window(10.0, 10.0)));
		let _ = engine.evaluate(Size::new(128, 128), &frame_allocator);
		assert!(!engine.press(window(30.0, 30.0)));
		assert!(engine.press(window(10.0, 10.0)));
		assert!(!engine.press(window(10.0, 10.0)));
		assert!(engine.drag_to(window(12.0, 12.0)));
		engine.evaluate(Size::new(128, 128), &frame_allocator);
		let held = engine.ctx()[1].expect("expected test value");
		assert!(!held.dragging);
		assert_eq!(held.origin, UiPoint::new(10.0, 10.0));
		assert_eq!(held.position, UiPoint::new(12.0, 12.0));
		// The release itself can supply the motion that reaches the threshold.
		let dropped = engine.release(window(30.0, 10.0)).expect("expected test value");
		assert_eq!(dropped.source, engine.hit(window(10.0, 10.0)).expect("expected test value"));
		assert_eq!(dropped.position, UiPoint::new(30.0, 10.0));
		assert!(engine.drag().is_none());
		assert!(engine.release(window(30.0, 10.0)).is_none());
		assert!(!engine.drag_to(window(0.0, 0.0)));
	}

	#[test]
	fn click_restores_source_without_a_drop() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = drag_observer();
		engine.evaluate(Size::new(128, 128), &frame_allocator);
		assert!(engine.press(window(10.0, 10.0)));
		assert!(engine.release(window(12.0, 12.0)).is_none());
		assert!(engine.drag().is_none());
	}

	#[test]
	fn cancel_restores_the_source_and_discards_queued_input() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = drag_observer();
		let _ = engine.evaluate(Size::new(128, 128), &frame_allocator);
		let source = engine.hit(window(10.0, 10.0)).expect("expected test value");
		engine.press(window(10.0, 10.0));
		engine.drag_to(window(20.0, 10.0));
		engine.update_click_state(true);
		assert_eq!(engine.cancel(), Some(source));
		assert_eq!(engine.cancel(), None);
		assert!(engine.release(window(20.0, 10.0)).is_none());
		engine.evaluate(Size::new(128, 128), &frame_allocator);
		assert_eq!(engine.ctx()[1], None);
		assert!(!engine.core.runtime.pointer.pressed);
		assert!(engine.press(window(10.0, 10.0)));
	}

	#[test]
	fn resized_viewport_cancels_the_held_source_before_evaluation() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = drag_observer();
		let _ = engine.evaluate(Size::new(128, 128), &frame_allocator);
		engine.press(window(10.0, 10.0));
		let _ = engine.evaluate(Size::new(128, 128), &frame_allocator);
		engine.evaluate(Size::new(64, 128), &frame_allocator);
		let observed = engine.ctx();
		assert!(observed[1].is_some());
		assert_eq!(observed[2], None);
	}

	/// What the board's components observed, in the order the source and target saw it.
	#[derive(Default)]
	struct DragLog {
		/// The target, its inner child, and the source.
		ids: Option<(Id, Id, Id)>,
		grabbed: usize,
		dragged: Vec<UiVector>,
		/// The surface each gesture ended on.
		ended: Vec<Option<Id>>,
		drops: Vec<Option<Id>>,
		/// The number of drops recorded when each end arrived.
		drops_before_end: Vec<usize>,
	}

	/// Mounts a target with a hit-testable child and a source drawn over the target's corner.
	fn drag_board() -> Engine<DragLog> {
		let mut engine = Engine::with_context(DragLog::default());
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.hit_testable(false)).await;
			let mut target = root.element("target").container(|c| c.absolute_position(0, 0).size(50.into())).await;
			let inner = target.element("inner").container(|c| c.absolute_position(30, 30).size(20.into())).await;
			let mut source = root.element("source").container(|c| c.absolute_position(0, 0).size(20.into())).await;
			let ids = (target.id(), inner.id(), source.id());
			ctx.with(|log| log.ids = Some(ids)).await;
			loop {
				// A biased select takes the queued drop before the end, as a client would.
				utils::r#async::select_biased! {
					event = target.on(Events::Dropped) => ctx.with(|log| log.drops.push(event.source)).await,
					_ = source.on(Events::Grabbed) => ctx.with(|log| log.grabbed += 1).await,
					event = source.on(Events::Dragged) => ctx.with(|log| log.dragged.push(event.delta.expect("expected test value"))).await,
					event = source.on(Events::DragEnded) => {
						ctx.with(|log| {
							log.ended.push(event.source);
							let drops = log.drops.len();
							log.drops_before_end.push(drops);
						})
						.await;
					},
				}
			}
		});
		engine
	}

	#[test]
	fn drag_events_reach_the_source_and_the_surface_under_the_release() {
		let allocator = bumpalo::Bump::new();
		let mut engine = drag_board();
		let _ = engine.evaluate(Size::new(128, 128), &allocator);
		let (target, inner, source) = engine.ctx().ids.expect("expected test value");

		// Motion past the threshold starts the drag; the drop lands on the target's child and bubbles up.
		assert!(engine.press(window(10.0, 10.0)));
		assert!(engine.drag_to(window(40.0, 40.0)));
		let _ = engine.evaluate(Size::new(128, 128), &allocator);
		{
			let observed = engine.ctx();
			assert_eq!(observed.grabbed, 1);
			assert_eq!(observed.dragged, vec![UiVector::new(30.0, 30.0)]);
		}
		assert!(engine.release(window(40.0, 40.0)).is_some());
		let _ = engine.evaluate(Size::new(128, 128), &allocator);
		{
			let observed = engine.ctx();
			assert_eq!(observed.drops, vec![Some(source)]);
			assert_eq!(observed.ended, vec![Some(inner)]);
		}

		// A release whose motion activates the drag still drops, and a release over the
		// source itself drops onto the surface beneath it.
		assert!(engine.press(window(1.0, 1.0)));
		assert!(engine.release(window(19.0, 19.0)).is_some());
		let _ = engine.evaluate(Size::new(128, 128), &allocator);
		{
			let observed = engine.ctx();
			assert_eq!(observed.grabbed, 2);
			assert_eq!(observed.drops, vec![Some(source), Some(source)]);
			assert_eq!(observed.ended, vec![Some(inner), Some(target)]);
		}

		// A release outside every surface drops nowhere, and a click ends its grab without a drop.
		assert!(engine.press(window(10.0, 10.0)));
		assert!(engine.release(window(90.0, 90.0)).is_some());
		let _ = engine.evaluate(Size::new(128, 128), &allocator);
		assert!(engine.press(window(10.0, 10.0)));
		assert!(engine.release(window(12.0, 12.0)).is_none());
		engine.evaluate(Size::new(128, 128), &allocator);
		let observed = engine.ctx();
		assert_eq!(observed.grabbed, 4);
		assert_eq!(observed.drops.len(), 2);
		// Every grab ends once, and the target's drop is recorded before the source's end.
		assert_eq!(observed.ended, vec![Some(inner), Some(target), None, None]);
		assert_eq!(observed.drops_before_end, vec![1, 2, 2, 2]);
	}

	#[test]
	fn motion_reaches_the_source_once_per_evaluation() {
		let allocator = bumpalo::Bump::new();
		let mut engine = drag_board();
		let _ = engine.evaluate(Size::new(128, 128), &allocator);

		assert!(engine.press(window(10.0, 10.0)));
		assert!(engine.drag_to(window(12.0, 10.0)));
		let _ = engine.evaluate(Size::new(128, 128), &allocator);
		// Motion inside the threshold is not a drag.
		assert!(engine.ctx().dragged.is_empty());
		assert!(engine.drag_to(window(30.0, 10.0)));
		assert!(engine.drag_to(window(40.0, 10.0)));
		let _ = engine.evaluate(Size::new(128, 128), &allocator);
		let _ = engine.evaluate(Size::new(128, 128), &allocator);
		assert_eq!(engine.ctx().dragged, vec![UiVector::new(30.0, 0.0)]);
		assert!(engine.drag_to(window(40.0, 20.0)));
		engine.evaluate(Size::new(128, 128), &allocator);
		assert_eq!(engine.ctx().dragged, vec![UiVector::new(30.0, 0.0), UiVector::new(30.0, 10.0)]);
	}

	#[test]
	fn cancel_ends_every_grab() {
		let allocator = bumpalo::Bump::new();
		let mut engine = drag_board();
		let _ = engine.evaluate(Size::new(128, 128), &allocator);
		let (_, _, source) = engine.ctx().ids.expect("expected test value");

		assert!(engine.press(window(10.0, 10.0)));
		assert_eq!(engine.cancel(), Some(source));
		let _ = engine.evaluate(Size::new(128, 128), &allocator);
		assert!(engine.press(window(10.0, 10.0)));
		assert!(engine.drag_to(window(40.0, 40.0)));
		assert_eq!(engine.cancel(), Some(source));
		engine.evaluate(Size::new(128, 128), &allocator);
		let observed = engine.ctx();
		assert_eq!(observed.grabbed, 2);
		assert_eq!(observed.ended, vec![None, None]);
		assert!(observed.dragged.is_empty());
		assert!(observed.drops.is_empty());
	}

	#[test]
	fn reparent_appends_to_the_new_parent_and_refuses_cycles() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(None);
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.flow(flow::column_with_gap(0))).await;
			let mut first = root.element("first").container(|c| c.size(10.into())).await;
			let mut second = root
				.element("second")
				.container(|c| c.size(10.into()).flow(flow::column_with_gap(0)))
				.await;
			second.element("inner").container(|c| c.size(5.into())).await;
			let ids = (first.id(), second.id());
			ctx.with(|log| *log = Some(ids)).await;
			ctx.render().await;
			first.reparent(second.id()).await;
			// A cycle is refused when applied, so `first` stays under `second`.
			second.reparent(first.id()).await;
			loop {
				ctx.render().await;
			}
		});
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		let (first, second) = engine.ctx().expect("expected test value");
		assert_eq!(engine.core.runtime.geometry[&first].y(), 0.0);
		assert_eq!(engine.core.runtime.geometry[&second].y(), 10.0);
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		engine.evaluate(Size::new(100, 100), &allocator);
		let runtime = &engine.core.runtime;
		assert_eq!(runtime.geometry[&second].y(), 0.0);
		assert_eq!(runtime.geometry[&first].y(), 5.0);
	}

	/// Evaluates and renders one frame, returning the revision and a copy of the damage.
	fn damage_frame<C: 'static>(
		engine: &mut Engine<C>,
		size: Size,
	) -> (RenderRevision, Option<(RenderRevision, Vec<Geometry>)>) {
		let frame_allocator = bumpalo::Bump::new();
		engine.evaluate(size, &frame_allocator);
		let render = engine.render();
		(render.revision(), render.damage().map(|(base, rects)| (base, rects.to_vec())))
	}

	/// Reports whether one damage rectangle contains the given layout-unit box.
	fn damage_covers(damage: &[Geometry], x: f32, y: f32, width: f32, height: f32) -> bool {
		damage.iter().any(|rect| {
			rect.x() <= x + 0.01
				&& rect.y() <= y + 0.01
				&& rect.right() >= x + width - 0.01
				&& rect.bottom() >= y + height - 0.01
		})
	}

	/// The `BoxState` struct lets a test drive the box that [`mount_box`] mounts through the engine context.
	#[derive(Default)]
	struct BoxState {
		position: (i32, i32),
		red: f32,
	}

	/// Mounts a root with one absolutely positioned box whose position and color follow the engine's [`BoxState`].
	fn mount_box() -> Engine<BoxState> {
		let mut engine = Engine::with_context(BoxState {
			position: (0, 0),
			red: 1.0,
		});
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut r#box = root
				.element("box")
				.container(|c| c.size(10.into()).absolute_position(0, 0))
				.await;
			let (mut last_position, mut last_red) = ((0, 0), 1.0f32);
			loop {
				let (position, red) = ctx.with(|state| (state.position, state.red)).await;
				if position != last_position {
					last_position = position;
					r#box.update_container(|c| c.position(last_position)).await;
				}
				if red != last_red {
					last_red = red;
					r#box
						.update_container(|c| {
							c.style(ConcreteLayer::default().color(RGBA::new(last_red, 0.0, 0.0, 1.0).into()))
						})
						.await;
				}
				ctx.render().await;
			}
		});
		engine
	}

	#[test]
	fn first_render_and_viewport_change_damage_everything() {
		let mut engine = mount_box();
		let (first, damage) = damage_frame(&mut engine, Size::new(100, 100));
		assert!(damage.is_none(), "The first render has no base to be relative to.");
		let (second, damage) = damage_frame(&mut engine, Size::new(120, 100));
		assert_ne!(first, second);
		assert!(damage.is_none(), "A viewport change invalidates every pixel.");
	}

	#[test]
	fn moved_element_damages_old_and_new_bounds() {
		let mut engine = mount_box();
		let (first, _) = damage_frame(&mut engine, Size::new(100, 100));
		engine.ctx_mut().position = (50, 50);
		let (second, damage) = damage_frame(&mut engine, Size::new(100, 100));
		let (base, damage) = damage.expect("a second render is relative to the first");
		assert_ne!(first, second);
		assert_eq!(base, first);
		assert!(damage_covers(&damage, 0.0, 0.0, 10.0, 10.0), "old bounds: {damage:?}");
		assert!(damage_covers(&damage, 50.0, 50.0, 10.0, 10.0), "new bounds: {damage:?}");
		assert!(
			!damage_covers(&damage, 0.0, 0.0, 100.0, 100.0),
			"the unchanged root is not damaged: {damage:?}"
		);
	}

	/// A transform edit takes the incremental render path; damage still covers the moved
	/// box's old and new bounds and nothing of the untouched sibling.
	#[test]
	fn transformed_element_damages_old_and_new_bounds_only() {
		let mut engine = Engine::with_context(0.0f32);
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.hit_testable(false)).await;
			// Enough untouched siblings that the moved subtree is a small share of the tree.
			for slot in 0..12 {
				root.element(("static", slot as usize))
					.container(|c| c.size(10.into()).absolute_position(80, 80 + slot))
					.await;
			}
			let mut moved = root
				.element("moved")
				.container(|c| c.size(10.into()).absolute_position(0, 0).clip(false))
				.await;
			// Its child sits apart from the parent's own bounds, so its damage is separate.
			moved
				.element("child")
				.container(|c| c.size(10.into()).absolute_position(20, 20))
				.await;
			let mut applied = 0.0f32;
			loop {
				let offset = ctx.with(|offset| *offset).await;
				if offset != applied {
					applied = offset;
					moved
						.update_container(|c| c.transform(Transform::identity().translate(applied, applied)))
						.await;
				}
				ctx.render().await;
			}
		});
		let (first, _) = damage_frame(&mut engine, Size::new(100, 100));
		*engine.ctx_mut() = 50.0;
		let (second, damage) = damage_frame(&mut engine, Size::new(100, 100));
		let (base, damage) = damage.expect("a second render is relative to the first");
		assert_ne!(first, second);
		assert_eq!(base, first);
		assert!(damage_covers(&damage, 0.0, 0.0, 10.0, 10.0), "old bounds: {damage:?}");
		assert!(damage_covers(&damage, 50.0, 50.0, 10.0, 10.0), "new bounds: {damage:?}");
		assert!(damage_covers(&damage, 20.0, 20.0, 10.0, 10.0), "old child bounds: {damage:?}");
		assert!(damage_covers(&damage, 70.0, 70.0, 10.0, 10.0), "new child bounds: {damage:?}");
		assert!(
			!damage
				.iter()
				.any(|rect| rect.right() > 80.0 && rect.bottom() > 80.0 && rect.x() < 80.0),
			"the untouched sibling is not damaged: {damage:?}"
		);
		// A second move damages the previous and the next bounds again, without the first.
		*engine.ctx_mut() = 20.0;
		let (_, damage) = damage_frame(&mut engine, Size::new(100, 100));
		let (base, damage) = damage.expect("relative damage");
		assert_eq!(base, second);
		assert!(damage_covers(&damage, 50.0, 50.0, 10.0, 10.0), "previous bounds: {damage:?}");
		assert!(damage_covers(&damage, 20.0, 20.0, 10.0, 10.0), "next bounds: {damage:?}");
		assert!(!damage_covers(&damage, 0.0, 0.0, 10.0, 10.0), "first bounds: {damage:?}");
	}

	#[test]
	fn style_change_damages_one_rectangle() {
		let mut engine = mount_box();
		let _ = damage_frame(&mut engine, Size::new(100, 100));
		engine.ctx_mut().red = 0.5;
		let (_, damage) = damage_frame(&mut engine, Size::new(100, 100));
		let (_, damage) = damage.expect("relative damage");
		assert_eq!(damage.len(), 1, "{damage:?}");
		assert!(damage_covers(&damage, 0.0, 0.0, 10.0, 10.0));
	}

	#[test]
	fn added_and_removed_elements_damage_their_bounds() {
		let mut engine = Engine::with_context(false);
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut shown = false;
			let mut node = None;
			loop {
				let visible = ctx.with(|visible| *visible).await;
				if visible != shown {
					shown = visible;
					if shown {
						node = Some(
							root.element("node")
								.container(|c| c.size(20.into()).absolute_position(30, 40))
								.await,
						);
					} else if let Some(mut node) = node.take() {
						node.remove().await;
					}
				}
				ctx.render().await;
			}
		});
		let _ = damage_frame(&mut engine, Size::new(100, 100));
		*engine.ctx_mut() = true;
		let (_, damage) = damage_frame(&mut engine, Size::new(100, 100));
		let (_, damage) = damage.expect("relative damage");
		assert!(damage_covers(&damage, 30.0, 40.0, 20.0, 20.0), "added: {damage:?}");
		*engine.ctx_mut() = false;
		let (_, damage) = damage_frame(&mut engine, Size::new(100, 100));
		let (_, damage) = damage.expect("relative damage");
		assert!(damage_covers(&damage, 30.0, 40.0, 20.0, 20.0), "removed: {damage:?}");
		assert!(!damage_covers(&damage, 0.0, 0.0, 100.0, 100.0), "root untouched: {damage:?}");
	}

	#[test]
	fn child_of_moved_parent_is_damaged_and_long_lists_collapse() {
		let mut engine = Engine::with_context(0);
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut panel = root
				.element("panel")
				.container(|c| c.size(50.into()).absolute_position(0, 0))
				.await;
			for index in 0..12 {
				panel
					.element(format!("child{index}"))
					.container(|c| c.size(4.into()).absolute_position(index * 4, 0))
					.await;
			}
			let mut last = 0;
			loop {
				let offset = ctx.with(|offset| *offset).await;
				if offset != last {
					last = offset;
					panel.update_container(|c| c.position((last, last))).await;
				}
				ctx.render().await;
			}
		});
		let _ = damage_frame(&mut engine, Size::new(100, 100));
		*engine.ctx_mut() = 20;
		let (_, damage) = damage_frame(&mut engine, Size::new(100, 100));
		let (_, damage) = damage.expect("relative damage");
		// Twelve children plus the panel exceed the cap, so one union remains.
		assert_eq!(damage.len(), 1, "{damage:?}");
		assert!(damage_covers(&damage, 0.0, 0.0, 70.0, 70.0), "{damage:?}");
	}

	#[test]
	fn unchanged_tree_keeps_layout_and_render_revision_across_frames() {
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			root.element("label").text("Stable", |t| t).await;
			loop {
				ctx.render().await;
			}
		});
		let frame_allocator = bumpalo::Bump::new();

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let first_layout = engine.retained_layout.as_ref().map(|layout| layout.revision);
		let first_render = engine.render();
		let (first_revision, first_size) = (first_render.revision(), first_render.size());
		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let second_layout = engine.retained_layout.as_ref().map(|layout| layout.revision);
		let second_render = engine.render();

		assert_eq!(first_layout, second_layout);
		assert_eq!(first_revision, second_render.revision());
		assert_eq!(first_size, second_render.size());
		assert!(engine.retained_layout.is_some());
	}

	#[test]
	fn property_mutation_and_resize_advance_the_render_revision() {
		let mut engine = Engine::with_context(1.0f32);
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			loop {
				let opacity = ctx.with(|opacity| *opacity).await;
				root.update_container(|c| c.opacity(opacity)).await;
				ctx.render().await;
			}
		});
		let frame_allocator = bumpalo::Bump::new();
		let snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let baseline = engine.render().revision();

		// The mounted task mutates the container every frame, so revisions must move.
		*engine.ctx_mut() = 0.5;
		let snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let mutated = engine.render();
		assert_ne!(baseline, mutated.revision());
		assert_eq!(mutated.elements().next().unwrap().opacity, 0.5);
		let mutated = mutated.revision();

		engine.evaluate(Size::new(200, 100), &frame_allocator);
		let resized = engine.render();
		assert_ne!(mutated, resized.revision());
		assert_eq!(resized.root().size, Size::new(200, 100));
	}

	#[test]
	fn retained_tree_revision_tracks_insertion_mutation_and_removal() {
		let mut tree = RetainedTree::new();
		let start = tree.revision();
		let container = |_, _: &mut Spares| Primitives::Container(Container::default());
		let id = crate::ui::layout::context::slot_path(ROOT_PATH, "root".into());
		assert!(tree.add_element(None, ROOT_PATH, id, container).is_some());
		assert!(tree.revision() > start);

		let after_insert = tree.revision();
		// Re-declaring the same path on a later frame is idempotent and must not invalidate retained state.
		tree.begin_frame();
		assert!(tree.add_element(None, ROOT_PATH, id, container).is_none());
		assert_eq!(tree.revision(), after_insert);

		// An edit that writes nothing new keeps retained state valid.
		assert_eq!(tree.update_element(id, |_, _| true), Some(true));
		assert_eq!(tree.revision(), after_insert);

		let updated = tree.update_element(id, |primitive, _| {
			let Primitives::Container(container) = primitive else {
				return false;
			};
			container.visual.opacity = 0.5;
			true
		});
		assert_eq!(updated, Some(true));
		assert!(tree.revision() > after_insert);

		let after_mutation = tree.revision();
		let child = crate::ui::layout::context::slot_path(id.get(), "child".into());
		let child_path = child.get();
		tree.add_element(Some(id), id.get(), child, container);
		let after_child = tree.revision();
		assert!(after_child > after_mutation);
		assert!(!tree.remove_scope(child_path).is_empty());
		assert!(tree.revision() > after_child);
		assert!(tree.remove_scope(child_path).is_empty());
	}

	/// The `ScopeCounters` struct lets a test observe and close the scopes that [`counting_scope`] mounts.
	#[derive(Default)]
	struct ScopeCounters {
		ticks: [u32; 2],
		open: [bool; 2],
	}

	/// Mounts a scope that spawns a component counting the frames it runs in `ticks[scope]`, until `open[scope]` clears.
	fn counting_scope(scope: usize) -> impl AsyncFnOnce(&mut EvaluationContext<ScopeCounters>) + 'static {
		async move |ctx| {
			ctx.element("body").container(|c| c).await;
			ctx.element("ticker")
				.component(async move |ctx| {
					loop {
						ctx.with(|counters| counters.ticks[scope] += 1).await;
						ctx.render().await;
					}
				})
				.await;
			while ctx.with(|counters| counters.open[scope]).await {
				ctx.render().await;
			}
		}
	}

	#[test]
	fn removing_a_mounted_scope_ends_the_components_spawned_inside_it() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(ScopeCounters::default());
		engine.ctx_mut().open[0] = true;
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			loop {
				if ctx.with(|counters| counters.open[0]).await {
					root.element("menu").mount(counting_scope(0)).await;
				} else {
					ctx.render().await;
				}
			}
		});
		let frames = |engine: &mut Engine<ScopeCounters>, count: usize| {
			for _ in 0..count {
				engine.evaluate(Size::new(100, 100), &allocator);
			}
		};

		frames(&mut engine, 3);
		assert!(engine.ctx().ticks[0] > 0);
		engine.ctx_mut().open[0] = false;
		frames(&mut engine, 2);
		let closed = engine.ctx().ticks[0];
		frames(&mut engine, 3);
		assert_eq!(
			engine.ctx().ticks[0],
			closed,
			"A component kept running after its scope was removed."
		);

		// Reopening spawns a fresh component that runs again.
		engine.ctx_mut().open[0] = true;
		frames(&mut engine, 3);
		assert!(engine.ctx().ticks[0] > closed);
	}

	#[test]
	fn removing_a_mounted_scope_keeps_tasks_of_a_live_scope_with_the_same_path() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(ScopeCounters::default());
		engine.ctx_mut().open[0] = true;
		engine.ctx_mut().open[1] = true;
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut first = root.element("toast").mount(counting_scope(0));
			utils::r#async::select_biased! {
				_ = first => {},
				_ = ctx.render() => {},
			}
			// Declared on a later frame under the same parent and name, so its path repeats the first's.
			let mut second = root.element("toast").mount(counting_scope(1));
			loop {
				utils::r#async::select_biased! {
					_ = first => {},
					_ = second => {},
					_ = ctx.render() => {},
				}
			}
		});
		let frames = |engine: &mut Engine<ScopeCounters>, count: usize| {
			for _ in 0..count {
				engine.evaluate(Size::new(100, 100), &allocator);
			}
		};

		frames(&mut engine, 3);
		assert!(engine.ctx().ticks[0] > 0 && engine.ctx().ticks[1] > 0);
		engine.ctx_mut().open[0] = false;
		frames(&mut engine, 2);
		let (first_closed, second_closed) = (engine.ctx().ticks[0], engine.ctx().ticks[1]);
		frames(&mut engine, 3);
		assert_eq!(engine.ctx().ticks[0], first_closed);
		assert!(
			engine.ctx().ticks[1] > second_closed,
			"Removing one scope ended a live scope's component."
		);
	}

	#[test]
	fn dropping_engine_releases_mounted_context() {
		let (sender, drops) = mpsc::channel();
		let mut engine = Engine::with_context(DropCounter(sender));

		engine.mount(async move |_ctx| {});
		drop(engine);

		assert_eq!(drops.try_iter().count(), 1);
	}

	#[test]
	fn mounted_task_retains_markup_without_render_loop() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			ctx.element("root").container(|c| c.flow(flow::column)).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(engine.render().size(), 1);

		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(engine.render().size(), 1);
	}

	#[test]
	fn retained_button_receives_later_click_event() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);

		engine.mount(async move |ctx| {
			let mut button = ctx.element("button").container(|c| c).await;
			loop {
				button.on(Events::Actuated).await;
				ctx.with(|hits| *hits += 1).await;
			}
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::zero());
		engine.update_click_state(true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 1);
	}

	#[test]
	fn context_pointer_reflects_engine_pointer_state() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(None);

		engine.set_cursor_position(UiPoint::new(0.25, -0.5));
		engine.update_click_state(true);
		engine.mount(async move |ctx| {
			let value = Some(ctx.pointer().await);
			ctx.with(|observed| *observed = value).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(
			*engine.ctx(),
			Some(PointerState {
				position: UiPoint::new(62.5, 75.0),
				pressed: true,
			})
		);
	}

	#[test]
	fn context_pointer_updates_across_render_await_frames() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(Vec::new());

		engine.mount(async move |ctx| {
			let value = ctx.pointer().await;
			ctx.with(|observed| observed.push(value)).await;
			ctx.render().await;
			let value = ctx.pointer().await;
			ctx.with(|observed| observed.push(value)).await;
		});

		engine.set_cursor_position(UiPoint::new(-1.0, -1.0));
		engine.update_click_state(false);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.75, 0.5));
		engine.update_click_state(true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(
			*engine.ctx(),
			vec![
				PointerState {
					position: UiPoint::new(0.0, 100.0),
					pressed: false,
				},
				PointerState {
					position: UiPoint::new(87.5, 25.0),
					pressed: true,
				},
			]
		);
	}

	#[test]
	fn scroll_event_bubbles_from_hovered_child_to_parent() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(None);

		engine.mount(async move |ctx| {
			let mut parent = ctx.element("parent").container(|c| c).await;
			parent.element("child").container(|c| c).await;
			let event = parent.on(Events::Scrolled).await;
			let value = event.delta;
			ctx.with(|received| *received = value).await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::zero());
		engine.update_scroll_state(UiVector::new(0.0, -1.0));
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), Some(UiVector::new(0.0, -1.0)));
	}

	#[test]
	fn nested_retained_components_attach_under_declaring_element_with_stable_ids() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c.flow(flow::column)).await;
			frame
				.element("child")
				.component(async move |ctx| {
					ctx.element("button").container(|c| c.size(20.into())).await;
				})
				.await;
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let first_ids = first.elements.iter().map(|element| element.id).collect::<Vec<_>>();
		let first_relations = first.relations.to_vec();

		assert_eq!(first_ids.len(), 2);
		assert_eq!(first_relations, [(first_ids[0], first_ids[1])]);

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let second_ids = second.elements.iter().map(|element| element.id).collect::<Vec<_>>();

		assert_eq!(second_ids, first_ids);
		assert_eq!(second.relations, first_relations.as_slice());
	}

	#[test]
	fn indexed_sibling_keys_keep_stable_ids_across_frames() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c.flow(flow::column)).await;
			for index in 0..64usize {
				frame.element(("item", index)).container(|c| c.size(1.into())).await;
			}
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let first_ids = first.elements.iter().map(|element| element.id).collect::<Vec<_>>();

		assert_eq!(first_ids.len(), 65);

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let second_ids = second.elements.iter().map(|element| element.id).collect::<Vec<_>>();

		assert_eq!(second_ids, first_ids);
	}

	#[test]
	fn formatted_and_owned_names_key_the_same_element() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(None);

		engine.mount(async move |ctx| {
			let node = 7;
			let formatted = ctx.element(format_args!("node-{node}")).container(|c| c).await.id();
			ctx.render().await;
			let owned = ctx.element(format!("node-{node}")).container(|c| c).await.id();
			ctx.with(|ids| *ids = Some((formatted, owned))).await;
		});
		for _ in 0..2 {
			engine.evaluate(Size::new(100, 100), &frame_allocator);
		}

		let (formatted, owned) = engine.ctx().expect("The component did not declare both keys.");
		assert_eq!(formatted, owned);
	}

	#[test]
	fn mounted_scope_cleanup_removes_structural_path_descendants() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			frame
				.element("modal")
				.mount(async move |ctx| {
					ctx.element("body").container(|c| c.size(10.into())).await;
					ctx.render().await;
				})
				.await;
		});

		assert_eq!(engine.evaluate(Size::new(100, 100), &frame_allocator).elements.len(), 2);
		assert_eq!(engine.evaluate(Size::new(100, 100), &frame_allocator).elements.len(), 1);
	}

	#[test]
	fn retained_hits_and_render_clones_keep_their_geometry_after_resize() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			ctx.element("root").container(|c| c.size(Sizing::Relative(1, 1))).await;
		});
		let mut first_hits = crate::ui::intersection::HitTest::default();
		engine.evaluate(Size::new(100, 100), &allocator).retain_hit_test(&mut first_hits);
		let first_render = engine.render().clone();
		let root = first_hits.query(UiPoint::zero()).unwrap();
		assert_eq!(first_hits.query(UiPoint::new(2.0, -2.0)), None);
		let mut second_hits = crate::ui::intersection::HitTest::default();
		engine.evaluate(Size::new(200, 200), &allocator).retain_hit_test(&mut second_hits);
		assert_eq!(second_hits.query(UiPoint::new(0.5, -0.5)), Some(root));
		assert_eq!(
			engine.render().elements().next().unwrap().size,
			Size::new(200, 200)
		);
		// Geometry a host copied out and a render it cloned belong to the host, so a later frame leaves them as they were.
		assert_eq!(first_hits.query(UiPoint::zero()), Some(root));
		assert_eq!(first_hits.query(UiPoint::new(2.0, -2.0)), None);
		assert_eq!(first_render.elements().next().unwrap().size, Size::new(100, 100));
	}

	#[test]
	fn a_task_waking_during_its_poll_completes_in_the_same_evaluation() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let mut waiting = true;
			std::future::poll_fn(move |cx| {
				if std::mem::take(&mut waiting) {
					cx.waker().wake_by_ref();
					cx.waker().wake_by_ref();
					Poll::Pending
				} else {
					Poll::Ready(())
				}
			})
			.await;
			ctx.element("ready").container(|c| c.size(20.into())).await;
		});
		let snapshot = engine.evaluate(Size::new(100, 100), &allocator);
		assert_eq!(snapshot.elements.len(), 1);
		assert_eq!(snapshot.elements[0].size, Size::new(20, 20));
	}

	#[test]
	fn mounted_scope_cleanup_follows_ownership_after_reparenting() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let destination = root.element("destination").container(|c| c).await.id();
			root.element("scope")
				.mount(async move |ctx| {
					let mut child = ctx.element("child").container(|c| c).await;
					child.reparent(destination).await;
					child.element("grandchild").container(|c| c).await;
					ctx.render().await;
				})
				.await;
			root.element("replacement").container(|c| c.size(15.into())).await;
		});
		let first = engine.evaluate(Size::new(100, 100), &allocator);
		assert_eq!(first.elements.len(), 4);
		let survivors = [first.elements[0].id, first.elements[1].id];
		let second = engine.evaluate(Size::new(100, 100), &allocator);
		assert_eq!(second.elements.len(), 3);
		assert_eq!([second.elements[0].id, second.elements[1].id], survivors);
		assert_eq!(second.elements[2].size, Size::new(15, 15));
		assert_eq!(
			second.relations,
			&[(survivors[0], survivors[1]), (survivors[0], second.elements[2].id)]
		);
	}

	#[test]
	fn context_wait_wakes_from_runtime_frame_loop() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);

		engine.mount(async move |ctx| {
			ctx.wait(Duration::from_millis(1)).await;
			ctx.with(|hits| *hits += 1).await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 0);

		std::thread::sleep(Duration::from_millis(2));
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 1);
	}

	#[test]
	fn empty_retained_tree_does_not_panic() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		let snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert!(snapshot.elements.is_empty());
		assert_eq!(engine.render().size(), 0);
	}

	#[test]
	fn default_container_clip_skips_fully_clipped_descendants_in_render() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut parent = root.element("parent").container(|c| c.size(50.into())).await;
			parent
				.element("child")
				.container(|c| c.size(20.into()).absolute_position(70, 0))
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let ids = render.elements().map(|element| element.id).collect::<Vec<_>>();

		assert_eq!(ids.len(), 2);
		assert!(ids.contains(&1));
		assert!(ids.contains(&2));
	}

	#[test]
	fn default_container_clip_is_carried_to_partially_clipped_descendants() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut parent = root.element("parent").container(|c| c.size(50.into())).await;
			parent
				.element("child")
				.container(|c| c.size(30.into()).absolute_position(35, 10))
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let child = render
			.elements()
			.find(|element| element.id == 3)
			.expect("expected test value");

		assert_eq!(child.position, Location3::new(35, 10, 2));
		assert_eq!(child.size, Size::new(30, 30));
		assert_eq!(child.clip, Some(Geometry::new(Location3::new(0, 0, 1), Size::new(50, 50))));
	}

	#[test]
	fn clip_false_allows_descendant_render_overflow() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut parent = root.element("parent").container(|c| c.size(50.into()).clip(false)).await;
			parent
				.element("child")
				.container(|c| c.size(20.into()).absolute_position(70, 0))
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let child = render
			.elements()
			.find(|element| element.id == 3)
			.expect("expected test value");

		assert_eq!(child.position, Location3::new(70, 0, 2));
		assert_eq!(child.clip, Some(Geometry::new(Location3::new(0, 0, 0), Size::new(100, 100))));
	}

	#[test]
	fn retained_hits_preserve_clipping_and_ignore_decoration_after_frame_reset() {
		let mut allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.hit_testable(false)).await;
			let mut parent = root
				.element("parent")
				.container(|c| c.absolute_position(10, 10).size(40.into()))
				.await;
			let _child = parent
				.element("child")
				.container(|c| c.absolute_position(25, 25).size(40.into()))
				.await;
			let _decoration = root
				.element("decoration")
				.container(|c| c.absolute_position(0, 0).size(100.into()).hit_testable(false))
				.await;
			loop {
				ctx.render().await;
			}
		});
		let mut hits = crate::ui::intersection::HitTest::default();
		let inside = UiPoint::new(-0.2, 0.2);
		let outside = UiPoint::new(0.2, -0.2);
		let target = {
			let mut snapshot = engine.evaluate(Size::new(100, 100), &allocator);
			snapshot.retain_hit_test(&mut hits);
			assert_eq!(snapshot.click(outside), None);
			snapshot.click(inside).expect("the child's visible area should accept clicks")
		};
		allocator.reset();
		assert_eq!(hits.query(inside), Some(target));
		assert_eq!(hits.query(outside), None);
	}

	#[test]
	fn clipping_prunes_descendants_from_hit_testing() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);

		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut parent = root.element("parent").container(|c| c.size(50.into())).await;
			let mut child = parent
				.element("child")
				.container(|c| c.size(20.into()).absolute_position(70, 0))
				.await;

			loop {
				child.on(Events::Actuated).await;
				ctx.with(|hits| *hits += 1).await;
			}
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.5, 0.8));
		engine.update_click_state(true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 0);
	}

	#[test]
	fn clip_false_preserves_descendant_hit_testing_overflow() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);

		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut parent = root.element("parent").container(|c| c.size(50.into()).clip(false)).await;
			let mut child = parent
				.element("child")
				.container(|c| c.size(20.into()).absolute_position(70, 0))
				.await;

			loop {
				child.on(Events::Actuated).await;
				ctx.with(|hits| *hits += 1).await;
			}
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.5, 0.8));
		engine.update_click_state(true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 1);
	}

	#[test]
	fn clip_false_preserves_absolute_descendant_render_overflow() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.size(50.into()).clip(false)).await;
			root.element("toast")
				.container(|c| c.size(20.into()).depth(Depth::absolute(1)).absolute_position(70, 0))
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		let toast = render
			.elements()
			.find(|element| element.position.x() == 70.0)
			.expect("expected test value");

		assert_eq!(toast.size, Size::new(20, 20));
	}

	#[test]
	fn absolute_depth_container_escapes_ancestor_clip_in_render() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.size(50.into())).await;
			root.element("toast")
				.container(|c| c.size(20.into()).depth(Depth::absolute(1)).absolute_position(70, 0))
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		let toast = render
			.elements()
			.find(|element| element.position.x() == 70.0)
			.expect("expected test value");

		assert_eq!(toast.size, Size::new(20, 20));
		assert_eq!(toast.clip, None);
	}

	#[test]
	fn absolute_depth_container_escapes_ancestor_clip_in_hit_testing() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);

		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.size(50.into())).await;
			let mut toast = root
				.element("toast")
				.container(|c| c.size(20.into()).depth(Depth::absolute(1)).absolute_position(70, 0))
				.await;

			loop {
				toast.on(Events::Actuated).await;
				ctx.with(|hits| *hits += 1).await;
			}
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.5, 0.8));
		engine.update_click_state(true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 1);
	}

	#[test]
	fn retained_geometry_is_available_after_layout_evaluation() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(None::<Geometry>);

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c.clip(false)).await;
			let mut button = frame
				.element("button")
				.container(|c| c.width(30.into()).height(20.into()).absolute_position(12, 18))
				.await;

			assert_eq!(button.geometry().await, None);
			button.render().await;
			let value = button.geometry().await;
			ctx.with(|geometry| *geometry = value).await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), None);

		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), Some(Geometry::new(Location3::new(12, 18, 1), Size::new(30, 20))));
	}

	#[test]
	fn retained_geometry_updates_after_property_mutation() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(None::<Geometry>);

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c.clip(false)).await;
			let mut button = frame
				.element("button")
				.container(|c| c.width(30.into()).height(20.into()).absolute_position(12, 18))
				.await;

			button.render().await;

			button
				.update_container(|c| c.width(Sizing::pixels(40)).position((24, 36)))
				.await;
			button.render().await;
			let value = button.geometry().await;
			ctx.with(|geometry| *geometry = value).await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), Some(Geometry::new(Location3::new(24, 36, 1), Size::new(40, 20))));
	}

	#[test]
	fn wait_future_resumes_mounted_task_after_duration() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);

		engine.mount(async move |ctx| {
			ctx.wait(Duration::from_millis(5)).await;
			ctx.with(|hits| *hits += 1).await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 0);

		std::thread::sleep(Duration::from_millis(20));
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 1);
	}

	#[test]
	fn context_seconds_returns_timer_future() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);

		engine.mount(async move |ctx| {
			ctx.seconds(0).await;
			ctx.with(|hits| *hits += 1).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 1);
	}

	#[test]
	fn focused_key_goes_to_most_recent_focus_target() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context((0, 0));

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			frame
				.element("first")
				.component(async move |ctx| {
					let mut first = ctx.element("button").container(|c| c).await;
					first.request_focus().await;
					first.on_key(Key::Escape).await;
					ctx.with(|(first_hits, _)| *first_hits += 1).await;
				})
				.await;
			frame
				.element("second")
				.component(async move |ctx| {
					let mut second = ctx.element("button").container(|c| c).await;
					second.request_focus().await;
					second.on_key(Key::Escape).await;
					ctx.with(|(_, second_hits)| *second_hits += 1).await;
				})
				.await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(engine.ctx().0, 0);
		assert_eq!(engine.ctx().1, 1);
	}

	#[test]
	fn requesting_focus_again_moves_target_without_duplication() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context((0, 0));

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			let mut first = frame.element("first").container(|c| c).await;
			let mut second = frame.element("second").container(|c| c).await;
			first.request_focus().await;
			second.request_focus().await;
			first.request_focus().await;

			first.on_key(Key::Escape).await;
			ctx.with(|(first_hits, _)| *first_hits += 1).await;
			first.release_focus().await;
			ctx.render().await;
			second.on_key(Key::Escape).await;
			ctx.with(|(_, second_hits)| *second_hits += 1).await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		engine.update_key_state(Key::Escape, false);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(engine.ctx().0, 1);
		assert_eq!(engine.ctx().1, 1);
	}

	#[test]
	fn releasing_focus_reveals_previous_focus_target() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context((0, 0));

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			frame
				.element("first")
				.component(async move |ctx| {
					let mut first = ctx.element("button").container(|c| c).await;
					first.request_focus().await;
					first.on_key(Key::Escape).await;
					ctx.with(|(first_hits, _)| *first_hits += 1).await;
				})
				.await;
			frame
				.element("second")
				.component(async move |ctx| {
					let mut second = ctx.element("button").container(|c| c).await;
					second.request_focus().await;
					second.release_focus().await;
					second.on_key(Key::Escape).await;
					ctx.with(|(_, second_hits)| *second_hits += 1).await;
				})
				.await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(engine.ctx().0, 1);
		assert_eq!(engine.ctx().1, 0);
	}

	#[test]
	fn escape_release_does_not_wake_key_future() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);

		engine.mount(async move |ctx| {
			let mut button = ctx.element("button").container(|c| c).await;
			button.request_focus().await;
			button.on_key(Key::Escape).await;
			ctx.with(|hits| *hits += 1).await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, false);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 0);

		engine.update_key_state(Key::Escape, true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 1);
	}

	/// The `TestContext` struct is the application context these tests lend to components, with a slot for what they observe.
	struct TestContext {
		value: u32,
		seen: Vec<u32>,
	}

	impl TestContext {
		fn new(value: u32) -> Self {
			Self {
				value,
				seen: Vec::new(),
			}
		}
	}

	trait TestUiContext = Context<TestContext>;

	#[test]
	fn components_can_access_engine_context() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(TestContext::new(7));

		engine.mount(async move |ctx| {
			ctx.with(|c| c.seen.push(c.value)).await;

			ctx.element("child")
				.component(async move |ctx: &mut EvaluationContext<TestContext>| {
					ctx.with(|c| c.seen.push(c.value + 1)).await;
				})
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(engine.ctx().seen, vec![7, 8]);
	}

	#[test]
	fn mounted_component_can_access_engine_context() {
		async fn modal(ctx: &mut impl TestUiContext) -> u32 {
			ctx.with(|c| c.value).await
		}

		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(TestContext::new(11));

		engine.mount(async move |ctx| {
			let value = ctx.element("modal").mount(async move |ctx| modal(ctx).await).await;
			ctx.with(|c| c.seen.push(value)).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(engine.ctx().seen, vec![11]);
	}

	#[test]
	fn awaited_modal_blocks_caller_until_component_returns_value() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(None);

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			let value = frame
				.element("modal")
				.mount(async move |ctx| {
					let mut button = ctx.element("button").container(|c| c).await;
					button.on(Events::Actuated).await;
					42
				})
				.await;
			let value = Some(value);
			ctx.with(|result| *result = value).await;
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements.len(), 2);
		assert_eq!(*engine.ctx(), None);

		engine.set_cursor_position(UiPoint::zero());
		engine.update_click_state(true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), Some(42));
	}

	#[test]
	fn awaited_modal_subtree_is_removed_after_component_returns() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			frame
				.element("modal")
				.mount(async move |ctx| {
					let mut button = ctx.element("button").container(|c| c).await;
					button.on(Events::Actuated).await;
				})
				.await;
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements.len(), 2);

		engine.set_cursor_position(UiPoint::zero());
		engine.update_click_state(true);
		let during_close = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(during_close.elements.len(), 2);

		let after_close = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(after_close.elements.len(), 1);
	}

	#[test]
	fn dropping_pending_awaited_modal_removes_its_subtree() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			let mut modal = frame.element("modal").mount(async move |ctx| {
				let mut button = ctx.element("button").container(|c| c).await;
				button.on(Events::Actuated).await;
			});

			utils::r#async::select! {
				_ = modal => {}
				_ = ctx.render() => {}
			}
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements.len(), 2);

		let after_drop = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(after_drop.elements.len(), 1);
	}

	#[test]
	fn removing_focused_mounted_modal_reveals_previous_focus_target() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			frame
				.element("background")
				.component(async move |ctx| {
					let mut background = ctx.element("button").container(|c| c).await;
					background.request_focus().await;
					background.on_key(Key::Escape).await;
					ctx.with(|background_hits| *background_hits += 1).await;
				})
				.await;

			let mut modal = frame.element("modal").mount(async move |ctx| {
				let mut modal = ctx.element("window").container(|c| c).await;
				modal.request_focus().await;
				modal.on_key(Key::Escape).await;
			});

			utils::r#async::select! {
				_ = modal => {}
				_ = ctx.render() => {}
			}
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements.len(), 3);

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(second.elements.len(), 2);

		engine.update_key_state(Key::Escape, true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 1);
	}

	#[test]
	fn awaited_modal_can_return_cancelled_from_escape() {
		#[derive(Debug, PartialEq, Eq)]
		enum Result {
			Confirmed,
			Cancelled,
		}

		async fn modal<C: 'static>(ctx: &mut impl Context<C>) -> Result {
			let mut window = ctx.element("window").container(|c| c).await;
			let mut ok = window.element("ok").container(|c| c).await;
			window.request_focus().await;

			utils::r#async::select! {
				_ = ok.on(Events::Actuated) => Result::Confirmed,
				_ = window.on_key(Key::Escape) => Result::Cancelled,
			}
		}

		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(None);

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			let value = frame.element("modal").mount(async move |ctx| modal(ctx).await).await;
			let value = Some(value);
			ctx.with(|result| *result = value).await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), Some(Result::Cancelled));
	}

	#[test]
	// The retained-modal fixture intentionally nests async component declarations to exercise stable structural IDs.
	#[allow(clippy::excessive_nesting)]
	fn reopening_awaited_modal_reuses_stable_ids() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(Vec::new());

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			for _ in 0..2 {
				frame
					.element("modal")
					.mount(async move |ctx| {
						let mut button = ctx.element("button").container(|c| c).await;
						let id = button.id();
						ctx.with(|ids| ids.push(id)).await;
						button.on(Events::Actuated).await;
					})
					.await;
			}
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements.len(), 2);

		engine.set_cursor_position(UiPoint::zero());
		engine.update_click_state(true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(second.elements.len(), 2);

		let ids = engine.ctx();

		assert_eq!(ids.len(), 2);
		assert_eq!(ids[0], ids[1]);
	}

	#[test]
	fn awaited_modal_can_mount_absolute_depth_container_above_opener() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			frame.element("opener").container(|c| c).await;
			frame
				.element("modal")
				.mount(async move |ctx| {
					let mut modal = ctx
						.element("modal_container")
						.container(|c| c.depth(Depth::absolute(1)))
						.await;
					modal.element("button").container(|c| c).await;
					modal.on(Events::Actuated).await;
				})
				.await;
		});

		let snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(snapshot.elements.len(), 4);
		assert_eq!(snapshot.elements[0].position.z(), 0);
		assert_eq!(snapshot.elements[1].position.z(), 1);
		assert_eq!(snapshot.elements[2].position, Location3::new(0, 0, 2));
		assert_eq!(snapshot.elements[3].position.z(), 3);
	}

	#[test]
	fn awaited_modal_absolute_depth_container_escapes_opener_clip() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut opener = ctx.element("opener").container(|c| c.size(20.into())).await;
			opener
				.element("modal")
				.mount(async move |ctx| {
					let mut modal = ctx
						.element("modal_container")
						.container(|c| {
							c.width(80.into())
								.height(30.into())
								.depth(Depth::absolute(1))
								.absolute_position(30, 0)
						})
						.await;
					modal.on(Events::Actuated).await;
				})
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		let modal = render
			.elements()
			.find(|element| element.position.x() == 30.0)
			.expect("expected test value");

		assert_eq!(modal.size, Size::new(80, 30));
		assert_eq!(modal.clip, None);
	}

	#[test]
	fn render_orders_elements_by_resolved_depth() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c.clip(false)).await;
			let mut high = frame.element("high").container(|c| c.depth(10).size(20.into())).await;
			high.element("text").text("high", |t| t).await;
			let mut low = frame.element("low").container(|c| c.size(20.into())).await;
			low.element("text").text("low", |t| t).await;
			frame
				.element("tie")
				.container(|c| c.size(20.into()))
				.await
				.element("text")
				.text("tie", |t| t)
				.await;
			ctx.render().await;
			low.update_container(|c| c.opacity(0.5)).await;
			ctx.render().await;
			high.update_container(|c| c.depth(Depth::relative(0))).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let depths = render.elements().map(|element| element.position.z()).collect::<Vec<_>>();

		// Depth is the rank in the paint order, and each container is followed by its text.
		assert_eq!(depths, vec![0, 1, 3, 5]);
		let ids = render.elements().map(|element| element.id).collect::<Vec<_>>();
		assert_eq!(
			render.texts().map(|element| element.content.as_str()).collect::<Vec<_>>(),
			["low", "tie", "high"]
		);

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		assert_eq!(render.elements().map(|element| element.id).collect::<Vec<_>>(), ids);
		assert_eq!(
			render.texts().map(|element| element.content.as_str()).collect::<Vec<_>>(),
			["low", "tie", "high"]
		);

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		assert_eq!(
			render.elements().map(|element| element.id).collect::<Vec<_>>(),
			[ids[0], ids[3], ids[1], ids[2]]
		);
		assert_eq!(
			render.texts().map(|element| element.content.as_str()).collect::<Vec<_>>(),
			["high", "low", "tie"]
		);
	}

	#[test]
	fn render_uses_container_stored_style() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			ctx.element("frame")
				.container(|c| c.style(ConcreteLayer::default().color(RGBA::new(0.2, 0.3, 0.4, 1.0).into())))
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		assert_eq!(render.elements().next().unwrap().style.layers().len(), 1);
		assert_eq!(render.elements().next().unwrap().style.layers()[0].kind(), LayerKind::Fill);
		match Layer::fill(&render.elements().next().unwrap().style.layers()[0]) {
			Color::Value(color) => assert_eq!(*color, RGBA::new(0.2, 0.3, 0.4, 1.0)),
			_ => panic!("expected value color"),
		}
	}

	#[test]
	fn render_preserves_container_backdrop_blur_radius() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			ctx.element("frame")
				.container(|c| c.style(ConcreteLayer::default().backdrop_blur(18.0)))
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		assert_eq!(render.elements().next().unwrap().backdrop_blur_radius, 18.0);
	}

	#[test]
	fn render_backdrop_blur_does_not_change_opacity_or_clip() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			ctx.element("frame")
				.container(|c| {
					c.size(20.into())
						.opacity(0.5)
						.style(ConcreteLayer::default().backdrop_blur(12.0))
				})
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		assert_eq!(render.elements().next().unwrap().backdrop_blur_radius, 12.0);
		assert_eq!(render.elements().next().unwrap().opacity, 0.5);
		assert_eq!(render.elements().next().unwrap().clip, None);
	}

	#[test]
	fn render_preserves_layered_container_style() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			ctx.element("frame")
				.container(|c| {
					c.style(
						ConcreteStyle::new()
							.layer(ConcreteLayer::default().color(RGBA::new(0.2, 0.3, 0.4, 1.0).into()))
							.layer(
								ConcreteLayer::default()
									.color(RGBA::new(0.9, 0.8, 0.7, 1.0).into())
									.stroke(2.0),
							),
					)
				})
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		assert_eq!(render.elements().next().unwrap().style.layers().len(), 2);
		assert_eq!(render.elements().next().unwrap().style.layers()[0].kind(), LayerKind::Fill);
		assert_eq!(
			render.elements().next().unwrap().style.layers()[1].kind(),
			LayerKind::Stroke { width: 2.0 }
		);
	}

	#[test]
	fn feathered_layer_mask_propagates_to_descendant_elements_and_text() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx
				.element("frame")
				.container(|c| {
					c.width(50.into()).height(40.into()).style(
						ConcreteLayer::default()
							.color(RGBA::white().into())
							.feather(EdgeFeather::vertical(8.0)),
					)
				})
				.await;
			frame.element("child").container(|c| c.size(10.into())).await;
			frame.element("label").text("Masked", |t| t).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let parent = render
			.elements()
			.find(|element| element.id == 1)
			.expect("expected test value");
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");
		let text = render.texts().find(|text| text.id == 3).expect("expected test value");
		let expected = ClipMask {
			geometry: Geometry::new(Location3::new(0, 0, 0), Size::new(50, 40)),
			feather: EdgeFeather::vertical(8.0),
			corner_radius: 0.0,
			corner_exponent: 2.0,
		};

		assert_eq!(parent.clip_mask, None);
		assert_eq!(child.clip_mask, Some(expected));
		assert_eq!(text.clip_mask, Some(expected));
	}

	#[test]
	fn clip_false_prevents_layer_clip_mask_inheritance() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx
				.element("frame")
				.container(|c| {
					c.width(50.into())
						.height(40.into())
						.clip(false)
						.style(ConcreteLayer::default().feather(EdgeFeather::all(8.0)))
				})
				.await;
			frame.element("child").container(|c| c.size(10.into())).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");

		assert_eq!(child.clip_mask, None);
	}

	#[test]
	fn first_nonzero_feathered_layer_defines_descendant_mask() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx
				.element("frame")
				.container(|c| {
					c.width(50.into()).height(40.into()).style(
						ConcreteStyle::new()
							.layer(ConcreteLayer::default())
							.layer(ConcreteLayer::default().feather(EdgeFeather::horizontal(4.0)))
							.layer(ConcreteLayer::default().feather(EdgeFeather::vertical(9.0))),
					)
				})
				.await;
			frame.element("child").container(|c| c.size(10.into())).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");

		assert_eq!(
			child.clip_mask,
			Some(ClipMask {
				geometry: Geometry::new(Location3::new(0, 0, 0), Size::new(50, 40)),
				feather: EdgeFeather::horizontal(4.0),
				corner_radius: 0.0,
				corner_exponent: 2.0,
			})
		);
	}

	#[test]
	fn feathered_layer_mask_preserves_source_container_corner_shape() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx
				.element("frame")
				.container(|c| {
					c.width(50.into())
						.height(40.into())
						.corner_radius(8.0)
						.corner_exponent(4.0)
						.style(ConcreteLayer::default().feather(EdgeFeather::vertical(8.0)))
				})
				.await;
			frame.element("child").container(|c| c.size(10.into())).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");
		let mask = child.clip_mask.expect("expected test value");

		assert_eq!(mask.corner_radius, 8.0);
		assert_eq!(mask.corner_exponent, 4.0);
	}

	#[test]
	fn rounded_bordered_container_masks_descendants_inside_its_border() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx
				.element("frame")
				.container(|c| {
					c.width(50.into()).height(40.into()).corner_radius(8.0).style(
						ConcreteStyle::new()
							.layer(ConcreteLayer::default())
							.layer(ConcreteLayer::default().stroke(2.0)),
					)
				})
				.await;
			frame.element("child").container(|c| c.size(60.into())).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");
		let inside = Geometry::new(Location3::new(2, 2, 0), Size::new(46, 36));
		let mask = child.clip_mask.expect("expected test value");

		assert_eq!(child.clip, Some(inside));
		assert_eq!(mask.geometry.size, inside.size);
		assert_eq!(mask.feather, EdgeFeather::none());
		assert_eq!(mask.corner_radius, 6.0);
	}

	#[test]
	fn render_inherits_parent_opacity() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c.size(10.into()).opacity(0.5)).await;
			frame.element("child").container(|c| c.size(10.into())).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let parent = render
			.elements()
			.find(|element| element.id == 1)
			.expect("expected test value");
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");

		assert_eq!(parent.opacity, 0.5);
		assert_eq!(child.opacity, 0.5);
	}

	#[test]
	fn render_multiplies_nested_and_local_opacity() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c.size(10.into()).opacity(0.5)).await;
			frame.element("child").container(|c| c.size(10.into()).opacity(0.25)).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");

		assert_eq!(child.opacity, 0.125);
	}

	#[test]
	fn render_inherits_opacity_for_text() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c.size(10.into()).opacity(0.5)).await;
			frame
				.element("label")
				.text("Hello", |t| {
					t.style(ConcreteLayer::default().color(RGBA::new(1.0, 1.0, 1.0, 0.8).into()))
				})
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let text = render.texts().next().expect("expected test value");

		assert_eq!(text.opacity, 0.5);
		assert_eq!(text.color, RGBA::new(1.0, 1.0, 1.0, 0.8));
	}

	#[test]
	fn render_uses_shape_opacity_from_settings() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			ctx.element("shape").shape(|s| s.size(10.into()).opacity(0.4)).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		assert_eq!(render.elements().next().unwrap().opacity, 0.4);
	}

	#[test]
	fn render_sanitizes_opacity() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.size(10.into()).clip(false)).await;
			root.element("negative").container(|c| c.size(10.into()).opacity(-1.0)).await;
			root.element("invalid")
				.container(|c| c.size(10.into()).opacity(f32::NAN))
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		assert_eq!(
			render
				.elements()
				.find(|element| element.id == 2)
				.expect("expected test value")
				.opacity,
			0.0
		);
		assert_eq!(
			render
				.elements()
				.find(|element| element.id == 3)
				.expect("expected test value")
				.opacity,
			1.0
		);
	}

	#[test]
	fn render_uses_container_corner_exponent() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			ctx.element("frame")
				.container(|c| c.corner_radius(8.0).corner_exponent(4.0))
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		assert_eq!(render.elements().next().unwrap().corner_radius, 8.0);
		assert_eq!(render.elements().next().unwrap().corner_exponent, 4.0);
	}

	#[test]
	fn render_uses_shape_corner_exponent_from_settings() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			ctx.element("shape")
				.shape(|s| s.size(20.into()).corner_radius(6.0).corner_exponent(4.0))
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		assert_eq!(render.elements().next().unwrap().corner_radius, 6.0);
		assert_eq!(render.elements().next().unwrap().corner_exponent, 4.0);
	}

	#[test]
	fn render_uses_container_transform_after_layout() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			ctx.element("frame")
				.container(|c| {
					c.width(20.into())
						.height(10.into())
						.transform(Transform::identity().translate_y(6.0).scale(0.5))
				})
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let frame = render.elements().next().expect("expected test value");

		assert_eq!(frame.position, Location3::new(5.0, 8.5, 0));
		assert_eq!(frame.size, Size::new(10, 5));
	}

	#[test]
	fn child_visual_bounds_inherit_parent_transform() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx
				.element("frame")
				.container(|c| {
					c.size(100.into())
						.flow(flow::row)
						.transform(Transform::identity().translate_y(10.0).scale(0.5))
				})
				.await;
			frame
				.element("child")
				.container(|c| c.width(20.into()).height(10.into()))
				.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");

		assert_eq!(child.position, Location3::new(25, 35, 1));
		assert_eq!(child.size, Size::new(10, 5));
	}

	#[test]
	fn hit_testing_uses_transformed_visual_bounds() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);

		engine.mount(async move |ctx| {
			let mut button = ctx
				.element("button")
				.container(|c| c.size(20.into()).transform(Transform::identity().translate(40.0, 40.0)))
				.await;

			loop {
				button.on(Events::Actuated).await;
				ctx.with(|hits| *hits += 1).await;
			}
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.0, 0.0));
		engine.update_click_state(true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 1);
	}

	#[test]
	fn opacity_does_not_disable_hit_testing() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);

		engine.mount(async move |ctx| {
			let mut button = ctx.element("button").container(|c| c.opacity(0.0)).await;

			loop {
				button.on(Events::Actuated).await;
				ctx.with(|hits| *hits += 1).await;
			}
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.0, 0.0));
		engine.update_click_state(true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), 1);
	}

	#[test]
	fn update_container_changes_later_layout_and_render_style() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c.size(10.into())).await;
			frame.render().await;

			frame
				.update_container(|c| {
					c.width(Sizing::pixels(30))
						.style(ConcreteLayer::default().color(RGBA::new(0.4, 0.5, 0.6, 1.0).into()))
				})
				.await;
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements[0].size, Size::new(10, 10));

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(second.elements[0].size, Size::new(30, 10));

		let render = engine.render();
		match Layer::fill(&render.elements().next().unwrap().style.layers()[0]) {
			Color::Value(color) => assert_eq!(*color, RGBA::new(0.4, 0.5, 0.6, 1.0)),
			_ => panic!("expected value color"),
		}
	}

	#[test]
	fn update_container_changes_later_render_opacity() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			frame.render().await;

			frame.update_container(|c| c.opacity(0.25)).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();

		assert_eq!(render.elements().next().unwrap().opacity, 0.25);
	}

	#[test]
	fn next_tick_is_now_while_a_component_waits_for_frames() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let _frame = ctx.element("frame").container(|c| c).await;
			loop {
				ctx.render().await;
			}
		});
		engine.evaluate(Size::new(100, 100), &allocator);

		assert!(engine.next_tick().is_some_and(|tick| tick <= std::time::Instant::now()));
	}

	#[test]
	fn next_tick_follows_a_pending_ui_timer() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			ctx.wait(std::time::Duration::from_secs(3600)).await;
			frame.on(Events::Actuated).await;
		});
		engine.evaluate(Size::new(100, 100), &allocator);

		let tick = engine.next_tick().expect("A pending UI timer did not schedule a tick.");
		let now = std::time::Instant::now();
		assert!(tick > now + std::time::Duration::from_secs(3500));
		assert!(tick <= now + std::time::Duration::from_secs(3600));
	}

	/// Mounts a container, text, curve, and image, and applies the edit selected by `edit` on every frame.
	fn mount_edited_elements(engine: &mut Engine<u8>) {
		engine.mount(async move |ctx| {
			let mut frame = ctx
				.element("frame")
				.container(|c| {
					c.size(40.into())
						.style(ConcreteLayer::default().color(RGBA::new(0.1, 0.2, 0.3, 1.0).into()))
				})
				.await;
			let mut label = ctx.element("label").text("Hello", |t| t).await;
			let mut wire = ctx
				.element("wire")
				.curve(|c| {
					c.size(100.into())
						.line((0.0, 0.0), (10.0, 10.0))
						.style(ConcreteLayer::default().stroke(2.0))
				})
				.await;
			let mut picture = ctx.element("picture").image(1, 1, vec![255; 4], |i| i).await;
			loop {
				ctx.render().await;
				match ctx.with(|edit| std::mem::replace(edit, 0)).await {
					// Write the values already present.
					1 => {
						frame
							.update_container(|c| {
								c.width(Sizing::pixels(40))
									.opacity(1.0)
									.style(ConcreteLayer::default().color(RGBA::new(0.1, 0.2, 0.3, 1.0).into()))
							})
							.await;
						label.update_text(|t| t.content("Hello").opacity(1.0)).await;
						wire.update_curve(|c| c.clear_segments().line((0.0, 0.0), (10.0, 10.0))).await;
						picture.update_image(|i| i.opacity(1.0)).await;
					}
					// Change a value and restore it within the same edit.
					2 => {
						label.update_text(|t| t.content("Changed").content("Hello")).await;
					}
					3 => {
						frame
							.update_container(|c| c.style(ConcreteLayer::default().color(RGBA::new(0.9, 0.2, 0.3, 1.0).into())))
							.await;
					}
					4 => {
						picture.update_image(|i| i.pixels(1, 1, vec![255; 4])).await;
					}
					_ => {}
				}
			}
		});
	}

	#[test]
	fn updates_that_change_nothing_keep_the_render_revision() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(0);
		mount_edited_elements(&mut engine);
		let snapshot = engine.evaluate(Size::new(100, 100), &allocator);
		let first = engine.render().revision();

		for step in [1, 2] {
			*engine.ctx_mut() = step;
			engine.evaluate(Size::new(100, 100), &allocator);
			assert_eq!(
				engine.render().revision(),
				first,
				"Edit {step} changed the render revision."
			);
		}
	}

	#[test]
	fn updates_that_change_style_or_image_contents_advance_the_render_revision() {
		for step in [3, 4] {
			let allocator = bumpalo::Bump::new();
			let mut engine = Engine::with_context(0);
			mount_edited_elements(&mut engine);
			let snapshot = engine.evaluate(Size::new(100, 100), &allocator);
			let first = engine.render().revision();

			*engine.ctx_mut() = step;
			engine.evaluate(Size::new(100, 100), &allocator);
			assert_ne!(
				engine.render().revision(),
				first,
				"Edit {step} kept the render revision."
			);
		}
	}

	#[test]
	fn update_text_changes_later_render_style() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut text = ctx.element("label").text("Hello", |t| t).await;
			text.render().await;

			text.update_text(|t| {
				t.content("Updated")
					.style(ConcreteLayer::default().color(RGBA::new(0.7, 0.8, 0.9, 1.0).into()))
			})
			.await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let text = render.texts().next().expect("expected test value");

		assert_eq!(text.content, "Updated");
		assert_eq!(text.color, RGBA::new(0.7, 0.8, 0.9, 1.0));
	}

	#[test]
	fn text_edit_applies_to_app_owned_string() {
		let mut content = String::from("Hi");

		TextEdit::Inserted('é').apply_to(&mut content);

		assert_eq!(content, "Hié");

		TextEdit::Deleted('é').apply_to(&mut content);

		assert_eq!(content, "Hi");

		TextEdit::Deleted('x').apply_to(&mut content);

		assert_eq!(content, "Hi");
	}

	#[test]
	fn text_field_renders_visible_content() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			ctx.element("field").text_field("Hello", |f| f).await;
		});

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		let text = render.texts().next().expect("expected test value");

		assert_eq!(text.content, "Hello");
		assert!(text.size.x() > 0.0);
		assert!(render.texts().nth(1).is_none());
	}

	#[test]
	fn focused_text_field_receives_inserted_text_edit() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(None);

		engine.mount(async move |ctx| {
			let mut field = ctx.element("field").text_field("", |f| f).await;
			field.request_focus().await;
			let event = field.on_text_edit().await;
			let value = Some(event.edit);
			ctx.with(|received| *received = value).await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.input_character('a');
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), Some(TextEdit::Inserted('a')));
	}

	#[test]
	fn unfocused_text_field_does_not_receive_inserted_text_edit() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(None);

		engine.mount(async move |ctx| {
			let mut field = ctx.element("field").text_field("", |f| f).await;
			let event = field.on_text_edit().await;
			let value = Some(event.edit);
			ctx.with(|received| *received = value).await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.input_character('a');
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), None);
	}

	#[test]
	fn focused_text_field_delete_emits_deleted_last_character() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(None);

		engine.mount(async move |ctx| {
			let mut field = ctx.element("field").text_field("Hié", |f| f).await;
			field.request_focus().await;
			let event = field.on_text_edit().await;
			let value = Some(event.edit);
			ctx.with(|received| *received = value).await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.delete_text_backward();
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*engine.ctx(), Some(TextEdit::Deleted('é')));
	}

	#[test]
	fn app_owned_string_update_changes_later_text_field_render() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(String::from("a"));

		engine.mount(async move |ctx| {
			let initial = ctx.with(|content| content.clone()).await;
			let mut field = ctx.element("field").text_field(initial, |f| f).await;
			field.request_focus().await;
			let event = field.on_text_edit().await;
			let updated = ctx
				.with(|content| {
					event.edit.apply_to(content);
					content.clone()
				})
				.await;
			field.update_text_field(|f| f.content(updated)).await;
			field.render().await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.input_character('b');
		engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let content = engine.ctx().clone();
		let render = engine.render();
		let text = render.texts().next().expect("expected test value");

		assert_eq!(content, "ab");
		assert_eq!(text.content, "ab");
	}

	#[test]
	fn centered_flow_overlays_full_size_curve_children() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx
				.element("frame")
				.container(|c| c.width(100.into()).height(50.into()).flow(flow::center))
				.await;
			frame
				.element("first")
				.curve(|c| c.width(100.into()).height(50.into()).line((0.0, 10.0), (100.0, 10.0)))
				.await;
			frame
				.element("second")
				.curve(|c| {
					c.width(100.into())
						.height(50.into())
						.quadratic((0.0, 40.0), (50.0, 0.0), (100.0, 40.0))
				})
				.await;
		});

		engine.evaluate(Size::new(200, 100), &frame_allocator);
		let render = engine.render();
		let curves: std::vec::Vec<_> = render.curves().collect();

		assert_eq!(curves.len(), 2);
		assert_eq!(
			(curves[0].position.x(), curves[0].position.y()),
			(curves[1].position.x(), curves[1].position.y())
		);
		assert_eq!(curves[0].size, Size::new(100, 50));
		assert_eq!(curves[1].size, Size::new(100, 50));
	}

	#[test]
	fn owned_names_declare_distinct_elements_and_repeat_by_content() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(std::vec::Vec::new());
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut ids = std::vec::Vec::new();
			for index in 0..3 {
				let node = root.element(format!("node-{index}")).container(|c| c.size(10.into())).await;
				ids.push(node.id());
			}
			ctx.with(|out| *out = ids.clone()).await;
			ctx.render().await;
			// The same content on a later frame resolves the same retained element.
			let again = root.element(String::from("node-1")).container(|c| c).await;
			ids.push(again.id());
			ctx.with(|out| *out = ids).await;
		});
		engine.evaluate(Size::new(100, 100), &allocator);
		engine.evaluate(Size::new(100, 100), &allocator);
		let ids = std::mem::take(engine.ctx_mut());
		assert_eq!(ids.len(), 4);
		assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2]);
		assert_eq!(ids[3], ids[1]);
	}

	#[test]
	fn removing_an_element_drops_its_subtree_and_ends_components_declared_under_it() {
		let allocator = bumpalo::Bump::new();
		/// The `Removal` struct carries what the test drives and observes through the engine context.
		#[derive(Default)]
		struct Removal {
			ticks: u32,
			/// 0 idle, 1 remove the node, 2 declare it again.
			stage: u8,
			ids: Option<(Id, Id, Id)>,
		}
		let mut engine = Engine::with_context(Removal::default());
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut node = root.element("node").container(|c| c.size(10.into())).await;
			let label = node.element("label").text("node", |t| t).await;
			node.element("ticker")
				.component(async move |ctx| {
					loop {
						ctx.with(|state| state.ticks += 1).await;
						ctx.render().await;
					}
				})
				.await;
			let keep = root.element("keep").container(|c| c.size(10.into())).await;
			let ids = (node.id(), label.id(), keep.id());
			ctx.with(|state| state.ids = Some(ids)).await;
			loop {
				ctx.render().await;
				match ctx.with(|state| std::mem::replace(&mut state.stage, 0)).await {
					1 => {
						assert!(node.geometry().await.is_some());
						node.remove().await;
						// Removing twice and editing a removed element change nothing.
						node.remove().await;
						node.update_container(|c| c.opacity(1.0)).await;
						// The next frame lays the tree out without the node.
						ctx.render().await;
						assert!(node.geometry().await.is_none());
					}
					2 => {
						let again = root.element("node").container(|c| c.size(10.into())).await;
						assert_eq!(again.id(), node.id(), "Declaring the name again did not reuse its id.");
					}
					_ => {}
				}
			}
		});
		// Runs `count` frames and returns the ids the last one laid out.
		let frames = |engine: &mut Engine<Removal>, count: usize| {
			for _ in 1..count {
				engine.evaluate(Size::new(100, 100), &allocator);
			}
			let snapshot = engine.evaluate(Size::new(100, 100), &allocator);
			snapshot.elements.iter().map(|element| element.id).collect::<Vec<_>>()
		};
		let contains = |ids: &Vec<Id>, id: Id| ids.contains(&id);

		let snapshot = frames(&mut engine, 3);
		let (node, label, keep) = engine.ctx().ids.expect("expected test value");
		assert!(contains(&snapshot, node) && contains(&snapshot, label) && contains(&snapshot, keep));
		assert!(engine.ctx().ticks > 0);

		engine.ctx_mut().stage = 1;
		let snapshot = frames(&mut engine, 2);
		assert!(!contains(&snapshot, node) && !contains(&snapshot, label));
		assert!(contains(&snapshot, keep), "Removing one element removed its sibling.");
		let closed = engine.ctx().ticks;
		frames(&mut engine, 3);
		assert_eq!(
			engine.ctx().ticks,
			closed,
			"A component kept running after its element was removed."
		);

		engine.ctx_mut().stage = 2;
		let snapshot = frames(&mut engine, 2);
		assert!(
			contains(&snapshot, node),
			"A removed element declared again was not laid out."
		);
	}

	#[test]
	fn update_curve_repaints_segments_without_replaying_placement() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(false);
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c).await;
			let mut wire = root
				.element("wire")
				.curve(|c| {
					c.size(100.into())
						.line((0.0, 0.0), (10.0, 10.0))
						.style(ConcreteLayer::default().stroke(2.0))
				})
				.await;
			loop {
				ctx.render().await;
				if ctx.with(|reroute| std::mem::replace(reroute, false)).await {
					wire.update_curve(|c| c.clear_segments().cubic((0.0, 0.0), (5.0, 0.0), (5.0, 10.0), (10.0, 10.0)))
						.await;
				}
			}
		});
		let snapshot = engine.evaluate(Size::new(100, 100), &allocator);
		let render = engine.render();
		let first = render.revision();
		let curve = render.curves().next().unwrap();
		assert!(matches!(curve.segments.as_slice(), [CurveSegment::Line { .. }]));
		let placement = engine.core.tree.placement_revision;

		*engine.ctx_mut() = true;
		engine.evaluate(Size::new(100, 100), &allocator);
		let render = engine.render();
		assert_ne!(render.revision(), first);
		let curve = render.curves().next().unwrap();
		assert!(matches!(curve.segments.as_slice(), [CurveSegment::Cubic { .. }]));
		assert_eq!(
			engine.core.tree.placement_revision, placement,
			"Re-routing a curve replayed placement."
		);
	}

	#[test]
	fn scaled_ancestor_scales_curves_and_text_in_the_render() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.hit_testable(false)).await;
			root.element("plain")
				.curve(|c| {
					c.size(10.into())
						.line((0.0, 0.0), (10.0, 10.0))
						.style(ConcreteLayer::default().stroke(1.0))
				})
				.await;
			let mut canvas = root
				.element("canvas")
				.container(|canvas| {
					canvas
						.absolute_position(20, 30)
						.size(100.into())
						// Overlay the wire and the label instead of flowing the label out of the clip.
						.flow(flow::center)
						.transform(Transform::identity().origin(UiPoint::zero()).scale_xy(2.0, 3.0))
				})
				.await;
			canvas
				.element("wire")
				.curve(|c| {
					c.size(100.into())
						.line((0.0, 0.0), (10.0, 10.0))
						.style(ConcreteLayer::default().stroke(1.0))
				})
				.await;
			canvas.element("label").text("node", |t| t.font_size(10.0)).await;
		});
		engine.evaluate(Size::new(400, 400), &allocator);
		let render = engine.render();
		let curves: std::vec::Vec<_> = render.curves().collect();
		assert_eq!(curves.len(), 2);
		assert_eq!(curves[0].scale, [1.0, 1.0]);
		assert_eq!(curves[1].scale, [2.0, 3.0]);
		assert_eq!((curves[1].position.x(), curves[1].position.y()), (20.0, 30.0));
		let text = render.texts().next().unwrap();
		assert_eq!(text.scale, 2.0, "Text takes the smaller axis of an anisotropic scale.");
	}

	/// The engine context of the hover tests: every hover event a surface received, in order.
	type HoverLog = std::vec::Vec<(&'static str, Events)>;

	/// Records every hover event a surface receives, in order, into the engine's [`HoverLog`].
	async fn hover_log(ctx: &mut EvaluationContext<HoverLog>, name: &'static str) {
		ctx.element("hover")
			.component(async move |ctx| {
				loop {
					let event = utils::r#async::select! {
						event = ctx.on(Events::PointerEntered) => event,
						event = ctx.on(Events::PointerExited) => event,
					};
					ctx.with(|log| log.push((name, event.kind))).await;
				}
			})
			.await;
	}

	/// An event that fires while its component is not awaiting it is discarded, even when that component
	/// awaited the same event before and dropped the wait, as a losing `select!` branch does.
	#[test]
	fn dropping_an_event_wait_discards_events_until_the_next_wait() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(std::vec::Vec::new());
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.hit_testable(false)).await;
			let mut target = root
				.element("target")
				.container(|c| c.absolute_position(0, 0).size(50.into()))
				.await;
			// Wait for one click, start and abandon a second wait, then look away for frames.
			let _ = target.on(Events::Actuated).await;
			ctx.with(|log| log.push("first")).await;
			let mut abandoned = target.on(Events::Actuated);
			let unexpected = utils::r#async::select! {
				_ = abandoned => true,
				_ = ctx.render() => false,
			};
			if unexpected {
				ctx.with(|log| log.push("unexpected")).await;
			}
			drop(abandoned);
			for _ in 0..3 {
				ctx.render().await;
			}
			let _ = target.on(Events::Actuated).await;
			ctx.with(|log| log.push("second")).await;
		});
		let frame = |engine: &mut Engine<std::vec::Vec<&'static str>>, click: bool| {
			engine.set_cursor_position(UiPoint::new(-0.5, 0.5));
			if click {
				engine.update_click_state(true);
				engine.update_click_state(false);
			}
			engine.evaluate(Size::new(100, 100), &allocator);
		};
		frame(&mut engine, false);
		frame(&mut engine, true);
		assert_eq!(*engine.ctx(), vec!["first"]);
		// The abandoned wait ends this frame; clicks during the look-away frames must not replay later.
		frame(&mut engine, false);
		frame(&mut engine, true);
		frame(&mut engine, true);
		frame(&mut engine, false);
		frame(&mut engine, false);
		assert_eq!(*engine.ctx(), vec!["first"], "a click fired while no wait was live was kept");
		frame(&mut engine, true);
		assert_eq!(*engine.ctx(), vec!["first", "second"]);
	}

	/// While a source is held, the pointer still enters and leaves other surfaces, including ones that
	/// appear or move under it during the gesture, as a menu opened by the press does. A sector that
	/// grows open frame by frame is hit as it is now, not as it was when the element was placed.
	#[test]
	fn hover_follows_the_pointer_while_a_source_is_held() {
		for (hold, sector) in [(false, false), (false, true), (true, false), (true, true)] {
			held_hover_probe(hold, sector);
		}
	}

	fn held_hover_probe(hold: bool, sector: bool) {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(HoverLog::default());
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.hit_testable(false)).await;
			let mut source = root
				.element("source")
				.container(|c| c.absolute_position(0, 0).size(20.into()))
				.await;
			// A dial parked out of the way until the press moves it under the pointer's path, holding a
			// sector petal that grows open over the frames after the press.
			let mut dial = root
				.element("dial")
				.container(|c| {
					c.depth(Depth::absolute(3))
						.absolute_position(500, 500)
						.size(100.into())
						.clip(false)
						.hit_testable(false)
				})
				.await;
			let mut target = dial
				.element("target")
				.container(|target| {
					if sector {
						target
							.absolute_position(0, 0)
							.size(100.into())
							.clip(false)
							.sector(crate::ui::Sector::new(0.0, 0.0, 0.3))
					} else {
						target.absolute_position(50, 50).size(50.into()).clip(false)
					}
				})
				.await;
			if hold {
				let _ = source.on(Events::Grabbed).await;
			} else {
				let _ = source.on(Events::Actuated).await;
			}
			dial.update_container(|c| c.position((0, 0))).await;
			let mut step = 0;
			loop {
				let event = utils::r#async::select! {
					event = target.on(Events::PointerEntered) => Some(event.kind),
					event = target.on(Events::PointerExited) => Some(event.kind),
					_ = ctx.render() => None,
				};
				match event {
					Some(kind) => ctx.with(|log| log.push(("target", kind))).await,
					None if sector && step < 4 => {
						step += 1;
						let sweep = std::f32::consts::FRAC_PI_2 * step as f32 / 4.0;
						target
							.update_container(|c| {
								c.sector(Some(crate::ui::Sector::new(
									std::f32::consts::FRAC_PI_4 - sweep * 0.5,
									sweep,
									0.3,
								)))
							})
							.await;
					}
					None => {}
				}
			}
		});
		let window = |x: f32, y: f32| UiPoint::new(x / 50.0 - 1.0, 1.0 - y / 50.0);
		let frame = |engine: &mut Engine<HoverLog>, position: UiPoint| {
			engine.set_cursor_position(position);
			engine.drag_to(position);
			for _ in 0..2 {
				engine.evaluate(Size::new(100, 100), &allocator);
				let _ = engine.render();
			}
			std::mem::take(engine.ctx_mut())
		};
		frame(&mut engine, window(10.0, 10.0));
		if hold {
			assert!(engine.press(window(10.0, 10.0)), "the source is under the press");
		} else {
			engine.update_click_state(true);
			engine.update_click_state(false);
		}
		for _ in 0..4 {
			frame(&mut engine, window(10.0, 10.0));
		}
		// The petal sweeps the lower-right quadrant of the dial centered at (50, 50).
		assert_eq!(
			frame(&mut engine, window(80.0, 80.0)),
			vec![("target", Events::PointerEntered)],
			"the held pointer entered the petal that opened under it"
		);
		assert_eq!(
			frame(&mut engine, window(20.0, 80.0)),
			vec![("target", Events::PointerExited)],
			"the held pointer left the petal"
		);
	}

	#[test]
	fn pointer_enter_and_exit_follow_the_hovered_surface_and_its_ancestors() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(HoverLog::default());
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.hit_testable(false)).await;
			let mut panel = root
				.element("panel")
				.container(|c| c.absolute_position(0, 0).width(100.into()).height(50.into()).flow(flow::row))
				.await;
			hover_log(&mut panel, "panel").await;
			let mut left = panel.element("left").container(|c| c.size(50.into())).await;
			hover_log(&mut left, "left").await;
			let mut right = panel.element("right").container(|c| c.size(50.into())).await;
			hover_log(&mut right, "right").await;
		});
		let window = |x: f32, y: f32| UiPoint::new(x / 50.0 - 1.0, 1.0 - y / 50.0);
		let mut frame = |position: UiPoint| {
			engine.set_cursor_position(position);
			let _ = engine.evaluate(Size::new(100, 100), &allocator);
			engine.evaluate(Size::new(100, 100), &allocator);
			std::mem::take(engine.ctx_mut())
		};

		// The first frames only register the waiters.
		frame(window(200.0, 200.0));
		assert_eq!(
			frame(window(10.0, 10.0)),
			vec![("left", Events::PointerEntered), ("panel", Events::PointerEntered)]
		);
		// Moving between siblings leaves the shared ancestor alone.
		assert_eq!(
			frame(window(60.0, 10.0)),
			vec![("left", Events::PointerExited), ("right", Events::PointerEntered)]
		);
		assert_eq!(frame(window(60.0, 20.0)), vec![], "Motion inside a surface produced events.");
		assert_eq!(
			frame(window(200.0, 200.0)),
			vec![("right", Events::PointerExited), ("panel", Events::PointerExited)]
		);
	}

	#[test]
	fn a_held_source_is_skipped_so_the_surface_beneath_it_is_hovered_and_previewed() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context(HoverLog::default());
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.hit_testable(false)).await;
			let mut target = root
				.element("target")
				.container(|c| c.absolute_position(50, 0).width(50.into()).height(100.into()))
				.await;
			hover_log(&mut target, "target").await;
			root.element("card")
				.container(|c| c.absolute_position(0, 0).size(40.into()).depth(Depth::absolute(1)))
				.await;
		});
		let window = |x: f32, y: f32| UiPoint::new(x / 50.0 - 1.0, 1.0 - y / 50.0);
		// Start over the card, which logs nothing, so only the target's events are observed.
		engine.set_cursor_position(window(10.0, 10.0));
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		let card = engine.hit(window(10.0, 10.0)).unwrap();
		let target = engine.hit(window(75.0, 50.0)).unwrap();
		assert!(engine.press(window(10.0, 10.0)));
		// The card stays under the pointer, but the held source is skipped.
		assert!(engine.drag_to(window(20.0, 20.0)));
		engine.set_cursor_position(window(20.0, 20.0));
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		assert_eq!(engine.hovered(), None);
		assert_eq!(engine.drag().unwrap().over, None);
		assert_eq!(engine.drag().unwrap().source, card);

		engine.set_cursor_position(window(75.0, 50.0));
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		engine.evaluate(Size::new(100, 100), &allocator);
		assert_eq!(engine.hovered(), Some(target));
		assert_eq!(engine.drag().unwrap().over, Some(target));
		assert_eq!(std::mem::take(engine.ctx_mut()), vec![("target", Events::PointerEntered)]);
	}

	#[test]
	fn anchored_children_keep_negative_positions_through_transforms_and_hits() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.hit_testable(false)).await;
			let mut canvas = root
				.element("canvas")
				.container(|c| {
					c.absolute_position(0, 0)
						.hit_testable(false)
						.clip(false)
						.transform(Transform::identity().origin(UiPoint::zero()).translate(-30.0, 0.0))
				})
				.await;
			canvas
				.element("node")
				.container(|c| c.absolute_position(-20, -10).width(80.into()).height(40.into()))
				.await;
		});
		engine.evaluate(Size::new(100, 100), &allocator);
		let render = engine.render();
		let node = render.elements().find(|element| element.size == Size::new(80, 40)).unwrap();
		assert_eq!((node.position.x(), node.position.y()), (-50.0, -10.0));
		// Only the part inside the viewport can be hit.
		assert!(engine.hit(UiPoint::new(10.0 / 50.0 - 1.0, 1.0 - 10.0 / 50.0)).is_some());
		assert!(engine.hit(UiPoint::new(40.0 / 50.0 - 1.0, 1.0 - 10.0 / 50.0)).is_none());
	}

	#[test]
	fn hit_testable_curves_are_found_along_their_stroke_and_scale_with_ancestors() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(async move |ctx| {
			let mut root = ctx.element("root").container(|c| c.hit_testable(false)).await;
			let mut frame = root
				.element("frame")
				.container(|c| c.absolute_position(0, 0).size(100.into()).flow(flow::center))
				.await;
			frame
				.element("decorative")
				.curve(|c| c.size(100.into()).line((0.0, 90.0), (100.0, 90.0)))
				.await;
			frame
				.element("wire")
				.curve(|c| c.size(100.into()).line((0.0, 0.0), (100.0, 100.0)).hit_width(8.0))
				.await;
			let mut zoomed = root
				.element("zoomed")
				.container(|c| {
					c.absolute_position(100, 0)
						.size(50.into())
						.hit_testable(false)
						.clip(false)
						.transform(Transform::identity().origin(UiPoint::zero()).scale(2.0))
				})
				.await;
			zoomed
				.element("wire")
				.curve(|c| c.size(50.into()).line((0.0, 50.0), (50.0, 0.0)).hit_width(4.0))
				.await;
		});
		let window = |x: f32, y: f32| UiPoint::new(x / 100.0 - 1.0, 1.0 - y / 100.0);
		let mut hits = crate::ui::intersection::HitTest::default();
		engine.evaluate(Size::new(200, 200), &allocator).retain_hit_test(&mut hits);
		let frame = engine.hit(window(90.0, 10.0)).unwrap();
		let wire = engine.hit(window(50.0, 50.0)).unwrap();
		assert_ne!(wire, frame, "The wire was not hit on its stroke.");
		assert_eq!(
			engine.hit(window(50.0, 53.0)),
			Some(wire),
			"A point within the hit width missed."
		);
		assert_eq!(
			engine.hit(window(50.0, 58.0)),
			Some(frame),
			"A point beyond the hit width hit the wire."
		);
		assert_eq!(
			engine.hit(window(50.0, 90.0)),
			Some(frame),
			"A curve without a hit width was hit."
		);

		// The zoomed wire runs from (100, 100) to (200, 0) in layout units.
		let zoomed = engine.hit(window(150.0, 50.0)).expect("the scaled wire accepts the pointer");
		assert!(zoomed != wire && zoomed != frame);
		assert_eq!(engine.hit(window(150.0, 55.0)), Some(zoomed), "The hit width did not scale.");
		assert_eq!(engine.hit(window(150.0, 60.0)), None);

		assert_eq!(hits.query(window(50.0, 50.0)), Some(wire));
		assert_eq!(hits.query(window(150.0, 50.0)), Some(zoomed));
	}

	#[test]
	fn animate_updates_existing_retained_element_across_frames() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c.size(10.into())).await;
			animate(&mut frame, spring(0.0, 1.0), async |frame, t| {
				frame
					.update_container(|c| c.width(Sizing::pixels(10 + (90.0 * t.clamp(0.0, 1.0)) as u32)))
					.await
			})
			.await;
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements.len(), 1);
		assert_eq!(first.elements[0].size, Size::new(10, 10));

		std::thread::sleep(Duration::from_millis(20));
		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(second.elements.len(), 1);
		assert!(second.elements[0].size.x() > 10.0);
	}

	#[test]
	fn backdrop_style_modal_receives_actuated_event() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::with_context((0, 0));

		engine.mount(async move |ctx| {
			let mut frame = ctx.element("frame").container(|c| c).await;
			frame
				.element("background")
				.component(async move |ctx| {
					let mut background = ctx.element("button").container(|c| c).await;
					background.on(Events::Actuated).await;
					ctx.with(|(_, background_hits)| *background_hits += 1).await;
				})
				.await;
			frame
				.element("modal")
				.mount(async move |ctx| {
					let mut backdrop = ctx.element("backdrop").container(|c| c.depth(Depth::absolute(1))).await;
					backdrop.on(Events::Actuated).await;
					ctx.with(|(hits, _)| *hits += 1).await;
				})
				.await;
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::zero());
		engine.update_click_state(true);
		engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(engine.ctx().0, 1);
		assert_eq!(engine.ctx().1, 0);
	}
}

use std::{
	borrow::Cow,
	boxed::Box,
	collections::{HashMap, HashSet, VecDeque},
	future::Future,
	marker::PhantomData,
	pin::Pin,
	sync::{
		Arc,
		mpsc::{Receiver, Sender},
	},
	task::{Context as TaskContext, Poll, Wake, Waker},
};

use smallvec::SmallVec;
use utils::{RGBA, StableVec, StableVecHandle, r#async::FusedFuture, sync::Mutex};

use super::{
	ClipMask, ConcreteElement, Geometry, IdedElement, LayoutElement, RenderCurveElement, RenderElement, RenderImageElement,
	RenderPathElement, RenderTextElement,
	context::{Context, ElementContext, ElementKey, ElementSlot},
	element::{ElementHandle, Id},
	flow::{Location, Location3, Size},
	layout_elements,
	retained_tree::{ROOT_PATH, RetainedTree},
	snapshot::Snapshot,
	visual_transform::Affine2,
};
use crate::ui::{
	Container, Depth, Text, Transform, UiPoint, UiVector,
	components::{
		curve::{Curve, CurvePoint},
		image::Image,
		shape::Shape,
		text_field::TextField,
	},
	drag::{Drag, DragCapture, DragDrop},
	font::TextSystem,
	intersection::{HitCurve, MouseClickAcceleration},
	primitive::{Events, Key, Primitive as _, Primitives, Shapes, TextEdit},
	style::{Color, ConcreteStyle, EdgeFeather, Layer as _, LayerKind},
	transform::Rotation,
};
