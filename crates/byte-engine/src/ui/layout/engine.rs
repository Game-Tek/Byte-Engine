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
	state: Rc<RefCell<EngineState>>,
	cursor_position: UiPoint,
	is_clicking: bool,
	clicks: Vec<bool>,
	scrolls: Vec<UiVector>,
	/// Released sources waiting for the next layout to resolve their drop targets.
	drops: Vec<DragDrop>,
	key_states: HashMap<Key, bool>,
	key_presses: VecDeque<Key>,
	text_edits: VecDeque<TextEdit>,
	text_system: TextSystem,
	ctx: Rc<C>,
	runtime: Rc<RefCell<Runtime>>,
	retained_layout: Option<RetainedLayout>,
	retained_render: Option<RetainedRender>,
	visual_state: Vec<VisualState>,
	visual_state_key: Option<(u64, u64, Size)>,
	measurements: Vec<super::Measurement>,
}

/// The `RetainedLayout` struct keeps the last computed layout so unchanged trees skip evaluation.
struct RetainedLayout {
	/// Last evaluated mutation; `revision` advances only when snapshot geometry changes.
	tree_revision: u64,
	placement_revision: u64,
	flow_revision: u64,
	has_custom_flows: bool,
	clip_revision: u64,
	revision: u64,
	size: Size,
	elements: Rc<Vec<LayoutElement>>,
	relations: Rc<Vec<(Id, Id)>>,
	acceleration: Rc<MouseClickAcceleration>,
}

/// Reuses uniquely owned snapshot storage without copying obsolete contents on a shared update.
fn retain_snapshot_data<T: Copy>(target: &mut Rc<Vec<T>>, source: &[T]) {
	if let Some(target) = Rc::get_mut(target) {
		target.clear();
		target.extend_from_slice(source);
	} else {
		*target = Rc::new(source.to_vec());
	}
}

/// The `RetainedRender` struct keeps the last render so unchanged trees report the same revision.
struct RetainedRender {
	tree_revision: u64,
	clip_revision: u64,
	visible: Vec<LayoutElement>,
	layout_revision: u64,
	size: Size,
	render: Render,
}

/// Layout distance a captured pointer must travel before a press becomes a drag.
const DRAG_THRESHOLD: f32 = 6.0;

static NEXT_RENDER_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl<C> Drop for Engine<C> {
	fn drop(&mut self) {
		// Detach tasks before dropping them because mounted-future cleanup may
		// re-enter the runtime to remove its retained scope.
		let tasks = {
			let mut runtime = self.runtime.borrow_mut();
			runtime.ready.lock().clear();
			runtime.frame_waiters.clear();
			runtime.event_waiters.clear();
			runtime.key_waiters.clear();
			runtime.text_edit_waiters.clear();
			std::mem::take(&mut runtime.tasks)
		};

		drop(tasks);
	}
}

pub(super) struct EngineState {
	element_ids: HashSet<Id>,
	cursor: Option<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerState {
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
mod evaluation_context;
mod futures;
#[cfg(test)]
mod invalidation_tests;
mod runtime;

use clipping::*;
pub use evaluation_context::*;
pub use futures::*;
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
	pub fn with_context(ctx: C) -> Self {
		Self {
			viewports: Vec::new(),
			state: Rc::new(RefCell::new(EngineState::new())),
			cursor_position: UiPoint::zero(),
			is_clicking: false,
			clicks: Vec::new(),
			scrolls: Vec::new(),
			drops: Vec::new(),
			key_states: HashMap::new(),
			key_presses: VecDeque::new(),
			text_edits: VecDeque::new(),
			text_system: TextSystem::new(),
			ctx: Rc::new(ctx),
			retained_layout: None,
			retained_render: None,
			visual_state: Vec::new(),
			visual_state_key: None,
			measurements: Vec::new(),
			runtime: Rc::new(RefCell::new(Runtime::new())),
		}
	}

	pub fn ctx(&self) -> &C {
		self.ctx.as_ref()
	}

	pub(crate) fn add_viewport(&mut self, viewport: VirtualViewport) {
		self.viewports.push(viewport);
	}

	/// Mounts the root asynchronous component into the retained UI tree.
	///
	/// Next, call [`Self::evaluate`] once per frame after updating pointer, key,
	/// and text input state.
	pub fn mount<F>(&mut self, root: F)
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> UiFuture<'ctx> + 'static,
	{
		let runtime = Rc::clone(&self.runtime);
		let tree = Rc::clone(&runtime.borrow().tree);
		let task_id = runtime.borrow_mut().reserve_task(ScopeId::ROOT);
		let ctx = EvaluationContext::new_root(Rc::clone(&self.ctx), Rc::clone(&runtime), tree, task_id);
		// Store an owning future while preserving the borrowed component interface.
		let future = Box::pin(async move {
			let mut ctx = ctx;
			root(&mut ctx).await;
		});
		runtime.borrow_mut().start_task(task_id, future);
	}

	/// Evaluates mounted UI tasks and returns a snapshot of the resulting layout.
	///
	/// A changed viewport size first ends the interaction in progress as
	/// [`Self::cancel`] does. Next, pass the mutable snapshot to [`Self::render`]
	/// and submit the returned render data through [`crate::ui::UiRenderPass`].
	pub fn evaluate<'a>(&mut self, size: Size, frame_allocator: &'a bumpalo::Bump) -> Snapshot<'a> {
		// Layout distances and the held source's place change with the viewport.
		if self.retained_layout.as_ref().is_some_and(|retained| retained.size != size) {
			self.cancel();
		}
		self.sync_pointer_state();
		self.route_drops();
		Runtime::begin_frame(Rc::clone(&self.runtime));
		Runtime::poll_ready_tasks(Rc::clone(&self.runtime));

		let mut snapshot = self.build_snapshot_from_ui_tree(size, frame_allocator);
		self.route_input_events(&mut snapshot);
		self.route_key_input_events();
		self.route_text_edit_events();

