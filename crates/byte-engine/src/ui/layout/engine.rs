//! UI retained tree evaluation, interaction state, and render snapshots.

/// The [`Engine`] struct owns UI evaluation state, text shaping, and pointer
/// interaction for one viewport.
///
/// Create an engine with [`Self::new`] or [`Self::with_context`], mount the root
/// component with [`Self::mount`], then call [`Self::evaluate`] and
/// [`Self::render`] for each frame.
/// See the [GUI guide](/docs/develop/gui)
/// for component, event, focus, and render-pass integration.
pub struct Engine<C = ()> {
	cursor_position: UiPoint,
	is_clicking: bool,
	/// Presses since the last evaluation, each delivered as one [`Events::Actuated`].
	presses: u32,
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
	/// The tree revisions last evaluated; `revision` advances only when snapshot geometry changes.
	tree: TreeRevisions,
	has_custom_flows: bool,
	revision: u64,
	/// The layout this one differs from by visual transforms inside the engine's dirty roots alone.
	transformed_from: Option<u64>,
	size: Size,
	elements: Vec<LayoutElement>,
	acceleration: MouseClickAcceleration,
}

/// Lends a host the retained layout.
///
/// It takes the field rather than the engine, so input routing can hit-test while it keeps changing the engine's
/// other state.
fn snapshot_of(retained: &RetainedLayout) -> Snapshot<'_> {
	Snapshot {
		#[cfg(test)]
		elements: &retained.elements,
		acceleration: &retained.acceleration,
		size: retained.size,
	}
}

/// The `RetainedRender` struct keeps the last render so unchanged trees report the same revision.
struct RetainedRender {
	/// The tree revisions this render was built from.
	tree: TreeRevisions,
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

// Isolate clipping, evaluation, future, and runtime mechanics from the engine facade.
mod clipping;
mod commands;
mod evaluation_context;
mod futures;
#[cfg(test)]
mod invalidation_tests;
pub(super) mod properties;
mod runtime;

pub(super) use clipping::first_layer_feather;
use clipping::*;
use commands::UiCommand;
use commands::apply;
pub use evaluation_context::*;
pub use futures::*;
use properties::Spares;
pub use properties::{ElementKind, Properties, Setup};
pub use runtime::*;

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
			cursor_position: UiPoint::zero(),
			is_clicking: false,
			presses: 0,
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
	/// [`Self::cancel`] does. The snapshot borrows the engine, so inspect or hit-test it, then drop it.
	/// Next, call [`Self::render`] and submit the returned render data through [`crate::ui::UiRenderPass`].
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
		self.route_focused_input();