		Runtime::poll_ready_tasks(Rc::clone(&self.runtime));
		snapshot
	}

	// Paint changes reuse placement for built-in flows. Custom flows may read captured
	// state, so they still replay after any mutation, including changes to other nodes.
	fn build_snapshot_from_ui_tree<'a>(&mut self, size: Size, frame_allocator: &'a bumpalo::Bump) -> Snapshot<'a> {
		let tree = Rc::clone(&self.runtime.borrow().tree);
		let tree = tree.borrow();
		let revision = tree.revision();
		let unchanged = self
			.retained_layout
			.as_ref()
			.is_some_and(|retained| retained.tree_revision == revision && retained.size == size);
		if !unchanged {
			let placement_unchanged = self.retained_layout.as_ref().is_some_and(|retained| {
				retained.placement_revision == tree.placement_revision && !retained.has_custom_flows && retained.size == size
			});
			let previous = self
				.retained_layout
				.as_ref()
				.filter(|_| placement_unchanged)
				.map(|retained| Rc::clone(&retained.elements));
			let mut placed;
			let elements = if let Some(previous) = &previous {
				previous.as_slice()
			} else {
				placed = layout_elements(&tree, size, &mut self.text_system, &mut self.measurements, frame_allocator);
				apply_visual_transforms(&mut placed, &tree, frame_allocator);
				placed.as_slice()
			};
			let has_custom_flows = self.retained_layout.as_ref()
				.filter(|retained| retained.flow_revision == tree.flow_revision)
				.map_or_else(|| tree.elements.iter().any(|element| {
					matches!(&element.element.primitive, Primitives::Container(container) if crate::ui::flow::placement_key(&container.flow).is_none())
				}), |retained| retained.has_custom_flows);
			let geometry_unchanged = self.retained_layout.as_ref().is_some_and(|retained| {
				retained.size == size
					&& retained.clip_revision == tree.clip_revision
					&& (placement_unchanged || retained.elements.as_slice() == elements)
			});
			if !geometry_unchanged {
				// Resizing can replay a stateful flow without a tree mutation. Give each
				// changed geometry its own revision so older snapshots keep distinct cache keys.
				let layout_revision = self.retained_layout.as_ref().map_or(1, |retained| retained.revision + 1);
				self.prepare_appearance(elements, &tree, layout_revision, size);
				let hit_elements = clipped_hit_elements(elements, &tree, &self.visual_state, frame_allocator);
				// A stable topology keeps IDs and layout order, so update only changed bounds.
				// Structural edits also advance clip_revision, including removal and remount of the same ID.
				if let Some(previous) = self
					.retained_layout
					.as_ref()
					.filter(|retained| retained.clip_revision == tree.clip_revision)
				{
					let mut runtime = self.runtime.borrow_mut();
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
					self.state
						.borrow_mut()
						.set_element_ids(elements.iter().map(|element| element.id));
					self.runtime.borrow_mut().update_geometry(elements);
				}
				let retained = self.retained_layout.get_or_insert_with(|| RetainedLayout {
					tree_revision: revision,
					placement_revision: tree.placement_revision,
					flow_revision: tree.flow_revision,
					has_custom_flows,
					clip_revision: tree.clip_revision,
					revision: layout_revision,
					size,
					elements: Rc::default(),
					relations: Rc::default(),
					acceleration: Rc::default(),
				});
				retained.revision = layout_revision;
				retained.clip_revision = tree.clip_revision;
				retained.size = size;
				if !placement_unchanged {
					retain_snapshot_data(&mut retained.elements, elements);
				}
				retain_snapshot_data(&mut retained.relations, &tree.relations);
				// An older snapshot owns its index until it is dropped. Replace shared storage
				// instead of copying an obsolete grid; otherwise refill it in place.
				if Rc::get_mut(&mut retained.acceleration).is_none() {
					retained.acceleration = Rc::default();
				}
				Rc::get_mut(&mut retained.acceleration).unwrap().update(&hit_elements);
			}
			let retained = self
				.retained_layout
				.as_mut()
				.expect("UI layout was not retained. Evaluation did not prepare its snapshot.");
			retained.tree_revision = revision;
			retained.placement_revision = tree.placement_revision;
			retained.flow_revision = tree.flow_revision;
			retained.has_custom_flows = has_custom_flows;
		}
		let retained = self
			.retained_layout
			.as_ref()
			.expect("UI layout was not retained. Evaluation did not prepare its snapshot.");
		Snapshot {
			elements: Rc::clone(&retained.elements),
			relations: Rc::clone(&retained.relations),
			acceleration: Rc::clone(&retained.acceleration),
			frame_allocator: PhantomData,
			cursor: self.state.borrow().cursor(),
			engine_state: Rc::clone(&self.state),
			size,
			layout_revision: retained.revision,
		}
	}

	/// Reuses inherited appearance only for the same tree inputs and snapshot geometry.
	fn prepare_appearance(&mut self, elements: &[LayoutElement], tree: &RetainedTree, layout_revision: u64, size: Size) {
		let key = (tree.appearance_revision, layout_revision, size);
		if self.visual_state_key != Some(key) {
			prepare_visual_state(elements, tree, &mut self.visual_state);
			self.visual_state_key = Some(key);
		}
	}

	fn sync_pointer_state(&mut self) {
		self.runtime.borrow_mut().pointer = PointerState {
			position: self.cursor_position,
			pressed: self.is_clicking,
		};
	}

	fn route_input_events(&mut self, snapshot: &mut Snapshot<'_>) {
		while let Some(click) = self.clicks.pop() {
			if click && let Some(target) = snapshot.click(self.cursor_position) {
				self.runtime.borrow_mut().push_event(UiEvent {
					target,
					kind: Events::Actuated,
					delta: None,
					source: None,
				});
			}
		}

		while let Some(delta) = self.scrolls.pop() {
			if let Some(target) = snapshot.click(self.cursor_position) {
				self.route_bubbling_event(target, Events::Scrolled, Some(delta), None);
			}
		}
	}

	/// Delivers queued drops before components poll, so a component sees the drop
	/// and the cleared capture together.
	///
	/// Targets come from the previous frame's hit geometry, which the caller
	/// measured the release against. The target is any surface under the release
	/// point other than the released source, which hears about the end after it.
	fn route_drops(&mut self) {
		for drop in std::mem::take(&mut self.drops) {
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
			self.runtime
				.borrow_mut()
				.push_event(drag_event(drop.source, Events::DragEnded));
		}
	}

	/// Delivers one event to a target and then to each of its ancestors.
	fn route_bubbling_event(&mut self, target: Id, kind: Events, delta: Option<UiVector>, source: Option<Id>) {
		let runtime = Rc::clone(&self.runtime);
		let tree = Rc::clone(&runtime.borrow().tree);
		let tree = tree.borrow();
		let mut current = Some(target);

		while let Some(target) = current {
			runtime.borrow_mut().push_event(UiEvent {
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
				let state = self.state.borrow();
				self.runtime
					.borrow_mut()
					.focused_target(|target| state.contains_element(target))
			};

			if let Some(target) = target {
				self.runtime.borrow_mut().push_key_event(UiKeyEvent { target, key });
			}
		}
	}

	fn route_text_edit_events(&mut self) {
		while let Some(edit) = self.text_edits.pop_front() {
			let target = {
				let state = self.state.borrow();
				self.runtime
					.borrow_mut()
					.focused_target(|target| state.contains_element(target))
			};

			if let Some(target) = target {
				self.runtime
					.borrow_mut()
					.push_text_edit_event(UiTextEditEvent { target, edit });
			}
		}
	}

	/// Builds render data from an evaluated UI snapshot.
	///
	/// Next, give the returned data to [`crate::ui::UiRenderPass`] for GPU drawing.
	/// The render is retained by the engine: while the tree, its layout, and the
	/// viewport are unchanged, the same render with the same [`Render::revision`]
	/// is returned again. Clone it only when the revision changed.
	pub fn render(&mut self, snapshot: &mut Snapshot<'_>) -> &Render {
		let tree_revision = self.runtime.borrow().tree.borrow().revision();
		let retained = self.retained_render.as_ref().is_some_and(|retained| {
			retained.tree_revision == tree_revision
				&& retained.layout_revision == snapshot.layout_revision
				&& retained.size == snapshot.size
		});
		if !retained {
			self.retained_render = Some(self.build_render(snapshot));
		}
		&self
			.retained_render
			.as_ref()
			.expect("UI render must be retained after a rebuild. The most likely cause is a rebuild that returned early.")
			.render
	}

	// Rebuild visible draw data while preserving its layout order and owned render buffers.
	#[allow(clippy::too_many_lines)]
	fn build_render(&mut self, snapshot: &mut Snapshot<'_>) -> RetainedRender {
		let tree = Rc::clone(&self.runtime.borrow().tree);
		let tree = tree.borrow();
		let visibility_unchanged = self.retained_render.as_ref().is_some_and(|retained| {
			retained.clip_revision == tree.clip_revision
				&& retained.layout_revision == snapshot.layout_revision
				&& retained.size == snapshot.size
		});
		// Reuse the engine-owned buffers. Render clones keep their independent contents.
		let (mut elements, mut curve_elements, mut image_elements, mut text_elements, mut visible) = self
			.retained_render
			.take()
			.map(|retained| {
				(
					retained.render.elements,
					retained.render.curve_elements,
					retained.render.image_elements,
					retained.render.text_elements,
					retained.visible,
				)
			})
			.unwrap_or_default();
		// Rewrite the live prefix while reusing each entry's owned buffers. Entries left
		// beyond that prefix are dropped after the walk, so removed content cannot escape.
		let (mut element_count, mut curve_count, mut text_count) = (0, 0, 0);
		image_elements.clear();
		// Input callbacks can change appearance after layout. The cache key includes those changes.
		self.prepare_appearance(&snapshot.elements, &tree, snapshot.layout_revision, snapshot.size);
		if !visibility_unchanged {
			visible.clear();
			visible.extend(
				snapshot
					.elements
					.iter()
					.filter(|element| {
						tree.element_indices.get(&element.id).is_some_and(|&index| {
							self.visual_state[index]
								.clip
								.apply(geometry_from_layout_element(element))
								.is_some()
						})
					})
					.copied(),
			);
			// Stable depth order is shared by every primitive list. Keep it with visibility
			// so paint-only rebuilds neither sort nor allocate scratch for larger render entries.
			visible.sort_by_key(|element| element.position.z());
		}
		for element in &visible {
			let Some(&index) = tree.element_indices.get(&element.id) else {
				continue;
			};
			let retained_element = &tree.elements[index];
			let state = self.visual_state[index];
			let clip = state.clip.as_rect();
			let feather_mask = state.feather;
			let opacity = effective_opacity(index, &tree, &mut self.visual_state);
			let style = retained_element.element.primitive.style();
			// Only layered geometry retains a style copy; images and text borrow what they need.
			let mut push_rectangle = |corner_radius, corner_exponent| {
				let mut layers = elements
					.get_mut(element_count)
					.map(|entry| std::mem::take(&mut entry.style.layers))
					.unwrap_or_default();
				layers.clone_from(&style.layers);
				let rendered = RenderElement {
					id: element.id.get(),
					position: element.position,
					size: element.size,
					clip,
					feather_mask,
					style: ConcreteStyle { layers },
					opacity,
					backdrop_blur_radius: style
						.layers()
						.iter()
						.find(|layer| matches!(layer.kind(), LayerKind::Fill) && layer.backdrop_blur_radius() > 0.0)
						.map_or(0.0, |layer| layer.backdrop_blur_radius()),
					corner_radius,
					corner_exponent,
				};
				if element_count < elements.len() {
					elements[element_count] = rendered;
				} else {
					elements.push(rendered);
				}
				element_count += 1;
			};
			let mut push_text = |content: &str, font_size| {
				let mut retained_content = text_elements
					.get_mut(text_count)
					.map(|entry| std::mem::take(&mut entry.content))
					.unwrap_or_default();
				retained_content.clear();
				retained_content.push_str(content);
				let rendered = RenderTextElement {
					id: element.id.get(),
					position: element.position,
					size: element.size,
					clip,
					feather_mask,
					color: match style.layers().first().map(|layer| &layer.color) {
						Some(Color::Value(rgba)) => *rgba,
						_ => RGBA::white(),
					},
					opacity,
					font_size,
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
				Primitives::Container(container) => push_rectangle(container.corner_radius, container.corner_exponent),
				Primitives::Shape(shape) => {
					let (corner_radius, corner_exponent) = match shape.shape {
						Shapes::Box { radius, exponent, .. } => (radius, exponent),
						_ => (0.0, 2.0),
					};

					push_rectangle(corner_radius, corner_exponent);
				}
				Primitives::Curve(curve) => {
					let (mut layers, mut segments) = curve_elements
						.get_mut(curve_count)
						.map(|entry| (std::mem::take(&mut entry.style.layers), std::mem::take(&mut entry.segments)))
						.unwrap_or_default();
					layers.clone_from(&style.layers);
					segments.clear();
					segments.extend_from_slice(curve.path().segments());
					let rendered = RenderCurveElement {
						id: element.id.get(),
						position: element.position,
						size: element.size,
						clip,
						feather_mask,
						style: ConcreteStyle { layers },
						opacity,
						segments,
					};
					if curve_count < curve_elements.len() {
						curve_elements[curve_count] = rendered;
					} else {
						curve_elements.push(rendered);
					}
					curve_count += 1;
				}
				Primitives::Image(image) => image_elements.push(RenderImageElement {
					id: element.id.get(),
					image_id: image.id(),
					version: image.version(),
					source_width: image.width_pixels(),
					source_height: image.height_pixels(),
					pixels: std::sync::Arc::clone(image.pixels()),
					position: element.position,
					size: element.size,
					clip,
					feather_mask,
					opacity,
				}),
				Primitives::Text(text) => push_text(text.content(), text.settings().font_size),
				Primitives::TextField(text_field) => push_text(text_field.content(), text_field.settings().font_size),
			}
		}

		elements.truncate(element_count);
		curve_elements.truncate(curve_count);
		text_elements.truncate(text_count);

		RetainedRender {
			tree_revision: tree.revision(),
			clip_revision: tree.clip_revision,
			layout_revision: snapshot.layout_revision,
			size: snapshot.size,
			visible,
			render: Render {
				elements,
				curve_elements,
				image_elements,
				text_elements,
				revision: RenderRevision(NEXT_RENDER_REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
			},
		}
	}

	pub fn set_cursor_position(&mut self, v: UiPoint) {
		self.cursor_position = v;
	}

	pub fn cursor(&self) -> Option<Id> {
		self.state.borrow().cursor()
	}

	pub fn set_cursor(&mut self, cursor: Option<Id>) -> Option<Id> {
		self.state.borrow_mut().set_cursor(cursor)
	}

	pub fn clear_cursor(&mut self) {
		self.state.borrow_mut().set_cursor(None);
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

	/// Captures a hit-tested source for a pointer gesture at a layout position.
	///
	/// Returns `false` while another source is held. Next, forward pointer motion
	/// through [`Self::drag_to`] without hit testing the source again.
	pub fn press(&mut self, source: Id, position: UiPoint) -> bool {
		self.runtime.borrow_mut().drag.press(source, position)
	}

	/// Moves the captured pointer and reports whether a source is held.
	///
	/// The gesture activates once the pointer travels the drag threshold and stays
	/// active if it returns. Activation sends [`Events::DragStarted`] to the source.
	/// Next, call [`Self::release`] when the pointer is released.
	pub fn drag_to(&mut self, position: UiPoint) -> bool {
		let mut runtime = self.runtime.borrow_mut();
		let was_dragging = runtime.drag.capture().is_some_and(|capture| capture.dragging);
		let held = runtime.drag.move_to(position);
		if let Some(capture) = runtime.drag.capture().filter(|capture| capture.dragging && !was_dragging) {
			runtime.push_event(drag_event(capture.source, Events::DragStarted));
		}
		held
	}

	/// Releases the captured source and returns a drop only after activation.
	///
	/// The release position participates in threshold detection. A click clears
	/// capture without yielding a drop. A drop is also delivered as
	/// [`Events::Dropped`] to the surface under the release position by the next
	/// [`Self::evaluate`], so either the caller or a component can apply it.
	pub fn release(&mut self, position: UiPoint) -> Option<DragDrop> {
		let mut runtime = self.runtime.borrow_mut();
		let was_dragging = runtime.drag.capture().is_some_and(|capture| capture.dragging);
		let dropped = runtime.drag.release(position)?;
		// The release itself may have supplied the activating motion.
		if !was_dragging {
			runtime.push_event(drag_event(dropped.source, Events::DragStarted));
		}
		drop(runtime);
		self.drops.push(dropped);
		Some(dropped)
	}

	/// Returns the captured drag gesture, if a source is held.
	pub fn drag(&self) -> Option<DragCapture> {
		self.runtime.borrow().drag.capture()
	}

	/// Ends the interaction in progress and returns the source of a cancelled drag.
	///
	/// Queued clicks, scrolls, drops, key presses, and text edits are discarded,
	/// held keys and the pointer are released, and a held source is restored
	/// without a drop. A started drag sends [`Events::DragEnded`] to its
	/// source. Call this when the window loses focus or the user cancels; a
	/// changed viewport size calls it from [`Self::evaluate`].
	pub fn cancel(&mut self) -> Option<Id> {
		self.is_clicking = false;
		self.clicks.clear();
		self.scrolls.clear();
		self.drops.clear();
		self.key_states.clear();
		self.key_presses.clear();
		self.text_edits.clear();
		let mut runtime = self.runtime.borrow_mut();
		let was_dragging = runtime.drag.capture().is_some_and(|capture| capture.dragging);
		let source = runtime.drag.cancel()?;
		if was_dragging {
			runtime.push_event(drag_event(source, Events::DragEnded));
		}
		Some(source)
	}

	fn focused_text_field_last_char(&mut self) -> Option<char> {
		let target = {
			let state = self.state.borrow();
			self.runtime
				.borrow_mut()
				.focused_target(|target| state.contains_element(target))?
		};
		let runtime = self.runtime.borrow();
		let tree = runtime.tree.borrow();
		let element = tree.element(target)?;
		let Primitives::TextField(text_field) = &element.element.primitive else {
			return None;
		};
		text_field.content().chars().last()
	}
}

/// The `RenderRevision` struct identifies the content of one [`Render`].
///
/// Revisions are unique across engines. Two renders with equal revisions
/// describe identical visuals, so consumers keep the revision they last adopted
/// and skip work while it repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderRevision(u64);

/// The `Render` struct preserves the visual data derived from a snapshot so UI primitives can be submitted to the renderer.
#[derive(Clone)]
pub struct Render {
	elements: Vec<RenderElement>,
	curve_elements: Vec<RenderCurveElement>,
	image_elements: Vec<RenderImageElement>,
	text_elements: Vec<RenderTextElement>,
	revision: RenderRevision,
}

impl Render {
	/// Identifies this render's content; unchanged UI keeps the same revision across frames.
	pub fn revision(&self) -> RenderRevision {
		self.revision
	}

	pub(crate) fn root(&self) -> &RenderElement {
		self.elements.iter().find(|e| e.id == 1).unwrap()
	}

	pub(crate) fn size(&self) -> usize {
		self.elements.len() + self.curve_elements.len() + self.image_elements.len() + self.text_elements.len()
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
	pub delta: Option<UiVector>,
	/// The released drag source of an [`Events::Dropped`] event.
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
	use std::sync::{
		Arc, Mutex as StdMutex,
		atomic::{AtomicUsize, Ordering},
	};
	use std::time::Duration;

	use super::*;
	use crate::ui::{
		Depth, animate,
		components::{container::Container, curve::CurvePath, shape::Shape, text_field::TextField},
		flow::{self, Location3},
		layout::{
			Geometry, Sizing,
			context::{ContainerContext, Context, ElementContext},
		},
		primitive::TextEdit,
		spring,
		style::{ConcreteLayer, ConcreteStyle, EdgeFeather, Layer, LayerKind},
	};

	/// The `DropCounter` struct verifies that engine-owned UI context is released with the engine.
	struct DropCounter(Arc<AtomicUsize>);

	impl Drop for DropCounter {
		fn drop(&mut self) {
			self.0.fetch_add(1, Ordering::Relaxed);
		}
	}

	/// Mounts a component that records the drag it sees on every frame.
	fn drag_observer() -> (Engine, Arc<StdMutex<Vec<Option<DragCapture>>>>) {
		let observed = Arc::new(StdMutex::new(Vec::new()));
		let observed_for_task = Arc::clone(&observed);
		let mut engine = Engine::new();
		engine.mount(move |ctx| {
			let observed = Arc::clone(&observed_for_task);
			Box::pin(async move {
				loop {
					observed.lock().expect("expected test value").push(ctx.drag());
					ctx.render().await;
				}
			})
		});
		(engine, observed)
	}

	#[test]
	fn press_activates_after_the_threshold_and_release_yields_the_drop_once() {
		let frame_allocator = bumpalo::Bump::new();
		let (mut engine, observed) = drag_observer();
		let source = Id::new(7).expect("expected test value");
		assert!(engine.press(source, UiPoint::new(10.0, 20.0)));
		assert!(!engine.press(source, UiPoint::zero()));
		assert!(engine.drag_to(UiPoint::new(12.0, 22.0)));
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let held = observed.lock().expect("expected test value")[0].expect("expected test value");
		assert!(!held.dragging);
		assert_eq!(held.position, UiPoint::new(12.0, 22.0));
		// The release itself can supply the motion that reaches the threshold.
		let dropped = engine.release(UiPoint::new(30.0, 20.0)).expect("expected test value");
		assert_eq!(dropped.source, source);
		assert_eq!(dropped.position, UiPoint::new(30.0, 20.0));
		assert!(engine.drag().is_none());
		assert!(engine.release(UiPoint::new(30.0, 20.0)).is_none());
		assert!(!engine.drag_to(UiPoint::zero()));
	}

	#[test]
	fn click_restores_source_without_a_drop() {
		let mut engine = Engine::new();
		assert!(engine.press(Id::new(1).expect("expected test value"), UiPoint::new(10.0, 20.0)));
		assert!(engine.release(UiPoint::new(12.0, 22.0)).is_none());
		assert!(engine.drag().is_none());
	}

	#[test]
	fn cancel_restores_the_source_and_discards_queued_input() {
		let frame_allocator = bumpalo::Bump::new();
		let (mut engine, observed) = drag_observer();
		let source = Id::new(3).expect("expected test value");
		engine.press(source, UiPoint::zero());
		engine.drag_to(UiPoint::new(10.0, 0.0));
		engine.update_click_state(true);
		assert_eq!(engine.cancel(), Some(source));
		assert_eq!(engine.cancel(), None);
		assert!(engine.release(UiPoint::new(10.0, 0.0)).is_none());
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		assert_eq!(observed.lock().expect("expected test value")[0], None);
		assert!(!engine.runtime.borrow().pointer.pressed);
		assert!(engine.press(source, UiPoint::zero()));
	}

	#[test]
	fn resized_viewport_cancels_the_held_source_before_evaluation() {
		let frame_allocator = bumpalo::Bump::new();
		let (mut engine, observed) = drag_observer();
		let source = Id::new(3).expect("expected test value");
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.press(source, UiPoint::zero());
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let _ = engine.evaluate(Size::new(50, 100), &frame_allocator);
		let observed = observed.lock().expect("expected test value");
		assert!(observed[1].is_some());
		assert_eq!(observed[2], None);
	}

	/// What the board's components observed, in the order the source and target saw it.
	#[derive(Default)]
	struct DragLog {
		ids: Option<(Id, Id)>,
		started: usize,
		ended: usize,
		drops: Vec<Option<Id>>,
		/// The number of drops recorded when each end arrived.
		drops_before_end: Vec<usize>,
	}

	/// Mounts a target with a hit-testable child and a source drawn over the target's corner.
	fn drag_board() -> (Engine, Arc<StdMutex<DragLog>>) {
		let log = Arc::new(StdMutex::new(DragLog::default()));
		let log_for_task = Arc::clone(&log);
		let mut engine = Engine::new();
		engine.mount(move |ctx| {
			let log = Arc::clone(&log_for_task);
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default().hit_testable(false));
				let mut target = root.element("target").container(
					Container::default()
						.absolute_position(0, 0)
						.width(50.into())
						.height(50.into()),
				);
				let _inner = target.element("inner").container(
					Container::default()
						.absolute_position(30, 30)
						.width(20.into())
						.height(20.into()),
				);
				let mut source = root.element("source").container(
					Container::default()
						.absolute_position(0, 0)
						.width(20.into())
						.height(20.into()),
				);
				log.lock().expect("expected test value").ids = Some((target.id(), source.id()));
				loop {
					// A biased select takes the queued drop before the end, as a client would.
					utils::r#async::select_biased! {
						event = target.on(Events::Dropped) => log.lock().expect("expected test value").drops.push(event.source),
						_ = source.on(Events::DragStarted) => log.lock().expect("expected test value").started += 1,
						_ = source.on(Events::DragEnded) => {
							let mut log = log.lock().expect("expected test value");
							log.ended += 1;
							let drops = log.drops.len();
							log.drops_before_end.push(drops);
						},
					}
				}
			})
		});
		(engine, log)
	}

	#[test]
	fn drag_events_reach_the_source_and_the_surface_under_the_release() {
		let allocator = bumpalo::Bump::new();
		let (mut engine, log) = drag_board();
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		let (_, source) = log.lock().expect("expected test value").ids.expect("expected test value");

		// Motion past the threshold starts the drag; the drop lands on the target's child and bubbles up.
		assert!(engine.press(source, UiPoint::new(10.0, 10.0)));
		assert!(engine.drag_to(UiPoint::new(40.0, 40.0)));
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		assert_eq!(log.lock().expect("expected test value").started, 1);
		assert!(engine.release(UiPoint::new(40.0, 40.0)).is_some());
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		assert_eq!(log.lock().expect("expected test value").drops, vec![Some(source)]);

		// A release whose motion activates the drag still starts it, and a release over the
		// source itself drops onto the surface beneath it.
		assert!(engine.press(source, UiPoint::new(1.0, 1.0)));
		assert!(engine.release(UiPoint::new(19.0, 19.0)).is_some());
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		let observed = log.lock().expect("expected test value");
		assert_eq!(observed.started, 2);
		assert_eq!(observed.drops, vec![Some(source), Some(source)]);
		drop(observed);

		// A release outside every surface drops nowhere.
		assert!(engine.press(source, UiPoint::new(10.0, 10.0)));
		assert!(engine.release(UiPoint::new(90.0, 90.0)).is_some());
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		let observed = log.lock().expect("expected test value");
		assert_eq!(observed.drops.len(), 2);
		// Every started gesture ends once, and the target's drop is recorded before the source's end.
		assert_eq!(observed.ended, 3);
		assert_eq!(observed.drops_before_end, vec![1, 2, 2]);
	}

	#[test]
	fn cancel_ends_a_started_source_only() {
		let allocator = bumpalo::Bump::new();
		let (mut engine, log) = drag_board();
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		let (_, source) = log.lock().expect("expected test value").ids.expect("expected test value");

		assert!(engine.press(source, UiPoint::new(10.0, 10.0)));
		assert_eq!(engine.cancel(), Some(source));
		assert!(engine.press(source, UiPoint::new(10.0, 10.0)));
		assert!(engine.drag_to(UiPoint::new(40.0, 40.0)));
		assert_eq!(engine.cancel(), Some(source));
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		let observed = log.lock().expect("expected test value");
		assert_eq!(observed.started, 1);
		assert_eq!(observed.ended, 1);
		assert!(observed.drops.is_empty());
	}

	#[test]
	fn reparent_appends_to_the_new_parent_and_refuses_cycles() {
		let allocator = bumpalo::Bump::new();
		let log = Arc::new(StdMutex::new((None, Vec::new())));
		let log_for_task = Arc::clone(&log);
		let mut engine = Engine::new();
		engine.mount(move |ctx| {
			let log = Arc::clone(&log_for_task);
			Box::pin(async move {
				let mut root = ctx
					.element("root")
					.container(Container::default().flow(flow::column_with_gap(0)));
				let mut first = root
					.element("first")
					.container(Container::default().width(10.into()).height(10.into()));
				let mut second = root.element("second").container(
					Container::default()
						.width(10.into())
						.height(10.into())
						.flow(flow::column_with_gap(0)),
				);
				second
					.element("inner")
					.container(Container::default().width(5.into()).height(5.into()));
				log.lock().expect("expected test value").0 = Some((first.id(), second.id()));
				ctx.render().await;
				let moved = first.reparent(second.id());
				let cycle = second.reparent(first.id());
				log.lock().expect("expected test value").1 = vec![moved, cycle];
				loop {
					ctx.render().await;
				}
			})
		});
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		let (first, second) = log.lock().expect("expected test value").0.expect("expected test value");
		assert_eq!(engine.runtime.borrow().geometry[&first].y(), 0.0);
		assert_eq!(engine.runtime.borrow().geometry[&second].y(), 10.0);
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		let _ = engine.evaluate(Size::new(100, 100), &allocator);
		assert_eq!(log.lock().expect("expected test value").1, vec![true, false]);
		let runtime = engine.runtime.borrow();
		assert_eq!(runtime.geometry[&second].y(), 0.0);
		assert_eq!(runtime.geometry[&first].y(), 5.0);
	}

	#[test]
	fn unchanged_tree_keeps_layout_and_render_revision_across_frames() {
		let mut engine = Engine::new();
		engine.mount(|ctx| {
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default());
				root.element("label").text(Text::new("Stable"));
				loop {
					ctx.render().await;
				}
			})
		});
		let frame_allocator = bumpalo::Bump::new();

		let mut first = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let first_render = engine.render(&mut first);
		let (first_revision, first_size) = (first_render.revision(), first_render.size());
		let mut second = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let second_render = engine.render(&mut second);

		assert_eq!(first.layout_revision, second.layout_revision);
		assert_eq!(first_revision, second_render.revision());
		assert_eq!(first_size, second_render.size());
		assert!(engine.retained_layout.is_some());
	}

	#[test]
	fn property_mutation_and_resize_advance_the_render_revision() {
		let mut engine = Engine::new();
		let opacity = Rc::new(std::cell::Cell::new(1.0f32));
		let shared = Rc::clone(&opacity);
		engine.mount(move |ctx| {
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default());
				loop {
					let opacity = shared.get();
					root.update_container(|container| container.set_opacity(opacity));
					ctx.render().await;
				}
			})
		});
		let frame_allocator = bumpalo::Bump::new();
		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let baseline = engine.render(&mut snapshot).revision();

		// The mounted task mutates the container every frame, so revisions must move.
		opacity.set(0.5);
		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let mutated = engine.render(&mut snapshot);
		assert_ne!(baseline, mutated.revision());
		assert_eq!(mutated.elements().next().unwrap().opacity, 0.5);
		let mutated = mutated.revision();

		let mut snapshot = engine.evaluate(Size::new(200, 100), &frame_allocator);
		let resized = engine.render(&mut snapshot);
		assert_ne!(mutated, resized.revision());
		assert_eq!(resized.root().size, Size::new(200, 100));
	}

	#[test]
	fn retained_tree_revision_tracks_insertion_mutation_and_removal() {
		let mut tree = RetainedTree::new();
		let start = tree.revision();
		let (id, _) = tree.add_element(None, 0, "root", ConcreteElement::container(Container::default()));
		assert!(tree.revision() > start);

		let after_insert = tree.revision();
		// Re-declaring the same path on a later frame is idempotent and must not invalidate retained state.
		tree.begin_frame();
		tree.add_element(None, 0, "root", ConcreteElement::container(Container::default()));
		assert_eq!(tree.revision(), after_insert);

		assert!(tree.update_element(id, |_| true));
		assert!(tree.revision() > after_insert);

		let after_mutation = tree.revision();
		let (_, child_path) = tree.add_element(Some(id), 0, "child", ConcreteElement::container(Container::default()));
		let after_child = tree.revision();
		assert!(after_child > after_mutation);
		assert!(!tree.remove_scope(child_path).is_empty());
		assert!(tree.revision() > after_child);
		assert!(tree.remove_scope(child_path).is_empty());
	}

	/// Mounts a scope that spawns a component counting the frames it runs, until `open` clears.
	fn counting_scope(
		ticks: Rc<std::cell::Cell<u32>>,
		open: Rc<std::cell::Cell<bool>>,
	) -> impl for<'ctx> FnOnce(&'ctx mut EvaluationContext) -> MountedUiFuture<'ctx, ()> + 'static {
		move |ctx| {
			Box::pin(async move {
				ctx.element("body").container(Container::default());
				ctx.element("ticker").component(move |ctx| {
					Box::pin(async move {
						loop {
							ticks.set(ticks.get() + 1);
							ctx.render().await;
						}
					})
				});
				while open.get() {
					ctx.render().await;
				}
			})
		}
	}

	#[test]
	fn removing_a_mounted_scope_ends_the_components_spawned_inside_it() {
		let allocator = bumpalo::Bump::new();
		let ticks = Rc::new(std::cell::Cell::new(0));
		let open = Rc::new(std::cell::Cell::new(true));
		let (task_ticks, task_open) = (Rc::clone(&ticks), Rc::clone(&open));
		let mut engine = Engine::new();
		engine.mount(move |ctx| {
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default());
				loop {
					if task_open.get() {
						root.element("menu")
							.mount(counting_scope(Rc::clone(&task_ticks), Rc::clone(&task_open)))
							.await;
					} else {
						ctx.render().await;
					}
				}
			})
		});
		let mut frames = |count: usize| {
			for _ in 0..count {
				let _ = engine.evaluate(Size::new(100, 100), &allocator);
			}
		};

		frames(3);
		assert!(ticks.get() > 0);
		open.set(false);
		frames(2);
		let closed = ticks.get();
		frames(3);
		assert_eq!(ticks.get(), closed, "A component kept running after its scope was removed.");

		// Reopening spawns a fresh component that runs again.
		open.set(true);
		frames(3);
		assert!(ticks.get() > closed);
	}

	#[test]
	fn removing_a_mounted_scope_keeps_tasks_of_a_live_scope_with_the_same_path() {
		let allocator = bumpalo::Bump::new();
		let first_ticks = Rc::new(std::cell::Cell::new(0));
		let second_ticks = Rc::new(std::cell::Cell::new(0));
		let first_open = Rc::new(std::cell::Cell::new(true));
		let (first, second, open) = (Rc::clone(&first_ticks), Rc::clone(&second_ticks), Rc::clone(&first_open));
		let mut engine = Engine::new();
		engine.mount(move |ctx| {
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default());
				let mut first = root.element("toast").mount(counting_scope(first, open));
				utils::r#async::select_biased! {
					_ = first => {},
					_ = ctx.render() => {},
				}
				// Declared on a later frame under the same parent and name, so its path repeats the first's.
				let mut second = root
					.element("toast")
					.mount(counting_scope(second, Rc::new(std::cell::Cell::new(true))));
				loop {
					utils::r#async::select_biased! {
						_ = first => {},
						_ = second => {},
						_ = ctx.render() => {},
					}
				}
			})
		});
		let mut frames = |count: usize| {
			for _ in 0..count {
				let _ = engine.evaluate(Size::new(100, 100), &allocator);
			}
		};

		frames(3);
		assert!(first_ticks.get() > 0 && second_ticks.get() > 0);
		first_open.set(false);
		frames(2);
		let (first_closed, second_closed) = (first_ticks.get(), second_ticks.get());
		frames(3);
		assert_eq!(first_ticks.get(), first_closed);
		assert!(
			second_ticks.get() > second_closed,
			"Removing one scope ended a live scope's component."
		);
	}

	#[test]
	fn dropping_engine_releases_mounted_context() {
		let drops = Arc::new(AtomicUsize::new(0));
		let mut engine = Engine::with_context(DropCounter(Arc::clone(&drops)));

		engine.mount(|_ctx| Box::pin(async {}));
		drop(engine);

		assert_eq!(drops.load(Ordering::Relaxed), 1);
	}

	#[test]
	fn mounted_task_retains_markup_without_render_loop() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("root").container(Container::default().flow(flow::column));
			})
		});

		let mut first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(engine.render(&mut first).size(), 1);

		let mut second = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(engine.render(&mut second).size(), 1);
	}

	#[test]
	fn retained_button_receives_later_click_event() {
		let frame_allocator = bumpalo::Bump::new();
		let hits = Arc::new(AtomicUsize::new(0));
		let hits_for_task = Arc::clone(&hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let hits = Arc::clone(&hits_for_task);
			Box::pin(async move {
				let mut button = ctx.element("button").container(Container::default());
				loop {
					button.on(Events::Actuated).await;
					hits.fetch_add(1, Ordering::SeqCst);
				}
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::zero());
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn context_pointer_reflects_engine_pointer_state() {
		let frame_allocator = bumpalo::Bump::new();
		let observed = Arc::new(StdMutex::new(None));
		let observed_for_task = Arc::clone(&observed);
		let mut engine = Engine::new();

		engine.set_cursor_position(UiPoint::new(0.25, -0.5));
		engine.update_click_state(true);
		engine.mount(move |ctx| {
			let observed = Arc::clone(&observed_for_task);
			Box::pin(async move {
				*observed.lock().expect("expected test value") = Some(ctx.pointer());
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(
			*observed.lock().expect("expected test value"),
			Some(PointerState {
				position: UiPoint::new(0.25, -0.5),
				pressed: true,
			})
		);
	}

	#[test]
	fn context_pointer_updates_across_render_await_frames() {
		let frame_allocator = bumpalo::Bump::new();
		let observed = Arc::new(StdMutex::new(Vec::new()));
		let observed_for_task = Arc::clone(&observed);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let observed = Arc::clone(&observed_for_task);
			Box::pin(async move {
				observed.lock().expect("expected test value").push(ctx.pointer());
				ctx.render().await;
				observed.lock().expect("expected test value").push(ctx.pointer());
			})
		});

		engine.set_cursor_position(UiPoint::new(-1.0, -1.0));
		engine.update_click_state(false);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.75, 0.5));
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(
			*observed.lock().expect("expected test value"),
			vec![
				PointerState {
					position: UiPoint::new(-1.0, -1.0),
					pressed: false,
				},
				PointerState {
					position: UiPoint::new(0.75, 0.5),
					pressed: true,
				},
			]
		);
	}

	#[test]
	fn scroll_event_bubbles_from_hovered_child_to_parent() {
		let frame_allocator = bumpalo::Bump::new();
		let received = Arc::new(StdMutex::new(None));
		let received_for_task = Arc::clone(&received);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let received = Arc::clone(&received_for_task);
			Box::pin(async move {
				let mut parent = ctx.element("parent").container(Container::default());
				parent.element("child").container(Container::default());
				let event = parent.on(Events::Scrolled).await;
				*received.lock().expect("expected test value") = event.delta;
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::zero());
		engine.update_scroll_state(UiVector::new(0.0, -1.0));
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*received.lock().expect("expected test value"), Some(UiVector::new(0.0, -1.0)));
	}

	#[test]
	fn nested_retained_components_attach_under_declaring_element_with_stable_ids() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default().flow(flow::column));
				frame.element("child").component(|ctx| {
					Box::pin(async move {
						ctx.element("button").container(Container::default().size(20.into()));
					})
				});
			})
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let first_ids = first.elements.iter().map(|element| element.id).collect::<Vec<_>>();

		assert_eq!(first.elements.len(), 2);
		assert_eq!(first.relations.as_slice(), &[(first_ids[0], first_ids[1])]);

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let second_ids = second.elements.iter().map(|element| element.id).collect::<Vec<_>>();

		assert_eq!(second_ids, first_ids);
		assert_eq!(second.relations, first.relations);
	}

	#[test]
	fn repeated_sibling_names_keep_stable_ids_across_frames() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default().flow(flow::column));
				for _ in 0..64 {
					frame.element("item").container(Container::default().size(1.into()));
				}
			})
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let first_ids = first.elements.iter().map(|element| element.id).collect::<Vec<_>>();

		assert_eq!(first_ids.len(), 65);

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let second_ids = second.elements.iter().map(|element| element.id).collect::<Vec<_>>();

		assert_eq!(second_ids, first_ids);
	}

	#[test]
	fn mounted_scope_cleanup_removes_structural_path_descendants() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame
					.element("modal")
					.mount(|ctx| {
						Box::pin(async move {
							ctx.element("body").container(Container::default().size(10.into()));
							ctx.render().await;
						})
					})
					.await;
			})
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements.len(), 2);

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(second.elements.len(), 1);
	}

	#[test]
	fn earlier_snapshots_and_render_clones_keep_their_geometry_after_resize() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("root")
					.container(Container::default().size(Sizing::Relative(1, 1)));
			})
		});
		let mut first = engine.evaluate(Size::new(100, 100), &allocator);
		let first_render = engine.render(&mut first).clone();
		let mut hits = crate::ui::intersection::HitTest::default();
		first.retain_hit_test(&mut hits);
		let root = hits.query(UiPoint::zero()).unwrap();
		assert_eq!(hits.query(UiPoint::new(2.0, -2.0)), None);
		let mut second = engine.evaluate(Size::new(200, 200), &allocator);
		second.retain_hit_test(&mut hits);
		assert_eq!(hits.query(UiPoint::new(0.5, -0.5)), Some(root));
		assert_eq!(
			engine.render(&mut second).elements().next().unwrap().size,
			Size::new(200, 200)
		);
		first.retain_hit_test(&mut hits);
		assert_eq!(hits.query(UiPoint::zero()), Some(root));
		assert_eq!(hits.query(UiPoint::new(2.0, -2.0)), None);
		assert_eq!(first_render.elements().next().unwrap().size, Size::new(100, 100));
	}

	#[test]
	fn a_task_waking_during_its_poll_completes_in_the_same_evaluation() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(|ctx| {
			Box::pin(async move {
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
				ctx.element("ready").container(Container::default().size(20.into()));
			})
		});
		let snapshot = engine.evaluate(Size::new(100, 100), &allocator);
		assert_eq!(snapshot.elements.len(), 1);
		assert_eq!(snapshot.elements[0].size, Size::new(20, 20));
	}

	#[test]
	fn mounted_scope_cleanup_follows_ownership_after_reparenting() {
		let allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		engine.mount(|ctx| {
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default());
				let destination = root.element("destination").container(Container::default()).id();
				root.element("scope")
					.mount(move |ctx| {
						Box::pin(async move {
							let mut child = ctx.element("child").container(Container::default());
							assert!(child.reparent(destination));
							child.element("grandchild").container(Container::default());
							ctx.render().await;
						})
					})
					.await;
				root.element("replacement").container(Container::default().size(15.into()));
			})
		});
		let first = engine.evaluate(Size::new(100, 100), &allocator);
		assert_eq!(first.elements.len(), 4);
		let survivors = [first.elements[0].id, first.elements[1].id];
		let second = engine.evaluate(Size::new(100, 100), &allocator);
		assert_eq!(second.elements.len(), 3);
		assert_eq!([second.elements[0].id, second.elements[1].id], survivors);
		assert_eq!(second.elements[2].size, Size::new(15, 15));
		assert_eq!(
			second.relations.as_slice(),
			&[(survivors[0], survivors[1]), (survivors[0], second.elements[2].id)]
		);
	}

	#[test]
	fn context_wait_wakes_from_runtime_frame_loop() {
		let frame_allocator = bumpalo::Bump::new();
		let hits = Arc::new(AtomicUsize::new(0));
		let hits_for_task = Arc::clone(&hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let hits = Arc::clone(&hits_for_task);
			Box::pin(async move {
				ctx.wait(Duration::from_millis(1)).await;
				hits.fetch_add(1, Ordering::SeqCst);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 0);

		std::thread::sleep(Duration::from_millis(2));
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn empty_retained_tree_does_not_panic() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert!(snapshot.elements.is_empty());
		assert_eq!(engine.render(&mut snapshot).size(), 0);
	}

	#[test]
	fn default_container_clip_skips_fully_clipped_descendants_in_render() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default());
				let mut parent = root
					.element("parent")
					.container(Container::default().width(50.into()).height(50.into()));
				parent.element("child").container(
					Container::default()
						.width(20.into())
						.height(20.into())
						.absolute_position(70, 0),
				);
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let ids = render.elements().map(|element| element.id).collect::<Vec<_>>();

		assert_eq!(ids.len(), 2);
		assert!(ids.contains(&1));
		assert!(ids.contains(&2));
	}

	#[test]
	fn default_container_clip_is_carried_to_partially_clipped_descendants() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default());
				let mut parent = root
					.element("parent")
					.container(Container::default().width(50.into()).height(50.into()));
				parent.element("child").container(
					Container::default()
						.width(30.into())
						.height(30.into())
						.absolute_position(35, 10),
				);
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
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

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default());
				let mut parent = root
					.element("parent")
					.container(Container::default().width(50.into()).height(50.into()).clip(false));
				parent.element("child").container(
					Container::default()
						.width(20.into())
						.height(20.into())
						.absolute_position(70, 0),
				);
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
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
		engine.mount(|ctx| {
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default().hit_testable(false));
				let mut parent = root.element("parent").container(
					Container::default()
						.absolute_position(10, 10)
						.width(40.into())
						.height(40.into()),
				);
				let _child = parent.element("child").container(
					Container::default()
						.absolute_position(25, 25)
						.width(40.into())
						.height(40.into()),
				);
				let _decoration = root.element("decoration").container(
					Container::default()
						.absolute_position(0, 0)
						.width(100.into())
						.height(100.into())
						.hit_testable(false),
				);
				loop {
					ctx.render().await;
				}
			})
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
		let hits = Arc::new(AtomicUsize::new(0));
		let hits_for_task = Arc::clone(&hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let hits = Arc::clone(&hits_for_task);
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default());
				let mut parent = root
					.element("parent")
					.container(Container::default().width(50.into()).height(50.into()));
				let mut child = parent.element("child").container(
					Container::default()
						.width(20.into())
						.height(20.into())
						.absolute_position(70, 0),
				);

				loop {
					child.on(Events::Actuated).await;
					hits.fetch_add(1, Ordering::SeqCst);
				}
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.5, 0.8));
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 0);
	}

	#[test]
	fn clip_false_preserves_descendant_hit_testing_overflow() {
		let frame_allocator = bumpalo::Bump::new();
		let hits = Arc::new(AtomicUsize::new(0));
		let hits_for_task = Arc::clone(&hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let hits = Arc::clone(&hits_for_task);
			Box::pin(async move {
				let mut root = ctx.element("root").container(Container::default());
				let mut parent = root
					.element("parent")
					.container(Container::default().width(50.into()).height(50.into()).clip(false));
				let mut child = parent.element("child").container(
					Container::default()
						.width(20.into())
						.height(20.into())
						.absolute_position(70, 0),
				);

				loop {
					child.on(Events::Actuated).await;
					hits.fetch_add(1, Ordering::SeqCst);
				}
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.5, 0.8));
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn clip_false_preserves_absolute_descendant_render_overflow() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut root = ctx
					.element("root")
					.container(Container::default().width(50.into()).height(50.into()).clip(false));
				root.element("toast").container(
					Container::default()
						.width(20.into())
						.height(20.into())
						.depth(Depth::absolute(1))
						.absolute_position(70, 0),
				);
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

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

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut root = ctx
					.element("root")
					.container(Container::default().width(50.into()).height(50.into()));
				root.element("toast").container(
					Container::default()
						.width(20.into())
						.height(20.into())
						.depth(Depth::absolute(1))
						.absolute_position(70, 0),
				);
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

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
		let hits = Arc::new(AtomicUsize::new(0));
		let hits_for_task = Arc::clone(&hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let hits = Arc::clone(&hits_for_task);
			Box::pin(async move {
				let mut root = ctx
					.element("root")
					.container(Container::default().width(50.into()).height(50.into()));
				let mut toast = root.element("toast").container(
					Container::default()
						.width(20.into())
						.height(20.into())
						.depth(Depth::absolute(1))
						.absolute_position(70, 0),
				);

				loop {
					toast.on(Events::Actuated).await;
					hits.fetch_add(1, Ordering::SeqCst);
				}
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.5, 0.8));
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn retained_geometry_is_available_after_layout_evaluation() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = Arc::new(StdMutex::new(None::<Geometry>));
		let geometry_for_task = Arc::clone(&geometry);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let geometry = Arc::clone(&geometry_for_task);
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default().clip(false));
				let mut button = frame.element("button").container(
					Container::default()
						.width(30.into())
						.height(20.into())
						.absolute_position(12, 18),
				);

				assert_eq!(button.geometry(), None);
				button.render().await;
				*geometry.lock().expect("expected test value") = button.geometry();
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*geometry.lock().expect("expected test value"), None);

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(
			*geometry.lock().expect("expected test value"),
			Some(Geometry::new(Location3::new(12, 18, 1), Size::new(30, 20)))
		);
	}

	#[test]
	fn retained_geometry_updates_after_property_mutation() {
		let frame_allocator = bumpalo::Bump::new();
		let geometry = Arc::new(StdMutex::new(None::<Geometry>));
		let geometry_for_task = Arc::clone(&geometry);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let geometry = Arc::clone(&geometry_for_task);
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default().clip(false));
				let mut button = frame.element("button").container(
					Container::default()
						.width(30.into())
						.height(20.into())
						.absolute_position(12, 18),
				);

				button.render().await;

				assert!(button.update_container(|container| {
					container.width = Sizing::pixels(40);
					container.set_position((24, 36));
				}));
				button.render().await;
				*geometry.lock().expect("expected test value") = button.geometry();
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(
			*geometry.lock().expect("expected test value"),
			Some(Geometry::new(Location3::new(24, 36, 1), Size::new(40, 20)))
		);
	}

	#[test]
	fn wait_future_resumes_mounted_task_after_duration() {
		let frame_allocator = bumpalo::Bump::new();
		let hits = Arc::new(AtomicUsize::new(0));
		let hits_for_task = Arc::clone(&hits);
		let mut engine = Engine::new();

		engine.mount(move |_| {
			let hits = Arc::clone(&hits_for_task);
			Box::pin(async move {
				crate::ui::wait(Duration::from_millis(5)).await;
				hits.fetch_add(1, Ordering::SeqCst);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 0);

		std::thread::sleep(Duration::from_millis(20));
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn context_seconds_returns_timer_future() {
		let frame_allocator = bumpalo::Bump::new();
		let hits = Arc::new(AtomicUsize::new(0));
		let hits_for_task = Arc::clone(&hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let hits = Arc::clone(&hits_for_task);
			Box::pin(async move {
				ctx.seconds(0).await;
				hits.fetch_add(1, Ordering::SeqCst);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn focused_key_goes_to_most_recent_focus_target() {
		let frame_allocator = bumpalo::Bump::new();
		let first_hits = Arc::new(AtomicUsize::new(0));
		let second_hits = Arc::new(AtomicUsize::new(0));
		let first_hits_for_task = Arc::clone(&first_hits);
		let second_hits_for_task = Arc::clone(&second_hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let first_hits = Arc::clone(&first_hits_for_task);
			let second_hits = Arc::clone(&second_hits_for_task);
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame.element("first").component(move |ctx| {
					let first_hits = Arc::clone(&first_hits);
					Box::pin(async move {
						let mut first = ctx.element("button").container(Container::default());
						first.request_focus();
						first.on_key(Key::Escape).await;
						first_hits.fetch_add(1, Ordering::SeqCst);
					})
				});
				frame.element("second").component(move |ctx| {
					let second_hits = Arc::clone(&second_hits);
					Box::pin(async move {
						let mut second = ctx.element("button").container(Container::default());
						second.request_focus();
						second.on_key(Key::Escape).await;
						second_hits.fetch_add(1, Ordering::SeqCst);
					})
				});
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first_hits.load(Ordering::SeqCst), 0);
		assert_eq!(second_hits.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn requesting_focus_again_moves_target_without_duplication() {
		let frame_allocator = bumpalo::Bump::new();
		let first_hits = Arc::new(AtomicUsize::new(0));
		let second_hits = Arc::new(AtomicUsize::new(0));
		let first_hits_for_task = Arc::clone(&first_hits);
		let second_hits_for_task = Arc::clone(&second_hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let first_hits = Arc::clone(&first_hits_for_task);
			let second_hits = Arc::clone(&second_hits_for_task);
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				let mut first = frame.element("first").container(Container::default());
				let mut second = frame.element("second").container(Container::default());
				first.request_focus();
				second.request_focus();
				first.request_focus();

				first.on_key(Key::Escape).await;
				first_hits.fetch_add(1, Ordering::SeqCst);
				first.release_focus();
				ctx.render().await;
				second.on_key(Key::Escape).await;
				second_hits.fetch_add(1, Ordering::SeqCst);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		engine.update_key_state(Key::Escape, false);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first_hits.load(Ordering::SeqCst), 1);
		assert_eq!(second_hits.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn releasing_focus_reveals_previous_focus_target() {
		let frame_allocator = bumpalo::Bump::new();
		let first_hits = Arc::new(AtomicUsize::new(0));
		let second_hits = Arc::new(AtomicUsize::new(0));
		let first_hits_for_task = Arc::clone(&first_hits);
		let second_hits_for_task = Arc::clone(&second_hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let first_hits = Arc::clone(&first_hits_for_task);
			let second_hits = Arc::clone(&second_hits_for_task);
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame.element("first").component(move |ctx| {
					let first_hits = Arc::clone(&first_hits);
					Box::pin(async move {
						let mut first = ctx.element("button").container(Container::default());
						first.request_focus();
						first.on_key(Key::Escape).await;
						first_hits.fetch_add(1, Ordering::SeqCst);
					})
				});
				frame.element("second").component(move |ctx| {
					let second_hits = Arc::clone(&second_hits);
					Box::pin(async move {
						let mut second = ctx.element("button").container(Container::default());
						second.request_focus();
						second.release_focus();
						second.on_key(Key::Escape).await;
						second_hits.fetch_add(1, Ordering::SeqCst);
					})
				});
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first_hits.load(Ordering::SeqCst), 1);
		assert_eq!(second_hits.load(Ordering::SeqCst), 0);
	}

	#[test]
	fn escape_release_does_not_wake_key_future() {
		let frame_allocator = bumpalo::Bump::new();
		let hits = Arc::new(AtomicUsize::new(0));
		let hits_for_task = Arc::clone(&hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let hits = Arc::clone(&hits_for_task);
			Box::pin(async move {
				let mut button = ctx.element("button").container(Container::default());
				button.request_focus();
				button.on_key(Key::Escape).await;
				hits.fetch_add(1, Ordering::SeqCst);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, false);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 0);

		engine.update_key_state(Key::Escape, true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 1);
	}

	#[derive(Clone)]
	struct TestContext {
		value: u32,
	}

	trait TestUiContext = Context<TestContext>;

	#[test]
	fn components_can_access_engine_context() {
		let frame_allocator = bumpalo::Bump::new();
		let seen = Arc::new(StdMutex::new(Vec::new()));
		let seen_for_task = Arc::clone(&seen);
		let mut engine = Engine::with_context(TestContext { value: 7 });

		engine.mount(move |ctx| {
			let seen = Arc::clone(&seen_for_task);
			Box::pin(async move {
				seen.lock().expect("expected test value").push(ctx.ctx().value);

				ctx.element("child")
					.component(move |ctx: &mut EvaluationContext<TestContext>| {
						let seen = Arc::clone(&seen);
						Box::pin(async move {
							seen.lock().expect("expected test value").push(ctx.ctx().value + 1);
						})
					});
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*seen.lock().expect("expected test value"), vec![7, 8]);
	}

	#[test]
	fn mounted_component_can_access_engine_context() {
		async fn modal(ctx: &mut impl TestUiContext) -> u32 {
			ctx.ctx().value
		}

		let frame_allocator = bumpalo::Bump::new();
		let result = Arc::new(StdMutex::new(None));
		let result_for_task = Arc::clone(&result);
		let mut engine = Engine::with_context(TestContext { value: 11 });

		engine.mount(move |ctx| {
			let result = Arc::clone(&result_for_task);
			Box::pin(async move {
				let value = ctx.element("modal").mount(|ctx| Box::pin(modal(ctx))).await;
				*result.lock().expect("expected test value") = Some(value);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*result.lock().expect("expected test value"), Some(11));
	}

	#[test]
	fn awaited_modal_blocks_caller_until_component_returns_value() {
		let frame_allocator = bumpalo::Bump::new();
		let result = Arc::new(StdMutex::new(None));
		let result_for_task = Arc::clone(&result);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let result = Arc::clone(&result_for_task);
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				let value = frame
					.element("modal")
					.mount(|ctx| {
						Box::pin(async move {
							let mut button = ctx.element("button").container(Container::default());
							button.on(Events::Actuated).await;
							42
						})
					})
					.await;
				*result.lock().expect("expected test value") = Some(value);
			})
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements.len(), 2);
		assert_eq!(*result.lock().expect("expected test value"), None);

		engine.set_cursor_position(UiPoint::zero());
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*result.lock().expect("expected test value"), Some(42));
	}

	#[test]
	fn awaited_modal_subtree_is_removed_after_component_returns() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame
					.element("modal")
					.mount(|ctx| {
						Box::pin(async move {
							let mut button = ctx.element("button").container(Container::default());
							button.on(Events::Actuated).await;
						})
					})
					.await;
			})
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

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				let mut modal = frame.element("modal").mount(|ctx| {
					Box::pin(async move {
						let mut button = ctx.element("button").container(Container::default());
						button.on(Events::Actuated).await;
					})
				});

				utils::r#async::select! {
					_ = modal => {}
					_ = ctx.render() => {}
				}
			})
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements.len(), 2);

		let after_drop = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(after_drop.elements.len(), 1);
	}

	#[test]
	fn removing_focused_mounted_modal_reveals_previous_focus_target() {
		let frame_allocator = bumpalo::Bump::new();
		let background_hits = Arc::new(AtomicUsize::new(0));
		let background_hits_for_task = Arc::clone(&background_hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let background_hits = Arc::clone(&background_hits_for_task);
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame.element("background").component(move |ctx| {
					let background_hits = Arc::clone(&background_hits);
					Box::pin(async move {
						let mut background = ctx.element("button").container(Container::default());
						background.request_focus();
						background.on_key(Key::Escape).await;
						background_hits.fetch_add(1, Ordering::SeqCst);
					})
				});

				let mut modal = frame.element("modal").mount(|ctx| {
					Box::pin(async move {
						let mut modal = ctx.element("window").container(Container::default());
						modal.request_focus();
						modal.on_key(Key::Escape).await;
					})
				});

				utils::r#async::select! {
					_ = modal => {}
					_ = ctx.render() => {}
				}
			})
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements.len(), 3);

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(second.elements.len(), 2);

		engine.update_key_state(Key::Escape, true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(background_hits.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn awaited_modal_can_return_cancelled_from_escape() {
		#[derive(Debug, PartialEq, Eq)]
		enum Result {
			Confirmed,
			Cancelled,
		}

		async fn modal(ctx: &mut impl Context) -> Result {
			let mut window = ctx.element("window").container(Container::default());
			let mut ok = window.element("ok").container(Container::default());
			window.request_focus();

			utils::r#async::select! {
				_ = ok.on(Events::Actuated) => Result::Confirmed,
				_ = window.on_key(Key::Escape) => Result::Cancelled,
			}
		}

		let frame_allocator = bumpalo::Bump::new();
		let result = Arc::new(StdMutex::new(None));
		let result_for_task = Arc::clone(&result);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let result = Arc::clone(&result_for_task);
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				let value = frame.element("modal").mount(|ctx| Box::pin(modal(ctx))).await;
				*result.lock().expect("expected test value") = Some(value);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*result.lock().expect("expected test value"), Some(Result::Cancelled));
	}

	#[test]
	// The retained-modal fixture intentionally nests async component declarations to exercise stable structural IDs.
	#[allow(clippy::excessive_nesting)]
	fn reopening_awaited_modal_reuses_stable_ids() {
		let frame_allocator = bumpalo::Bump::new();
		let ids = Arc::new(StdMutex::new(Vec::new()));
		let ids_for_task = Arc::clone(&ids);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let ids = Arc::clone(&ids_for_task);
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				for _ in 0..2 {
					let ids = Arc::clone(&ids);
					frame
						.element("modal")
						.mount(move |ctx| {
							let ids = Arc::clone(&ids);
							Box::pin(async move {
								let mut button = ctx.element("button").container(Container::default());
								ids.lock().expect("expected test value").push(button.id());
								button.on(Events::Actuated).await;
							})
						})
						.await;
				}
			})
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements.len(), 2);

		engine.set_cursor_position(UiPoint::zero());
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(second.elements.len(), 2);

		let ids = ids.lock().expect("expected test value");

		assert_eq!(ids.len(), 2);
		assert_eq!(ids[0], ids[1]);
	}

	#[test]
	fn awaited_modal_can_mount_absolute_depth_container_above_opener() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame.element("opener").container(Container::default());
				frame
					.element("modal")
					.mount(|ctx| {
						Box::pin(async move {
							let mut modal = ctx
								.element("modal_container")
								.container(Container::default().depth(Depth::absolute(1)));
							modal.element("button").container(Container::default());
							modal.on(Events::Actuated).await;
						})
					})
					.await;
			})
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

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut opener = ctx
					.element("opener")
					.container(Container::default().width(20.into()).height(20.into()));
				opener
					.element("modal")
					.mount(|ctx| {
						Box::pin(async move {
							let mut modal = ctx.element("modal_container").container(
								Container::default()
									.width(80.into())
									.height(30.into())
									.depth(Depth::absolute(1))
									.absolute_position(30, 0),
							);
							modal.on(Events::Actuated).await;
						})
					})
					.await;
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

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

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default().clip(false));
				let mut high = frame
					.element("high")
					.container(Container::default().depth(10).size(20.into()));
				high.element("text").text(Text::new("high"));
				let mut low = frame.element("low").container(Container::default().size(20.into()));
				low.element("text").text(Text::new("low"));
				frame
					.element("tie")
					.container(Container::default().size(20.into()))
					.element("text")
					.text(Text::new("tie"));
				ctx.render().await;
				low.update_container(|value| value.set_opacity(0.5));
				ctx.render().await;
				high.update_container(|value| value.depth = Depth::relative(0));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let depths = render.elements().map(|element| element.position.z()).collect::<Vec<_>>();

		assert_eq!(depths, vec![0, 1, 1, 10]);
		let ids = render.elements().map(|element| element.id).collect::<Vec<_>>();
		assert_eq!(
			render.texts().map(|element| element.content.as_str()).collect::<Vec<_>>(),
			["low", "tie", "high"]
		);

		let mut painted = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut painted);
		assert_eq!(render.elements().map(|element| element.id).collect::<Vec<_>>(), ids);
		assert_eq!(
			render.texts().map(|element| element.content.as_str()).collect::<Vec<_>>(),
			["low", "tie", "high"]
		);

		let mut reordered = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut reordered);
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

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("frame").container(
					Container::default().style(ConcreteLayer::default().color(RGBA::new(0.2, 0.3, 0.4, 1.0).into())),
				);
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

		assert_eq!(render.elements().next().unwrap().style.layers().len(), 1);
		assert_eq!(render.elements().next().unwrap().style.layers()[0].kind(), LayerKind::Fill);
		match Layer::fill(&render.elements().next().unwrap().style.layers()[0]) {
			Color::Value(color) => assert_eq!(*color, RGBA::new(0.2, 0.3, 0.4, 1.0)),
			Color::Sample(_) => panic!("expected value color"),
		}
	}

	#[test]
	fn render_preserves_container_backdrop_blur_radius() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("frame")
					.container(Container::default().style(ConcreteLayer::default().backdrop_blur(18.0)));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

		assert_eq!(render.elements().next().unwrap().backdrop_blur_radius, 18.0);
	}

	#[test]
	fn render_backdrop_blur_does_not_change_opacity_or_clip() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("frame").container(
					Container::default()
						.width(20.into())
						.height(20.into())
						.opacity(0.5)
						.style(ConcreteLayer::default().backdrop_blur(12.0)),
				);
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

		assert_eq!(render.elements().next().unwrap().backdrop_blur_radius, 12.0);
		assert_eq!(render.elements().next().unwrap().opacity, 0.5);
		assert_eq!(render.elements().next().unwrap().clip, None);
	}

	#[test]
	fn render_preserves_layered_container_style() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("frame").container(
					Container::default().style(
						ConcreteStyle::new()
							.layer(ConcreteLayer::default().color(RGBA::new(0.2, 0.3, 0.4, 1.0).into()))
							.layer(
								ConcreteLayer::default()
									.color(RGBA::new(0.9, 0.8, 0.7, 1.0).into())
									.stroke(2.0),
							),
					),
				);
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

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

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(
					Container::default().width(50.into()).height(40.into()).style(
						ConcreteLayer::default()
							.color(RGBA::white().into())
							.feather(EdgeFeather::vertical(8.0)),
					),
				);
				frame
					.element("child")
					.container(Container::default().width(10.into()).height(10.into()));
				frame.element("label").text(Text::new("Masked"));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let parent = render
			.elements()
			.find(|element| element.id == 1)
			.expect("expected test value");
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");
		let text = render.texts().find(|text| text.id == 3).expect("expected test value");
		let expected = FeatherMask {
			geometry: Geometry::new(Location3::new(0, 0, 0), Size::new(50, 40)),
			feather: EdgeFeather::vertical(8.0),
			corner_radius: 0.0,
			corner_exponent: 2.0,
		};

		assert_eq!(parent.feather_mask, None);
		assert_eq!(child.feather_mask, Some(expected));
		assert_eq!(text.feather_mask, Some(expected));
	}

	#[test]
	fn clip_false_prevents_layer_feather_mask_inheritance() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(
					Container::default()
						.width(50.into())
						.height(40.into())
						.clip(false)
						.style(ConcreteLayer::default().feather(EdgeFeather::all(8.0))),
				);
				frame
					.element("child")
					.container(Container::default().width(10.into()).height(10.into()));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");

		assert_eq!(child.feather_mask, None);
	}

	#[test]
	fn first_nonzero_feathered_layer_defines_descendant_mask() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(
					Container::default().width(50.into()).height(40.into()).style(
						ConcreteStyle::new()
							.layer(ConcreteLayer::default())
							.layer(ConcreteLayer::default().feather(EdgeFeather::horizontal(4.0)))
							.layer(ConcreteLayer::default().feather(EdgeFeather::vertical(9.0))),
					),
				);
				frame
					.element("child")
					.container(Container::default().width(10.into()).height(10.into()));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");

		assert_eq!(
			child.feather_mask,
			Some(FeatherMask {
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

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(
					Container::default()
						.width(50.into())
						.height(40.into())
						.corner_radius(8.0)
						.corner_exponent(4.0)
						.style(ConcreteLayer::default().feather(EdgeFeather::vertical(8.0))),
				);
				frame
					.element("child")
					.container(Container::default().width(10.into()).height(10.into()));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let child = render
			.elements()
			.find(|element| element.id == 2)
			.expect("expected test value");
		let mask = child.feather_mask.expect("expected test value");

		assert_eq!(mask.corner_radius, 8.0);
		assert_eq!(mask.corner_exponent, 4.0);
	}

	#[test]
	fn render_inherits_parent_opacity() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx
					.element("frame")
					.container(Container::default().width(10.into()).height(10.into()).opacity(0.5));
				frame
					.element("child")
					.container(Container::default().width(10.into()).height(10.into()));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
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

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx
					.element("frame")
					.container(Container::default().width(10.into()).height(10.into()).opacity(0.5));
				frame
					.element("child")
					.container(Container::default().width(10.into()).height(10.into()).opacity(0.25));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
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

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx
					.element("frame")
					.container(Container::default().width(10.into()).height(10.into()).opacity(0.5));
				frame
					.element("label")
					.text(Text::new("Hello").style(ConcreteLayer::default().color(RGBA::new(1.0, 1.0, 1.0, 0.8).into())));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let text = render.texts().next().expect("expected test value");

		assert_eq!(text.opacity, 0.5);
		assert_eq!(text.color, RGBA::new(1.0, 1.0, 1.0, 0.8));
	}

	#[test]
	fn render_uses_shape_opacity_from_settings() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("shape").shape(Shape::new(
					Container::default().width(10.into()).height(10.into()).opacity(0.4),
				));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

		assert_eq!(render.elements().next().unwrap().opacity, 0.4);
	}

	#[test]
	fn render_sanitizes_opacity() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut root = ctx
					.element("root")
					.container(Container::default().width(10.into()).height(10.into()).clip(false));
				root.element("negative")
					.container(Container::default().width(10.into()).height(10.into()).opacity(-1.0));
				root.element("invalid")
					.container(Container::default().width(10.into()).height(10.into()).opacity(f32::NAN));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

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

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("frame")
					.container(Container::default().corner_radius(8.0).corner_exponent(4.0));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

		assert_eq!(render.elements().next().unwrap().corner_radius, 8.0);
		assert_eq!(render.elements().next().unwrap().corner_exponent, 4.0);
	}

	#[test]
	fn render_uses_shape_corner_exponent_from_settings() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("shape").shape(Shape::new(
					Container::default()
						.width(20.into())
						.height(20.into())
						.corner_radius(6.0)
						.corner_exponent(4.0),
				));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

		assert_eq!(render.elements().next().unwrap().corner_radius, 6.0);
		assert_eq!(render.elements().next().unwrap().corner_exponent, 4.0);
	}

	#[test]
	fn render_uses_container_transform_after_layout() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("frame").container(
					Container::default()
						.width(20.into())
						.height(10.into())
						.transform(Transform::identity().translate_y(6.0).scale(0.5)),
				);
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let frame = render.elements().next().expect("expected test value");

		assert_eq!(frame.position, Location3::new(5.0, 8.5, 0));
		assert_eq!(frame.size, Size::new(10, 5));
	}

	#[test]
	fn child_visual_bounds_inherit_parent_transform() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(
					Container::default()
						.width(100.into())
						.height(100.into())
						.flow(flow::row)
						.transform(Transform::identity().translate_y(10.0).scale(0.5)),
				);
				frame
					.element("child")
					.container(Container::default().width(20.into()).height(10.into()));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
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
		let hits = Arc::new(AtomicUsize::new(0));
		let hits_for_task = Arc::clone(&hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let hits = Arc::clone(&hits_for_task);
			Box::pin(async move {
				let mut button = ctx.element("button").container(
					Container::default()
						.width(20.into())
						.height(20.into())
						.transform(Transform::identity().translate(40.0, 40.0)),
				);

				loop {
					button.on(Events::Actuated).await;
					hits.fetch_add(1, Ordering::SeqCst);
				}
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.0, 0.0));
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn opacity_does_not_disable_hit_testing() {
		let frame_allocator = bumpalo::Bump::new();
		let hits = Arc::new(AtomicUsize::new(0));
		let hits_for_task = Arc::clone(&hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let hits = Arc::clone(&hits_for_task);
			Box::pin(async move {
				let mut button = ctx.element("button").container(Container::default().opacity(0.0));

				loop {
					button.on(Events::Actuated).await;
					hits.fetch_add(1, Ordering::SeqCst);
				}
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::new(0.0, 0.0));
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 1);
	}

	#[test]
	fn update_container_changes_later_layout_and_render_style() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx
					.element("frame")
					.container(Container::default().width(10.into()).height(10.into()));
				frame.render().await;

				assert!(frame.update_container(|container| {
					container.width = Sizing::pixels(30);
					container.set_style(ConcreteLayer::default().color(RGBA::new(0.4, 0.5, 0.6, 1.0).into()));
				}));
			})
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(first.elements[0].size, Size::new(10, 10));

		let mut second = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(second.elements[0].size, Size::new(30, 10));

		let render = engine.render(&mut second);
		match Layer::fill(&render.elements().next().unwrap().style.layers()[0]) {
			Color::Value(color) => assert_eq!(*color, RGBA::new(0.4, 0.5, 0.6, 1.0)),
			Color::Sample(_) => panic!("expected value color"),
		}
	}

	#[test]
	fn update_container_changes_later_render_opacity() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame.render().await;

				assert!(frame.update_container(|container| {
					container.set_opacity(0.25);
				}));
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);

		assert_eq!(render.elements().next().unwrap().opacity, 0.25);
	}

	#[test]
	fn update_text_changes_later_render_style() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut text = ctx.element("label").text(Text::new("Hello"));
				text.render().await;

				assert!(text.update_text(|text| {
					text.set_content("Updated");
					text.set_style(ConcreteLayer::default().color(RGBA::new(0.7, 0.8, 0.9, 1.0).into()));
				}));
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
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

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("field").text_field(TextField::new("Hello"));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let text = render.texts().next().expect("expected test value");

		assert_eq!(text.content, "Hello");
		assert!(text.size.x() > 0.0);
		assert!(render.texts().nth(1).is_none());
	}

	#[test]
	fn focused_text_field_receives_inserted_text_edit() {
		let frame_allocator = bumpalo::Bump::new();
		let received = Arc::new(StdMutex::new(None));
		let received_for_task = Arc::clone(&received);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let received = Arc::clone(&received_for_task);
			Box::pin(async move {
				let mut field = ctx.element("field").text_field(TextField::new(""));
				field.request_focus();
				let event = field.on_text_edit().await;
				*received.lock().expect("expected test value") = Some(event.edit);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.input_character('a');
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*received.lock().expect("expected test value"), Some(TextEdit::Inserted('a')));
	}

	#[test]
	fn unfocused_text_field_does_not_receive_inserted_text_edit() {
		let frame_allocator = bumpalo::Bump::new();
		let received = Arc::new(StdMutex::new(None));
		let received_for_task = Arc::clone(&received);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let received = Arc::clone(&received_for_task);
			Box::pin(async move {
				let mut field = ctx.element("field").text_field(TextField::new(""));
				let event = field.on_text_edit().await;
				*received.lock().expect("expected test value") = Some(event.edit);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.input_character('a');
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*received.lock().expect("expected test value"), None);
	}

	#[test]
	fn focused_text_field_delete_emits_deleted_last_character() {
		let frame_allocator = bumpalo::Bump::new();
		let received = Arc::new(StdMutex::new(None));
		let received_for_task = Arc::clone(&received);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let received = Arc::clone(&received_for_task);
			Box::pin(async move {
				let mut field = ctx.element("field").text_field(TextField::new("Hié"));
				field.request_focus();
				let event = field.on_text_edit().await;
				*received.lock().expect("expected test value") = Some(event.edit);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.delete_text_backward();
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*received.lock().expect("expected test value"), Some(TextEdit::Deleted('é')));
	}

	#[test]
	fn app_owned_string_update_changes_later_text_field_render() {
		let frame_allocator = bumpalo::Bump::new();
		let content = Arc::new(StdMutex::new(String::from("a")));
		let content_for_task = Arc::clone(&content);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let content = Arc::clone(&content_for_task);
			Box::pin(async move {
				let initial = content.lock().expect("expected test value").clone();
				let mut field = ctx.element("field").text_field(TextField::new(initial));
				field.request_focus();
				let event = field.on_text_edit().await;
				{
					let mut content = content.lock().expect("expected test value");
					event.edit.apply_to(&mut content);
					let updated = content.clone();

					assert!(field.update_text_field(|field| field.set_content(updated)));
				}
				field.render().await;
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.input_character('b');
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let text = render.texts().next().expect("expected test value");

		assert_eq!(*content.lock().expect("expected test value"), "ab");
		assert_eq!(text.content, "ab");
	}

	#[test]
	fn centered_flow_overlays_full_size_curve_children() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx
					.element("frame")
					.container(Container::default().width(100.into()).height(50.into()).flow(flow::center));
				frame.element("first").curve(Curve::new(
					CurvePath::new(100.into(), 50.into()).line((0.0, 10.0), (100.0, 10.0)),
				));
				frame
					.element("second")
					.curve(Curve::new(CurvePath::new(100.into(), 50.into()).quadratic(
						(0.0, 40.0),
						(50.0, 0.0),
						(100.0, 40.0),
					)));
			})
		});

		let mut snapshot = engine.evaluate(Size::new(200, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let curves: std::vec::Vec<_> = render.curves().collect();

		assert_eq!(curves.len(), 2);
		assert_eq!(curves[0].position, curves[1].position);
		assert_eq!(curves[0].size, Size::new(100, 50));
		assert_eq!(curves[1].size, Size::new(100, 50));
	}

	#[test]
	fn animate_updates_existing_retained_element_across_frames() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx
					.element("frame")
					.container(Container::default().width(10.into()).height(10.into()));
				animate(&mut frame, spring(0.0, 1.0), |frame, t| {
					frame.update_container(|container| {
						container.width = Sizing::pixels(10 + (90.0 * t.clamp(0.0, 1.0)) as u32);
					});
				})
				.await;
			})
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
		let hits = Arc::new(AtomicUsize::new(0));
		let hits_for_task = Arc::clone(&hits);
		let background_hits = Arc::new(AtomicUsize::new(0));
		let background_hits_for_task = Arc::clone(&background_hits);
		let mut engine = Engine::new();

		engine.mount(move |ctx| {
			let hits = Arc::clone(&hits_for_task);
			let background_hits = Arc::clone(&background_hits_for_task);
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
				frame.element("background").component(move |ctx| {
					let background_hits = Arc::clone(&background_hits);
					Box::pin(async move {
						let mut background = ctx.element("button").container(Container::default());
						background.on(Events::Actuated).await;
						background_hits.fetch_add(1, Ordering::SeqCst);
					})
				});
				frame
					.element("modal")
					.mount(move |ctx| {
						let hits = Arc::clone(&hits);
						Box::pin(async move {
							let mut backdrop = ctx
								.element("backdrop")
								.container(Container::default().depth(Depth::absolute(1)));
							backdrop.on(Events::Actuated).await;
							hits.fetch_add(1, Ordering::SeqCst);
						})
					})
					.await;
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(UiPoint::zero());
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(hits.load(Ordering::SeqCst), 1);
		assert_eq!(background_hits.load(Ordering::SeqCst), 0);
	}
}

use std::{
	boxed::Box,
	cell::RefCell,
	collections::{HashMap, HashSet, VecDeque},
	future::Future,
	marker::PhantomData,
	pin::Pin,
	rc::Rc,
	sync::Arc,
	task::{Context as TaskContext, Poll, Wake, Waker},
};

use utils::{RGBA, StableVec, StableVecHandle, r#async::FusedFuture, sync::Mutex};

use super::{
	ConcreteElement, FeatherMask, Geometry, IdedElement, LayoutElement, RenderCurveElement, RenderElement, RenderImageElement,
	RenderTextElement,
	context::{Context, ElementContext, ElementSlot, MountedUiFuture, UiFuture},
	element::{ElementHandle, Id},
	flow::{Location3, Size},
	layout_elements,
	retained_tree::RetainedTree,
	snapshot::Snapshot,
	visual_transform::Affine2,
};
use crate::ui::{
	Container, Depth, Text, Transform, UiPoint, UiVector,
	components::{curve::Curve, image::Image, shape::Shape, text_field::TextField},
	drag::{Drag, DragCapture, DragDrop},
	font::TextSystem,
	intersection::MouseClickAcceleration,
	primitive::{Events, Key, Primitive as _, Primitives, Shapes, TextEdit},
	style::{Color, ConcreteStyle, EdgeFeather, Layer as _, LayerKind},
};