		poll_ready_tasks(&mut self.core);
		let retained = self
			.retained_layout
			.as_ref()
			.expect("UI layout was not retained. Evaluation did not prepare its snapshot.");
		snapshot_of(retained)
	}

	// Visual transforms reuse placement for every flow. Other edits can replay custom
	// flows because their placement may depend on captured application state.
	fn build_layout(&mut self, size: Size, frame_allocator: &bumpalo::Bump) {
		let tree = &mut self.core.tree;
		let revisions = tree.revisions;
		let unchanged = self
			.retained_layout
			.as_ref()
			.is_some_and(|retained| retained.tree.any == revisions.any && retained.size == size);
		if !unchanged {
			let placement_unchanged = self.retained_layout.as_ref().is_some_and(|retained| {
				retained.tree.placement == revisions.placement
					&& (!retained.has_custom_flows || retained.tree.non_transform == revisions.non_transform)
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
					let covered = tree.lineage(index).skip(1).any(|ancestor| tree.transform_changed(ancestor));
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
			let has_custom_flows = self
				.retained_layout
				.as_ref()
				.filter(|retained| retained.tree.flow == revisions.flow)
				.map_or_else(
					|| {
						tree.elements.iter().any(|element| {
					matches!(&element.primitive, Primitives::Container(container) if crate::ui::flow::placement_key(&container.flow).is_none())
				})
					},
					|retained| retained.has_custom_flows,
				);
			let geometry_unchanged = self.retained_layout.as_ref().is_some_and(|retained| {
				retained.size == size
					&& retained.tree.clip == revisions.clip
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
						retained.tree.clip == revisions.clip
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
				// Structural edits also advance the clip revision (`TreeRevisions::clip`), including removal and remount of the same ID.
				if let Some(previous) = self
					.retained_layout
					.as_ref()
					.filter(|retained| retained.tree.clip == revisions.clip)
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
					self.core.runtime.update_geometry(elements);
				}
				let retained = self.retained_layout.get_or_insert_with(|| RetainedLayout {
					tree: revisions,
					has_custom_flows,
					revision: layout_revision,
					transformed_from,
					size,
					elements: Vec::new(),
					acceleration: MouseClickAcceleration::default(),
				});
				retained.revision = layout_revision;
				retained.transformed_from = transformed_from;
				retained.size = size;
				if !placement_unchanged || transforms_changed {
					retained.elements.clear();
					retained.elements.extend_from_slice(elements);
				}
				if let Some(hit) = hit {
					retained
						.acceleration
						.update(&hit.elements, &hit.sectors, &hit.curves, &hit.points);
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
			// Every class is current now, clipping included: an unchanged geometry already had the same clip revision.
			retained.tree = revisions;
			retained.has_custom_flows = has_custom_flows;
			if let Some(previous) = previous {
				retained.elements = previous;
			}
			tree.text_changes.clear();
			tree.clear_transform_changes();
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
		let key = (tree.revisions.appearance, layout_revision, size);
		if *visual_state_key == Some(key) {
			return;
		}
		let previous = transformed_from.map(|revision| (tree.revisions.appearance, revision, size));
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
		let snapshot = snapshot_of(retained);
		let hovered = snapshot.hit(position, held);
		// Every click and scroll this frame lands on the same position.
		let presses = std::mem::take(&mut self.presses);
		let pressed = presses > 0 || !self.scrolls.is_empty();
		let target = if pressed { snapshot.hit(position, None) } else { None };

		self.route_hover(hovered, held);
		if let Some(target) = target {
			for _ in 0..presses {
				self.core.runtime.push_event(bare_event(target, Events::Actuated));
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
	// Kept small so the unchanged-hover comparison stays inline in every idle evaluation.
	#[inline]
	fn route_hover(&mut self, hovered: Option<Id>, held: Option<Id>) {
		if held.is_some() {
			self.core.runtime.drag.set_over(hovered);
		}
		let previous = std::mem::replace(&mut self.hovered, hovered);
		if previous != hovered {
			self.route_hover_change(previous, hovered);
		}
	}

	/// Sends pointer exits and entries to the surfaces between the previous and the new hover target.
	#[cold]
	fn route_hover_change(&mut self, previous: Option<Id>, hovered: Option<Id>) {
		let tree = &self.core.tree;
		let [exited, entered] = &mut self.hover_chains;
		exited.clear();
		exited.extend(previous.into_iter().flat_map(|id| tree.ancestors(id)));
		entered.clear();
		entered.extend(hovered.into_iter().flat_map(|id| tree.ancestors(id)));
		let runtime = &mut self.core.runtime;
		// A surface containing both the old and the new target keeps the pointer and hears nothing.
		for &target in exited.iter().filter(|id| !entered.contains(id)) {
			runtime.push_event(bare_event(target, Events::PointerExited));
		}
		for &target in entered.iter().filter(|id| !exited.contains(id)) {
			runtime.push_event(bare_event(target, Events::PointerEntered));
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
		snapshot_of(self.retained_layout.as_ref()?).hit(position, None)
	}

	/// Delivers one event to a target and then to each of its ancestors.
	fn route_bubbling_event(&mut self, target: Id, kind: Events, delta: Option<UiVector>, source: Option<Id>) {
		let UiPoll { runtime, tree, .. } = &mut self.core;
		for target in tree.ancestors(target) {
			runtime.push_event(UiEvent {
				target,
				kind,
				delta,
				source,
			});
		}
	}

	/// Delivers the key presses and text edits queued since the last evaluation to the focused element.
	fn route_focused_input(&mut self) {
		// Most evaluations have no typed input, so skip resolving focus for them.
		if self.key_presses.is_empty() && self.text_edits.is_empty() {
			return;
		}
		let runtime = &mut self.core.runtime;
		let target = runtime.focused();
		route_focused(&mut self.key_presses, runtime, target, |target, key| UiKeyEvent {
			target,
			key,
		});
		route_focused(&mut self.text_edits, runtime, target, |target, edit| UiTextEditEvent {
			target,
			edit,
		});
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
		let layout = self
			.retained_layout
			.as_mut()
			.expect("UI render has no layout to draw. The most likely cause is calling render before the first evaluate.");
		let (layout_revision, size) = (layout.revision, layout.size);
		let tree_revision = self.core.tree.revision();
		let retained = self.retained_render.as_ref().is_some_and(|retained| {
			retained.tree.any == tree_revision && retained.layout_revision == layout_revision && retained.size == size
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
			retained.tree.clip == tree.revisions.clip && retained.layout_revision == layout_revision && retained.size == size
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
					&& render.tree.clip == tree.revisions.clip
					&& render.tree.appearance == tree.revisions.appearance
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
			.is_some_and(|retained| retained.tree.placement == tree.revisions.placement);
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
		// The live prefix length of each list, in the order of [`render_list`].
		let mut counts = [0usize; 5];
		let previous_footprints = std::mem::take(&mut self.rendered_footprints);
		let mut next_footprints = std::mem::take(&mut self.footprint_scratch);
		next_footprints.clear();
		next_footprints.resize(tree.elements.len(), None);
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
			// The footprint drawn last time records the revision it was built from, so an equal one means the
			// element's content is what its retained entry shows.
			let previous_footprint = previous_footprints.get(index).copied().flatten();
			let local_unchanged = previous_footprint.is_some_and(|footprint| {
				footprint.id == retained_element.serial && footprint.revision == retained_element.revision
			});
			if incremental && local_unchanged && self.dirty_stamps[index] != stamp {
				// Neither its geometry nor its content changed: the retained entry and footprint hold.
				next_footprints[index] = previous_footprint;
				counts[render_list(&retained_element.primitive)] += 1;
				continue;
			}
			let state = self.visual_state[index];
			let clip = state.clip.as_rect();
			let clip_mask = state.mask;
			let rotation = self
				.transforms
				.get(index)
				.map(|transform| transform.rotation)
				.filter(|rotation| !rotation.is_identity());
			let opacity = effective_opacity(index, &tree, &mut self.visual_state);
			let style = &retained_element.style;
			// Curves stroke outward from their path; rectangles stroke inward.
			let outset = match &retained_element.primitive {
				Primitives::Curve(_) => border_width(style.layers()) * state.scale[0].max(state.scale[1]) * 0.5,
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
			match previous_footprint {
				Some(previous) if previous == footprint => {}
				Some(previous) => {
					damage.extend(previous.rect);
					damage.extend(footprint.rect);
				}
				None => damage.extend(footprint.rect),
			}
			next_footprints[index] = Some(footprint);
			let placement = RenderPlacement {
				id: retained_element.serial,
				position: element.position,
				size: element.size,
				clip,
				clip_mask,
				rotation,
				opacity,
				scale: state.scale,
			};
			// Each arm writes its kind's next slot. Only layered geometry retains a style copy; images and text borrow
			// what they need.
			let count = &mut counts[render_list(&retained_element.primitive)];
			match &retained_element.primitive {
				Primitives::Container(container) => write_entry(elements, count, local_unchanged, placement, |previous| {
					let mut layers = previous
						.map(|entry| std::mem::take(&mut entry.style.layers))
						.unwrap_or_default();
					spares.fit_layers(&mut layers, style.layers.len());
					layers.clone_from(&style.layers);
					RenderElement {
						placement,
						style: ConcreteStyle { layers },
						corner_radius: container.corner_radius,
						corner_exponent: container.corner_exponent,
						sector: container.sector,
					}
				}),
				Primitives::Curve(curve) => write_entry(curve_elements, count, local_unchanged, placement, |previous| {
					let (mut layers, mut segments) = previous
						.map(|entry| (std::mem::take(&mut entry.style.layers), std::mem::take(&mut entry.segments)))
						.unwrap_or_else(|| (SmallVec::new(), spares.segments()));
					spares.fit_layers(&mut layers, style.layers.len());
					layers.clone_from(&style.layers);
					segments.clear();
					segments.extend_from_slice(curve.path().segments());
					RenderCurveElement {
						placement,
						style: ConcreteStyle { layers },
						segments,
					}
				}),
				Primitives::Path(path) => write_entry(path_elements, count, local_unchanged, placement, |previous| {
					// The outline is shared with every draw list entry that shows it, and only copied here when the
					// element changed.
					let segments = previous
						.as_ref()
						.filter(|entry| entry.path_id == path.id() && entry.version == path.version())
						.map(|entry| std::sync::Arc::clone(&entry.segments))
						.unwrap_or_else(|| std::sync::Arc::from(path.path().segments()));
					let mut layers = previous
						.map(|entry| std::mem::take(&mut entry.style.layers))
						.unwrap_or_default();
					spares.fit_layers(&mut layers, style.layers.len());
					layers.clone_from(&style.layers);
					RenderPathElement {
						placement,
						path_id: path.id(),
						version: path.version(),
						fill_rule: path.fill_rule,
						view_box: path.view_box,
						style: ConcreteStyle { layers },
						segments,
					}
				}),
				Primitives::Image(image) => {
					write_entry(image_elements, count, local_unchanged, placement, |_| RenderImageElement {
						placement,
						image_id: image.id(),
						version: image.version(),
						source_width: image.width_pixels(),
						source_height: image.height_pixels(),
						pixels: std::sync::Arc::clone(image.pixels()),
					})
				}
				Primitives::Text(text) => write_entry(text_elements, count, local_unchanged, placement, |previous| {
					let mut content = previous
						.map(|entry| std::mem::take(&mut entry.content))
						.unwrap_or_else(|| spares.string());
					content.clear();
					content.push_str(text.content());
					RenderTextElement {
						placement,
						color: match style.layers().first().map(|layer| &layer.color) {
							Some(Color::Value(rgba)) => *rgba,
							_ => RGBA::white(),
						},
						font_size: text.settings().font_size,
						content,
					}
				}),
			}
		}

		// Entries past the live prefix leave their buffers to the entries of later renders.
		let [element_count, curve_count, path_count, image_count, text_count] = counts;
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
			tree: tree.revisions,
			layout_revision,
			size,
			visible,
			render: Render { contents: shared },
		}
	}

	pub fn set_cursor_position(&mut self, v: UiPoint) {
		self.cursor_position = v;
	}

	pub fn update_click_state(&mut self, v: bool) {
		self.is_clicking = v;
		self.presses += u32::from(v);
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
		runtime.push_event(bare_event(source, Events::Grabbed));
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
			None => runtime.push_event(bare_event(held, Events::DragEnded)),
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
		self.presses = 0;
		self.scrolls.clear();
		self.drops.clear();
		self.key_states.clear();
		self.key_presses.clear();
		self.text_edits.clear();
		let runtime = &mut self.core.runtime;
		let source = runtime.drag.cancel()?;
		runtime.push_event(bare_event(source, Events::DragEnded));
		Some(source)
	}

	/// Returns the compact id render data carries for the element `id`.
	#[cfg(test)]
	pub(crate) fn render_id(&self, id: Id) -> u32 {
		self.core.tree.element(id).expect("the element is in the tree").serial
	}

	fn focused_text_field_last_char(&mut self) -> Option<char> {
		let target = self.core.runtime.focused()?;
		let element = self.core.tree.element(target)?;
		match &element.primitive {
			Primitives::Text(text) if text.editable => text.content().chars().last(),
			_ => None,
		}
	}
}

/// Returns which render list draws `primitive`: rectangles, curves, paths, images, or text, in that order.
fn render_list(primitive: &Primitives) -> usize {
	match primitive {
		Primitives::Container(_) => 0,
		Primitives::Curve(_) => 1,
		Primitives::Path(_) => 2,
		Primitives::Image(_) => 3,
		Primitives::Text(_) => 4,
	}
}

/// The `Placed` trait reaches the [`RenderPlacement`] every render list entry carries, so one writer serves all lists.
trait Placed {
	fn placement(&mut self) -> &mut RenderPlacement;
}

macro_rules! placed {
	($($entry:ty),*) => {$(
		impl Placed for $entry {
			fn placement(&mut self) -> &mut RenderPlacement {
				&mut self.placement
			}
		}
	)*};
}

placed!(
	RenderElement,
	RenderCurveElement,
	RenderPathElement,
	RenderImageElement,
	RenderTextElement
);

/// Writes the entry at slot `*count` of a render list and advances the count.
///
/// While the element is `unchanged` and the slot already shows it, only its placement is patched, so its content and
/// buffers stay. Otherwise `build` makes the entry from the slot's previous occupant, whose buffers it may take, and
/// the entry replaces that occupant or joins the end of the list.
fn write_entry<T: Placed>(
	list: &mut Vec<T>,
	count: &mut usize,
	unchanged: bool,
	placement: RenderPlacement,
	build: impl FnOnce(Option<&mut T>) -> T,
) {
	let slot = *count;
	*count += 1;
	if let Some(entry) = list.get_mut(slot)
		&& let head = entry.placement()
		&& unchanged
		&& head.id == placement.id
	{
		*head = placement;
		return;
	}
	let entry = build(list.get_mut(slot));
	match list.get_mut(slot) {
		Some(previous) => *previous = entry,
		None => list.push(entry),
	}
}

/// Drains `queue` into inputs `make` addresses to `target`, the focused element; without one they are dropped.
fn route_focused<T, E: runtime::Routed>(
	queue: &mut VecDeque<T>,
	runtime: &mut Runtime,
	target: Option<Id>,
	make: impl Fn(Id, T) -> E,
) {
	let Some(target) = target else {
		queue.clear();
		return;
	};
	for item in queue.drain(..) {
		runtime.push_event(make(target, item));
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
		self.elements.iter().find(|e| e.placement.id == 1).unwrap()
	}

	#[cfg(test)]
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

/// Builds an event addressed to `source` without a payload, such as a click or a drag notification.
fn bare_event(source: Id, kind: Events) -> UiEvent {
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
	use std::time::Duration;

	use super::*;
	use crate::ui::{
		Depth,
		components::curve::CurveSegment,
		flow::{self, Location3},
		layout::{
			Geometry, Sizing,
			context::{ContainerContext, Context, ElementContext},
		},
		primitive::TextEdit,
		style::{ConcreteLayer, ConcreteStyle, EdgeFeather},
	};

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
			let mut target = root
				.element("target")
				.container(|c| c.absolute_position(0, 0).size(50.into()))
				.await;
			let inner = target
				.element("inner")
				.container(|c| c.absolute_position(30, 30).size(20.into()))
				.await;
			let mut source = root
				.element("source")
				.container(|c| c.absolute_position(0, 0).size(20.into()))
				.await;
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
		assert_eq!(
			engine.ctx().dragged,
			vec![UiVector::new(30.0, 0.0), UiVector::new(30.0, 10.0)]
		);
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
		assert_eq!(first_ids.len(), 2);
		assert!(engine.core.tree.ancestors(first_ids[1]).eq([first_ids[1], first_ids[0]]));

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let second_ids = second.elements.iter().map(|element| element.id).collect::<Vec<_>>();

		assert_eq!(second_ids, first_ids);
		assert!(engine.core.tree.ancestors(first_ids[1]).eq([first_ids[1], first_ids[0]]));
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
		engine
			.evaluate(Size::new(100, 100), &allocator)
			.retain_hit_test(&mut first_hits);
		let first_render = engine.render().clone();
		let root = first_hits.query(UiPoint::zero()).unwrap();
		assert_eq!(first_hits.query(UiPoint::new(2.0, -2.0)), None);
		let mut second_hits = crate::ui::intersection::HitTest::default();
		engine
			.evaluate(Size::new(200, 200), &allocator)
			.retain_hit_test(&mut second_hits);
		assert_eq!(second_hits.query(UiPoint::new(0.5, -0.5)), Some(root));
		assert_eq!(engine.render().elements().next().unwrap().placement.size, Size::new(200, 200));
		// Geometry a host copied out and a render it cloned belong to the host, so a later frame leaves them as they were.
		assert_eq!(first_hits.query(UiPoint::zero()), Some(root));
		assert_eq!(first_hits.query(UiPoint::new(2.0, -2.0)), None);
		assert_eq!(first_render.elements().next().unwrap().placement.size, Size::new(100, 100));
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
		let replacement = second.elements[2].id;
		let tree = &engine.core.tree;
		let children = tree.children[tree.element_indices[&survivors[0]]]
			.iter()
			.map(|&child| tree.elements[child].id)
			.collect::<Vec<_>>();
		assert_eq!(children, [survivors[1], replacement]);
		assert!(tree.ancestors(survivors[1]).eq([survivors[1], survivors[0]]));
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
		let ids = render.elements().map(|element| element.placement.id).collect::<Vec<_>>();

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
			.find(|element| element.placement.id == 3)
			.expect("expected test value");

		assert_eq!(child.placement.position, Location3::new(35, 10, 2));
		assert_eq!(child.placement.size, Size::new(30, 30));
		assert_eq!(
			child.placement.clip,
			Some(Geometry::new(Location3::new(0, 0, 1), Size::new(50, 50)))
		);
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
			.find(|element| element.placement.id == 3)
			.expect("expected test value");

		assert_eq!(child.placement.position, Location3::new(70, 0, 2));
		assert_eq!(
			child.placement.clip,
			Some(Geometry::new(Location3::new(0, 0, 0), Size::new(100, 100)))
		);
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
			let snapshot = engine.evaluate(Size::new(100, 100), &allocator);
			snapshot.retain_hit_test(&mut hits);
			assert_eq!(snapshot.hit(outside, None), None);
			snapshot
				.hit(inside, None)
				.expect("the child's visible area should accept clicks")
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
			.find(|element| element.placement.position.x() == 70.0)
			.expect("expected test value");

		assert_eq!(toast.placement.size, Size::new(20, 20));
		assert_eq!(toast.placement.clip, None);
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

		assert_eq!(
			*engine.ctx(),
			Some(Geometry::new(Location3::new(24, 36, 1), Size::new(40, 20)))
		);
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
		let depths = render
			.elements()
			.map(|element| element.placement.position.z())
			.collect::<Vec<_>>();

		// Depth is the rank in the paint order, and each container is followed by its text.
		assert_eq!(depths, vec![0, 1, 3, 5]);
		let ids = render.elements().map(|element| element.placement.id).collect::<Vec<_>>();
		assert_eq!(
			render.texts().map(|element| element.content.as_str()).collect::<Vec<_>>(),
			["low", "tie", "high"]
		);

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		assert_eq!(render.elements().map(|element| element.placement.id).collect::<Vec<_>>(), ids);
		assert_eq!(
			render.texts().map(|element| element.content.as_str()).collect::<Vec<_>>(),
			["low", "tie", "high"]
		);

		engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render();
		assert_eq!(
			render.elements().map(|element| element.placement.id).collect::<Vec<_>>(),
			[ids[0], ids[3], ids[1], ids[2]]
		);
		assert_eq!(
			render.texts().map(|element| element.content.as_str()).collect::<Vec<_>>(),
			["high", "low", "tie"]
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
			.find(|element| element.placement.id == 1)
			.expect("expected test value");
		let child = render
			.elements()
			.find(|element| element.placement.id == 2)
			.expect("expected test value");
		let text = render
			.texts()
			.find(|text| text.placement.id == 3)
			.expect("expected test value");
		let expected = ClipMask {
			geometry: Geometry::new(Location3::new(0, 0, 0), Size::new(50, 40)),
			feather: EdgeFeather::vertical(8.0),
			corner_radius: 0.0,
			corner_exponent: 2.0,
		};

		assert_eq!(parent.placement.clip_mask, None);
		assert_eq!(child.placement.clip_mask, Some(expected));
		assert_eq!(text.placement.clip_mask, Some(expected));
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
			.find(|element| element.placement.id == 2)
			.expect("expected test value");

		assert_eq!(child.placement.clip_mask, None);
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
			.find(|element| element.placement.id == 2)
			.expect("expected test value");

		assert_eq!(
			child.placement.clip_mask,
			Some(ClipMask {
				geometry: Geometry::new(Location3::new(0, 0, 0), Size::new(50, 40)),
				feather: EdgeFeather::horizontal(4.0),
				corner_radius: 0.0,
				corner_exponent: 2.0,
			})
		);
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
			.find(|element| element.placement.id == 2)
			.expect("expected test value");
		let inside = Geometry::new(Location3::new(2, 2, 0), Size::new(46, 36));
		let mask = child.placement.clip_mask.expect("expected test value");

		assert_eq!(child.placement.clip, Some(inside));
		assert_eq!(mask.geometry.size, inside.size);
		assert_eq!(mask.feather, EdgeFeather::none());
		assert_eq!(mask.corner_radius, 6.0);
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
			.find(|element| element.placement.id == 2)
			.expect("expected test value");

		assert_eq!(child.placement.opacity, 0.125);
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
				.find(|element| element.placement.id == 2)
				.expect("expected test value")
				.placement
				.opacity,
			0.0
		);
		assert_eq!(
			render
				.elements()
				.find(|element| element.placement.id == 3)
				.expect("expected test value")
				.placement
				.opacity,
			1.0
		);
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
			.find(|element| element.placement.id == 2)
			.expect("expected test value");

		assert_eq!(child.placement.position, Location3::new(25, 35, 1));
		assert_eq!(child.placement.size, Size::new(10, 5));
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
		engine.evaluate(Size::new(100, 100), &allocator);
		let first = engine.render().revision();

		for step in [1, 2] {
			*engine.ctx_mut() = step;
			engine.evaluate(Size::new(100, 100), &allocator);
			assert_eq!(engine.render().revision(), first, "Edit {step} changed the render revision.");
		}
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
			field.update_text(|f| f.content(updated)).await;
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
		engine.evaluate(Size::new(100, 100), &allocator);
		let render = engine.render();
		let first = render.revision();
		let curve = render.curves().next().unwrap();
		assert!(matches!(curve.segments.as_slice(), [CurveSegment::Line { .. }]));
		let placement = engine.core.tree.revisions.placement;

		*engine.ctx_mut() = true;
		engine.evaluate(Size::new(100, 100), &allocator);
		let render = engine.render();
		assert_ne!(render.revision(), first);
		let curve = render.curves().next().unwrap();
		assert!(matches!(curve.segments.as_slice(), [CurveSegment::Cubic { .. }]));
		assert_eq!(
			engine.core.tree.revisions.placement, placement,
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
		assert_eq!(curves[0].placement.scale, [1.0, 1.0]);
		assert_eq!(curves[1].placement.scale, [2.0, 3.0]);
		assert_eq!(
			(curves[1].placement.position.x(), curves[1].placement.position.y()),
			(20.0, 30.0)
		);
		let text = render.texts().next().unwrap();
		assert_eq!(text.placement.scale, [2.0, 3.0]);
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
		let node = render
			.elements()
			.find(|element| element.placement.size == Size::new(80, 40))
			.unwrap();
		assert_eq!((node.placement.position.x(), node.placement.position.y()), (-50.0, -10.0));
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
	boxed::Box,
	collections::{HashMap, VecDeque},
	future::Future,
	marker::PhantomData,
	pin::Pin,
	sync::Arc,
	task::{Context as TaskContext, Poll, Wake, Waker},
};

use smallvec::SmallVec;
use utils::{RGBA, StableVec, StableVecHandle, r#async::FusedFuture, sync::Mutex};

use super::{
	ClipMask, Geometry, IdedElement, LayoutElement, RenderCurveElement, RenderElement, RenderImageElement, RenderPathElement,
	RenderPlacement, RenderTextElement,
	context::{Context, ElementContext, ElementKey, ElementSlot},
	element::Id,
	flow::{Location, Location3, Size},
	layout_elements,
	retained_tree::{ROOT_PATH, RetainedTree, TreeRevisions},
	snapshot::Snapshot,
	visual_transform::Affine2,
};
use crate::ui::{
	Container, Depth, Text, Transform, UiPoint, UiVector,
	components::{curve::Curve, image::Image},
	drag::{Drag, DragCapture, DragDrop},
	font::TextSystem,
	intersection::{HitCurve, MouseClickAcceleration},
	point::normalized_to_layout,
	primitive::{Events, Key, Primitives, TextEdit},
	style::{Color, ConcreteStyle, EdgeFeather, Layer as _, LayerKind},
	transform::Rotation,
};
