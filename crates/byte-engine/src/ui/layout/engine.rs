//! UI retained tree evaluation, interaction state, and render snapshots.

/// The [`Engine`] struct owns UI evaluation state, text shaping, and pointer
/// interaction across viewports.
pub struct Engine<C = ()> {
	viewports: Vec<VirtualViewport>,
	state: Rc<RefCell<EngineState>>,
	cursor_position: Vector2,
	is_clicking: bool,
	clicks: Vec<bool>,
	scrolls: Vec<Vector2>,
	key_states: HashMap<Key, bool>,
	key_presses: VecDeque<Key>,
	text_edits: VecDeque<TextEdit>,
	text_system: TextSystem,
	ctx: Rc<C>,
	runtime: Rc<RefCell<Runtime>>,
}

pub(super) struct EngineState {
	element_ids: HashSet<Id>,
	cursor: Option<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerState {
	pub position: Vector2,
	pub pressed: bool,
}

impl Default for PointerState {
	fn default() -> Self {
		Self {
			position: Vector2::zero(),
			pressed: false,
		}
	}
}

#[derive(Clone, Copy)]
enum EffectiveClip {
	Unbounded,
	Empty,
	Rect(Geometry),
}

#[derive(Clone, Copy)]
struct ClipInfo {
	element: EffectiveClip,
	descendants: EffectiveClip,
}

#[derive(Clone, Copy)]
struct FeatherInfo {
	element: Option<FeatherMask>,
	descendants: Option<FeatherMask>,
}

impl EffectiveClip {
	fn apply(self, geometry: Geometry) -> Option<Geometry> {
		match self {
			EffectiveClip::Unbounded => Some(geometry),
			EffectiveClip::Empty => None,
			EffectiveClip::Rect(clip) => geometry.intersect(clip),
		}
	}

	fn clip_descendants(self, geometry: Geometry) -> Self {
		match self.apply(geometry) {
			Some(geometry) => EffectiveClip::Rect(geometry),
			None => EffectiveClip::Empty,
		}
	}

	fn as_rect(self) -> Option<Geometry> {
		match self {
			EffectiveClip::Rect(geometry) => Some(geometry),
			EffectiveClip::Unbounded | EffectiveClip::Empty => None,
		}
	}
}

fn geometry_from_layout_element(element: &LayoutElement) -> Geometry {
	Geometry::new(element.position, element.size)
}

fn element_clips<'a>(elements: impl IntoIterator<Item = &'a LayoutElement>, tree: &RetainedTree) -> HashMap<Id, ClipInfo> {
	let mut clips = HashMap::new();

	for element in elements {
		let parent_clip = tree
			.parent_by_child
			.get(&element.id)
			.copied()
			.and_then(|parent| clips.get(&parent).map(|clip: &ClipInfo| clip.descendants))
			.unwrap_or(EffectiveClip::Unbounded);
		let inherited = if element_resets_clip(element.id, tree) {
			EffectiveClip::Unbounded
		} else {
			parent_clip
		};
		let geometry = geometry_from_layout_element(element);
		let descendants = match tree.element(element.id).map(|element| &element.element.primitive) {
			Some(Primitives::Container(container)) if container.clip => inherited.clip_descendants(geometry),
			_ => inherited,
		};

		clips.insert(
			element.id,
			ClipInfo {
				element: inherited,
				descendants,
			},
		);
	}

	clips
}

fn element_feather_masks<'a>(
	elements: impl IntoIterator<Item = &'a LayoutElement>,
	tree: &RetainedTree,
) -> HashMap<Id, FeatherInfo> {
	let mut masks = HashMap::new();

	for element in elements {
		let parent_mask = tree
			.parent_by_child
			.get(&element.id)
			.copied()
			.and_then(|parent| masks.get(&parent).and_then(|mask: &FeatherInfo| mask.descendants));
		let inherited = if element_resets_clip(element.id, tree) {
			None
		} else {
			parent_mask
		};
		let descendants = match tree.element(element.id).map(|element| &element.element.primitive) {
			Some(Primitives::Container(container)) if container.clip => first_layer_feather(container.style.layers())
				.map(|feather| FeatherMask {
					geometry: geometry_from_layout_element(element),
					feather,
					corner_radius: container.corner_radius,
					corner_exponent: container.corner_exponent,
				})
				.or(inherited),
			_ => inherited,
		};

		masks.insert(
			element.id,
			FeatherInfo {
				element: inherited,
				descendants,
			},
		);
	}

	masks
}

fn element_resets_clip(id: Id, tree: &RetainedTree) -> bool {
	matches!(
		tree.element(id).map(|element| &element.element.primitive),
		Some(Primitives::Container(container)) if matches!(container.depth, Depth::Absolute(_))
	)
}

fn first_layer_feather(layers: &[crate::ui::style::ConcreteLayer]) -> Option<EdgeFeather> {
	layers
		.iter()
		.map(crate::ui::style::Layer::feather)
		.find(|feather| !feather.is_none())
}

fn clipped_layout_elements<'a>(
	elements: &[LayoutElement],
	tree: &RetainedTree,
	frame_allocator: &'a bumpalo::Bump,
) -> Vec<LayoutElement, &'a bumpalo::Bump> {
	let clips = element_clips(elements, tree);
	let mut clipped = Vec::with_capacity_in(elements.len(), frame_allocator);

	for element in elements {
		let Some(geometry) = clips
			.get(&element.id)
			.map(|clip| clip.element)
			.unwrap_or(EffectiveClip::Unbounded)
			.apply(geometry_from_layout_element(element))
		else {
			continue;
		};

		if geometry.is_empty() {
			continue;
		}

		clipped.push(LayoutElement {
			id: element.id,
			position: Location3::new(geometry.x(), geometry.y(), element.position.z()),
			size: geometry.size,
			hit_testable: element.hit_testable,
		});
	}

	clipped
}

fn apply_visual_transforms<'a>(elements: &mut [LayoutElement], tree: &RetainedTree, frame_allocator: &'a bumpalo::Bump) {
	let mut resolved = Vec::with_capacity_in(tree.elements.len(), frame_allocator);
	for _ in 0..tree.elements.len() {
		resolved.push(None);
	}

	for element in elements {
		let parent_transform = tree
			.parent_by_child
			.get(&element.id)
			.copied()
			.and_then(|parent| tree.element_indices.get(&parent).and_then(|index| resolved.get(*index)))
			.and_then(|transform| *transform)
			.unwrap_or_else(Affine2::identity);

		let local_transform = tree
			.element(element.id)
			.map(|retained_element| *retained_element.element.primitive.transform())
			.unwrap_or_default();
		let transform = parent_transform.compose(Affine2::from_transform(local_transform, element));
		let (position, size) = transform.transform_rect(element);

		element.position = position;
		element.size = size;
		if let Some(index) = tree.element_indices.get(&element.id).copied() {
			resolved[index] = Some(transform);
		}
	}
}

/// Context owned by a mounted async UI task.
pub struct EvaluationContext<C = ()> {
	id: Id,
	parent: Option<Id>,
	path: Vec<PathSegment>,
	ctx: Rc<C>,
	runtime: Rc<RefCell<Runtime>>,
	tree: Rc<RefCell<RetainedTree>>,
	task_id: TaskId,
}

impl<C> EvaluationContext<C> {
	fn new_root(ctx: Rc<C>, runtime: Rc<RefCell<Runtime>>, tree: Rc<RefCell<RetainedTree>>, task_id: TaskId) -> Self {
		Self {
			id: Id::new(1).unwrap(),
			parent: None,
			path: Vec::new(),
			ctx,
			runtime,
			tree,
			task_id,
		}
	}

	fn new_child(
		ctx: Rc<C>,
		runtime: Rc<RefCell<Runtime>>,
		tree: Rc<RefCell<RetainedTree>>,
		task_id: TaskId,
		id: Id,
		path: Vec<PathSegment>,
	) -> Self {
		Self {
			id,
			parent: Some(id),
			path,
			ctx,
			runtime,
			tree,
			task_id,
		}
	}

	fn add_element(&mut self, name: &'static str, element: ConcreteElement) -> EvaluationContext<C> {
		let (id, path) = self.tree.borrow_mut().add_element(self.parent, &self.path, name, element);
		EvaluationContext::new_child(
			Rc::clone(&self.ctx),
			Rc::clone(&self.runtime),
			Rc::clone(&self.tree),
			self.task_id,
			id,
			path,
		)
	}

	pub fn update_container(&mut self, update: impl FnOnce(&mut Container)) -> bool {
		let mut tree = self.tree.borrow_mut();
		let Some(element) = tree.element_mut(self.id) else {
			return false;
		};
		let Primitives::Container(container) = &mut element.element.primitive else {
			return false;
		};

		update(container);
		true
	}

	pub fn update_text(&mut self, update: impl FnOnce(&mut Text)) -> bool {
		let mut tree = self.tree.borrow_mut();
		let Some(element) = tree.element_mut(self.id) else {
			return false;
		};
		let Primitives::Text(text) = &mut element.element.primitive else {
			return false;
		};

		update(text);
		true
	}

	pub fn update_text_field(&mut self, update: impl FnOnce(&mut TextField)) -> bool {
		let mut tree = self.tree.borrow_mut();
		let Some(element) = tree.element_mut(self.id) else {
			return false;
		};
		let Primitives::TextField(text_field) = &mut element.element.primitive else {
			return false;
		};

		update(text_field);
		true
	}

	pub fn update_shape(&mut self, update: impl FnOnce(&mut Shape)) -> bool {
		let mut tree = self.tree.borrow_mut();
		let Some(element) = tree.element_mut(self.id) else {
			return false;
		};
		let Primitives::Shape(shape) = &mut element.element.primitive else {
			return false;
		};

		update(shape);
		true
	}

	pub fn update_image(&mut self, update: impl FnOnce(&mut Image)) -> bool {
		let mut tree = self.tree.borrow_mut();
		let Some(element) = tree.element_mut(self.id) else {
			return false;
		};
		let Primitives::Image(image) = &mut element.element.primitive else {
			return false;
		};

		update(image);
		true
	}

	pub fn geometry(&self) -> Option<Geometry> {
		self.runtime.borrow().geometry.get(&self.id).copied()
	}

	pub fn pointer(&self) -> PointerState {
		self.runtime.borrow().pointer
	}
}

impl<C: 'static> Context<C> for EvaluationContext<C> {
	fn id(&self) -> Id {
		self.id
	}

	fn ctx(&self) -> &C {
		self.ctx.as_ref()
	}

	fn element<'a>(&'a mut self, name: &'static str) -> ElementSlot<'a, C> {
		ElementSlot { parent: self, name }
	}

	fn render(&mut self) -> RenderFuture {
		RenderFuture {
			runtime: Rc::clone(&self.runtime),
			frame_seen: None,
			complete: false,
		}
	}

	fn geometry(&self) -> Option<Geometry> {
		EvaluationContext::geometry(self)
	}

	fn pointer(&self) -> PointerState {
		EvaluationContext::pointer(self)
	}

	fn request_focus(&mut self) {
		self.runtime.borrow_mut().request_focus(self.id);
	}

	fn release_focus(&mut self) {
		self.runtime.borrow_mut().release_focus(self.id);
	}
}

impl<C: 'static> ElementContext<C> for ElementSlot<'_, C> {
	fn container(self, element: Container) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::container(element))
	}

	fn text(self, text: Text) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::text(text))
	}

	fn text_field(self, text_field: TextField) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::text_field(text_field))
	}

	fn shape(self, shape: Shape) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::shape(shape))
	}

	fn curve(self, curve: Curve) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::curve(curve))
	}

	fn image(self, image: Image) -> EvaluationContext<C> {
		self.parent.add_element(self.name, ConcreteElement::image(image))
	}

	fn component<F>(self, component: F)
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> UiFuture<'ctx> + 'static,
	{
		let runtime = Rc::clone(&self.parent.runtime);
		let tree = Rc::clone(&self.parent.tree);
		let task_id = Runtime::spawn_placeholder(Rc::clone(&runtime));
		let path = tree
			.borrow_mut()
			.scope_path(Some(self.parent.id), &self.parent.path, self.name);
		let ctx = EvaluationContext {
			id: self.parent.id,
			parent: Some(self.parent.id),
			path,
			ctx: Rc::clone(&self.parent.ctx),
			runtime: Rc::clone(&runtime),
			tree,
			task_id,
		};

		let ctx = Box::leak(Box::new(ctx));
		let future = component(ctx);
		Runtime::replace_task_future(runtime, task_id, future);
	}

	fn mount<F, T>(self, component: F) -> MountedComponentFuture<F, T, C>
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> MountedUiFuture<'ctx, T> + 'static,
	{
		MountedComponentFuture {
			component: Some(component),
			future: None,
			ctx: Rc::clone(&self.parent.ctx),
			runtime: Rc::clone(&self.parent.runtime),
			tree: Rc::clone(&self.parent.tree),
			parent: self.parent.id,
			parent_path: self.parent.path.clone(),
			name: self.name,
			task_id: self.parent.task_id,
			scope: None,
			complete: false,
			output: PhantomData,
		}
	}
}

impl<C: 'static> super::context::ContainerContext<C> for EvaluationContext<C> {
	fn on(&mut self, event: Events) -> EventFuture {
		EventFuture {
			runtime: Rc::clone(&self.runtime),
			task_id: self.task_id,
			target: self.id,
			kind: event,
			complete: false,
		}
	}

	fn on_key(&mut self, key: Key) -> KeyFuture {
		KeyFuture {
			runtime: Rc::clone(&self.runtime),
			task_id: self.task_id,
			target: self.id,
			key,
			complete: false,
		}
	}

	fn on_text_edit(&mut self) -> TextEditFuture {
		TextEditFuture {
			runtime: Rc::clone(&self.runtime),
			task_id: self.task_id,
			target: self.id,
			complete: false,
		}
	}
}

type BoxedMountedUiFuture<T> = Pin<Box<dyn Future<Output = T> + 'static>>;

pub struct MountedComponentFuture<F, T, C = ()> {
	component: Option<F>,
	future: Option<BoxedMountedUiFuture<T>>,
	ctx: Rc<C>,
	runtime: Rc<RefCell<Runtime>>,
	tree: Rc<RefCell<RetainedTree>>,
	parent: Id,
	parent_path: Vec<PathSegment>,
	name: &'static str,
	task_id: TaskId,
	scope: Option<Vec<PathSegment>>,
	complete: bool,
	output: PhantomData<T>,
}

impl<F, T, C> Unpin for MountedComponentFuture<F, T, C> {}

impl<F, T, C> MountedComponentFuture<F, T, C> {
	fn cleanup_scope(&mut self) {
		let Some(scope) = self.scope.take() else {
			return;
		};

		let removed = self.tree.borrow_mut().remove_scope(&scope);
		if !removed.is_empty() {
			self.runtime.borrow_mut().remove_targets(&removed);
		}
	}
}

impl<F, T, C> MountedComponentFuture<F, T, C>
where
	C: 'static,
	F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> MountedUiFuture<'ctx, T> + 'static,
{
	fn start(&mut self) {
		if self.future.is_some() {
			return;
		}

		let Some(component) = self.component.take() else {
			return;
		};

		let scope = self
			.tree
			.borrow_mut()
			.scope_path(Some(self.parent), &self.parent_path, self.name);
		let ctx = EvaluationContext {
			id: self.parent,
			parent: Some(self.parent),
			path: scope.clone(),
			ctx: Rc::clone(&self.ctx),
			runtime: Rc::clone(&self.runtime),
			tree: Rc::clone(&self.tree),
			task_id: self.task_id,
		};

		let ctx = Box::leak(Box::new(ctx));
		let future = component(ctx);
		self.scope = Some(scope);
		self.future = Some(future);
	}
}

impl<F, T, C> Future for MountedComponentFuture<F, T, C>
where
	C: 'static,
	F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> MountedUiFuture<'ctx, T> + 'static,
{
	type Output = T;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		self.start();

		let Some(future) = self.future.as_mut() else {
			return Poll::Pending;
		};

		match future.as_mut().poll(cx) {
			Poll::Ready(output) => {
				self.complete = true;
				self.future = None;
				self.cleanup_scope();
				Poll::Ready(output)
			}
			Poll::Pending => Poll::Pending,
		}
	}
}

impl<F, T, C> Drop for MountedComponentFuture<F, T, C> {
	fn drop(&mut self) {
		if !self.complete {
			self.cleanup_scope();
		}
	}
}

impl<F, T, C> FusedFuture for MountedComponentFuture<F, T, C>
where
	C: 'static,
	F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> MountedUiFuture<'ctx, T> + 'static,
{
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

pub struct RenderFuture {
	runtime: Rc<RefCell<Runtime>>,
	frame_seen: Option<u64>,
	complete: bool,
}

impl Future for RenderFuture {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		let current = self.runtime.borrow().frame;

		match self.frame_seen {
			None => {
				self.frame_seen = Some(current);
				self.runtime.borrow_mut().frame_waiters.push(cx.waker().clone());
				Poll::Pending
			}
			Some(seen) if seen < current => {
				self.complete = true;
				Poll::Ready(())
			}
			Some(_) => {
				self.runtime.borrow_mut().frame_waiters.push(cx.waker().clone());
				Poll::Pending
			}
		}
	}
}

impl FusedFuture for RenderFuture {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

pub struct EventFuture {
	runtime: Rc<RefCell<Runtime>>,
	task_id: TaskId,
	target: Id,
	kind: Events,
	complete: bool,
}

impl Future for EventFuture {
	type Output = UiEvent;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		let event = self.runtime.borrow_mut().take_event(self.task_id, self.target, self.kind);

		if let Some(event) = event {
			self.complete = true;
			return Poll::Ready(event);
		}

		self.runtime
			.borrow_mut()
			.wait_for_event(self.task_id, self.target, self.kind, cx.waker().clone());
		Poll::Pending
	}
}

impl FusedFuture for EventFuture {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

pub struct KeyFuture {
	runtime: Rc<RefCell<Runtime>>,
	task_id: TaskId,
	target: Id,
	key: Key,
	complete: bool,
}

impl Future for KeyFuture {
	type Output = UiKeyEvent;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		let event = self.runtime.borrow_mut().take_key_event(self.task_id, self.target, self.key);

		if let Some(event) = event {
			self.complete = true;
			return Poll::Ready(event);
		}

		self.runtime
			.borrow_mut()
			.wait_for_key(self.task_id, self.target, self.key, cx.waker().clone());
		Poll::Pending
	}
}

impl FusedFuture for KeyFuture {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

pub struct TextEditFuture {
	runtime: Rc<RefCell<Runtime>>,
	task_id: TaskId,
	target: Id,
	complete: bool,
}

impl Future for TextEditFuture {
	type Output = UiTextEditEvent;

	fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
		if self.complete {
			return Poll::Pending;
		}

		let event = self.runtime.borrow_mut().take_text_edit_event(self.task_id, self.target);

		if let Some(event) = event {
			self.complete = true;
			return Poll::Ready(event);
		}

		self.runtime
			.borrow_mut()
			.wait_for_text_edit(self.task_id, self.target, cx.waker().clone());
		Poll::Pending
	}
}

impl FusedFuture for TextEditFuture {
	fn is_terminated(&self) -> bool {
		self.complete
	}
}

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
	pub fn new() -> Self {
		Self::with_context(())
	}
}

impl<C: 'static> Engine<C> {
	pub fn with_context(ctx: C) -> Self {
		Self {
			viewports: Vec::new(),
			state: Rc::new(RefCell::new(EngineState::new())),
			cursor_position: Vector2::zero(),
			is_clicking: false,
			clicks: Vec::new(),
			scrolls: Vec::new(),
			key_states: HashMap::new(),
			key_presses: VecDeque::new(),
			text_edits: VecDeque::new(),
			text_system: TextSystem::new(),
			ctx: Rc::new(ctx),
			runtime: Rc::new(RefCell::new(Runtime::new())),
		}
	}

	pub fn ctx(&self) -> &C {
		self.ctx.as_ref()
	}

	pub(crate) fn add_viewport(&mut self, viewport: VirtualViewport) {
		self.viewports.push(viewport);
	}

	pub fn mount<F>(&mut self, root: F)
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext<C>) -> UiFuture<'ctx> + 'static,
	{
		let runtime = Rc::clone(&self.runtime);
		let tree = Rc::clone(&runtime.borrow().tree);
		let task_id = Runtime::spawn_placeholder(Rc::clone(&runtime));
		let ctx = EvaluationContext::new_root(Rc::clone(&self.ctx), Rc::clone(&runtime), tree, task_id);
		let ctx = Box::leak(Box::new(ctx));
		let future = root(ctx);
		Runtime::replace_task_future(runtime, task_id, future);
	}

	/// Evaluates mounted UI tasks and returns a snapshot of the resulting layout.
	pub fn evaluate<'a>(&mut self, size: Size, frame_allocator: &'a bumpalo::Bump) -> Snapshot<'a> {
		self.sync_pointer_state();
		Runtime::begin_frame(Rc::clone(&self.runtime));
		Runtime::poll_ready_tasks(Rc::clone(&self.runtime));

		let mut snapshot = self.build_snapshot_from_ui_tree(size, frame_allocator);
		self.route_input_events(&mut snapshot);
		self.route_key_input_events();
		self.route_text_edit_events();

		Runtime::poll_ready_tasks(Rc::clone(&self.runtime));
		snapshot
	}

	fn build_snapshot_from_ui_tree<'a>(&mut self, size: Size, frame_allocator: &'a bumpalo::Bump) -> Snapshot<'a> {
		let (mut elements, relations, clipped_elements) = {
			let tree = Rc::clone(&self.runtime.borrow().tree);
			let tree = tree.borrow();
			let mut relations = Vec::with_capacity_in(tree.relations.len(), frame_allocator);
			relations.extend_from_slice(&tree.relations);

			let mut elements = layout_elements(&tree.elements, &tree.relations, size, &mut self.text_system, frame_allocator);
			apply_visual_transforms(&mut elements, &tree, frame_allocator);
			let clipped_elements = clipped_layout_elements(&elements, &tree, frame_allocator);

			(elements, relations, clipped_elements)
		};

		{
			let mut state = self.state.borrow_mut();
			state.set_element_ids(elements.iter().map(|element| element.id));
		}

		let acceleration = build_mouse_click_acceleration(&clipped_elements, frame_allocator);
		self.runtime.borrow_mut().update_geometry(&elements);

		Snapshot {
			elements,
			relations,
			acceleration,
			cursor: self.state.borrow().cursor(),
			engine_state: Rc::clone(&self.state),
			size,
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
			if click {
				if let Some(target) = snapshot.click(self.cursor_position) {
					self.runtime.borrow_mut().push_event(UiEvent {
						target,
						kind: Events::Actuated,
						delta: None,
					});
				}
			}
		}

		while let Some(delta) = self.scrolls.pop() {
			if let Some(target) = snapshot.click(self.cursor_position) {
				self.route_scroll_event(target, delta);
			}
		}
	}

	fn route_scroll_event(&mut self, target: Id, delta: Vector2) {
		let runtime = Rc::clone(&self.runtime);
		let tree = Rc::clone(&runtime.borrow().tree);
		let tree = tree.borrow();
		let mut current = Some(target);

		while let Some(target) = current {
			runtime.borrow_mut().push_event(UiEvent {
				target,
				kind: Events::Scrolled,
				delta: Some(delta),
			});
			current = tree.parent_by_child.get(&target).copied();
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

	/// Renders the given snapshot into a [`Render`] object.
	pub fn render(&mut self, snapshot: &mut Snapshot<'_>) -> Render {
		let mut elements = Vec::new();
		let mut curve_elements = Vec::new();
		let mut image_elements = Vec::new();
		let mut text_elements = Vec::new();
		let mut effective_opacities = HashMap::new();
		let tree = Rc::clone(&self.runtime.borrow().tree);
		let tree = tree.borrow();
		let clips = element_clips(snapshot.elements.iter(), &tree);
		let feather_masks = element_feather_masks(snapshot.elements.iter(), &tree);

		for element in &mut snapshot.elements {
			let Some(retained_element) = tree.element(element.id) else {
				continue;
			};
			let clip = clips
				.get(&element.id)
				.map(|clip| clip.element)
				.unwrap_or(EffectiveClip::Unbounded);
			if clip.apply(geometry_from_layout_element(element)).is_none() {
				continue;
			}
			let clip = clip.as_rect();
			let feather_mask = feather_masks.get(&element.id).and_then(|mask| mask.element);

			let opacity = effective_opacity(element.id, &tree, &mut effective_opacities);
			let style = retained_element.element.primitive.style().clone();
			let color = style
				.layers()
				.first()
				.map(|layer| match &layer.color {
					Color::Value(rgba) => *rgba,
					Color::Sample(_) => RGBA::white(),
				})
				.unwrap_or_else(RGBA::white);

			match &retained_element.element.primitive {
				Primitives::Container(container) => elements.push(RenderElement {
					id: element.id.get(),
					position: element.position,
					size: element.size,
					clip,
					feather_mask,
					style,
					opacity,
					corner_radius: container.corner_radius,
					corner_exponent: container.corner_exponent,
				}),
				Primitives::Shape(shape) => {
					let (corner_radius, corner_exponent) = match shape.shape {
						Shapes::Box { radius, exponent, .. } => (radius, exponent),
						_ => (0.0, 2.0),
					};

					elements.push(RenderElement {
						id: element.id.get(),
						position: element.position,
						size: element.size,
						clip,
						feather_mask,
						style,
						opacity,
						corner_radius,
						corner_exponent,
					});
				}
				Primitives::Curve(curve) => curve_elements.push(RenderCurveElement {
					id: element.id.get(),
					position: element.position,
					size: element.size,
					clip,
					feather_mask,
					style,
					opacity,
					segments: curve.path().segments().to_vec(),
				}),
				Primitives::Image(image) => image_elements.push(RenderImageElement {
					id: element.id.get(),
					image_id: image.id(),
					version: image.version(),
					source_width: image.width_pixels(),
					source_height: image.height_pixels(),
					pixels: std::sync::Arc::from(image.pixels()),
					position: element.position,
					size: element.size,
					clip,
					feather_mask,
					opacity,
				}),
				Primitives::Text(text) => text_elements.push(RenderTextElement {
					id: element.id.get(),
					position: element.position,
					size: element.size,
					clip,
					feather_mask,
					color,
					opacity,
					font_size: text.settings().font_size,
					content: text.content().to_string(),
				}),
				Primitives::TextField(text_field) => text_elements.push(RenderTextElement {
					id: element.id.get(),
					position: element.position,
					size: element.size,
					clip,
					feather_mask,
					color,
					opacity,
					font_size: text_field.settings().font_size,
					content: text_field.content().to_string(),
				}),
			}
		}

		elements.sort_by_key(|element| element.position.z());
		curve_elements.sort_by_key(|element| element.position.z());
		image_elements.sort_by_key(|element| element.position.z());
		text_elements.sort_by_key(|element| element.position.z());

		Render {
			elements,
			curve_elements,
			image_elements,
			text_elements,
			relations: snapshot.relations.to_vec(),
		}
	}

	pub fn set_cursor_position(&mut self, v: Vector2) {
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

	pub fn update_scroll_state(&mut self, delta: Vector2) {
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

type BoxedUiFuture = Pin<Box<dyn Future<Output = ()> + 'static>>;

struct UiTask {
	future: Option<BoxedUiFuture>,
	inbox: VecDeque<UiEvent>,
	key_inbox: VecDeque<UiKeyEvent>,
	text_edit_inbox: VecDeque<UiTextEditEvent>,
	complete: bool,
}

type TaskId = usize;

struct EventWaiter {
	task_id: TaskId,
	target: Id,
	kind: Events,
	waker: Waker,
}

struct KeyWaiter {
	task_id: TaskId,
	target: Id,
	key: Key,
	waker: Waker,
}

struct TextEditWaiter {
	task_id: TaskId,
	target: Id,
	waker: Waker,
}

fn effective_opacity(id: Id, tree: &RetainedTree, effective_opacities: &mut HashMap<Id, f32>) -> f32 {
	if let Some(opacity) = effective_opacities.get(&id) {
		return *opacity;
	}

	let local_opacity = tree
		.element(id)
		.map(|element| sanitize_opacity(element.element.primitive.visual().opacity))
		.unwrap_or(1.0);
	let parent_opacity = tree
		.parent_by_child
		.get(&id)
		.copied()
		.map(|parent| effective_opacity(parent, tree, effective_opacities))
		.unwrap_or(1.0);
	let opacity = (parent_opacity * local_opacity).clamp(0.0, 1.0);
	effective_opacities.insert(id, opacity);
	opacity
}

fn sanitize_opacity(opacity: f32) -> f32 {
	if opacity.is_finite() {
		opacity.clamp(0.0, 1.0)
	} else {
		1.0
	}
}

pub struct Runtime {
	tasks: Vec<UiTask>,
	ready: Arc<Mutex<VecDeque<TaskId>>>,
	frame_waiters: Vec<Waker>,
	event_waiters: Vec<EventWaiter>,
	key_waiters: Vec<KeyWaiter>,
	text_edit_waiters: Vec<TextEditWaiter>,
	focus_stack: Vec<Id>,
	geometry: HashMap<Id, Geometry>,
	pointer: PointerState,
	frame: u64,
	tree: Rc<RefCell<RetainedTree>>,
}

struct TaskWaker {
	task: TaskId,
	ready: Arc<Mutex<VecDeque<TaskId>>>,
}

impl Wake for TaskWaker {
	fn wake(self: Arc<Self>) {
		self.ready.lock().push_back(self.task);
	}

	fn wake_by_ref(self: &Arc<Self>) {
		self.ready.lock().push_back(self.task);
	}
}

fn task_waker(task: TaskId, ready: Arc<Mutex<VecDeque<TaskId>>>) -> Waker {
	Waker::from(Arc::new(TaskWaker { task, ready }))
}

impl Runtime {
	fn new() -> Self {
		Self {
			tasks: Vec::new(),
			ready: Arc::new(Mutex::new(VecDeque::new())),
			frame_waiters: Vec::new(),
			event_waiters: Vec::new(),
			key_waiters: Vec::new(),
			text_edit_waiters: Vec::new(),
			focus_stack: Vec::new(),
			geometry: HashMap::new(),
			pointer: PointerState::default(),
			frame: 0,
			tree: Rc::new(RefCell::new(RetainedTree::new())),
		}
	}

	fn spawn_placeholder(runtime: Rc<RefCell<Self>>) -> TaskId {
		let mut runtime = runtime.borrow_mut();
		let id = runtime.tasks.len();
		runtime.tasks.push(UiTask {
			future: Some(Box::pin(async {})),
			inbox: VecDeque::new(),
			key_inbox: VecDeque::new(),
			text_edit_inbox: VecDeque::new(),
			complete: false,
		});
		runtime.ready.lock().push_back(id);
		id
	}

	fn replace_task_future(runtime: Rc<RefCell<Self>>, id: TaskId, future: UiFuture<'static>) {
		let mut runtime = runtime.borrow_mut();
		runtime.tasks[id].future = Some(future);
		runtime.tasks[id].complete = false;
		runtime.ready.lock().push_back(id);
	}

	fn begin_frame(runtime: Rc<RefCell<Self>>) {
		let mut runtime = runtime.borrow_mut();
		runtime.frame += 1;
		runtime.tree.borrow_mut().begin_frame();
		crate::ui::timer::wake_due_timers(std::time::Instant::now());

		for waker in runtime.frame_waiters.drain(..) {
			waker.wake();
		}
	}

	fn poll_ready_tasks(runtime: Rc<RefCell<Self>>) {
		loop {
			let (id, ready) = {
				let runtime = runtime.borrow();
				let Some(id) = runtime.ready.lock().pop_front() else {
					return;
				};
				(id, Arc::clone(&runtime.ready))
			};

			let mut future = {
				let mut runtime = runtime.borrow_mut();
				if runtime.tasks.get(id).map(|t| t.complete).unwrap_or(true) {
					continue;
				}

				let Some(future) = runtime.tasks[id].future.take() else {
					continue;
				};

				future
			};

			let waker = task_waker(id, ready);
			let mut cx = TaskContext::from_waker(&waker);
			let poll = future.as_mut().poll(&mut cx);

			let mut runtime = runtime.borrow_mut();
			if let Some(task) = runtime.tasks.get_mut(id) {
				match poll {
					Poll::Ready(()) => task.complete = true,
					Poll::Pending => task.future = Some(future),
				}
			}
		}
	}

	fn wait_for_event(&mut self, task_id: TaskId, target: Id, kind: Events, waker: Waker) {
		if let Some(waiter) = self
			.event_waiters
			.iter_mut()
			.find(|waiter| waiter.task_id == task_id && waiter.target == target && waiter.kind == kind)
		{
			waiter.waker = waker;
			return;
		}

		self.event_waiters.push(EventWaiter {
			task_id,
			target,
			kind,
			waker,
		});
	}

	fn wait_for_key(&mut self, task_id: TaskId, target: Id, key: Key, waker: Waker) {
		if let Some(waiter) = self
			.key_waiters
			.iter_mut()
			.find(|waiter| waiter.task_id == task_id && waiter.target == target && waiter.key == key)
		{
			waiter.waker = waker;
			return;
		}

		self.key_waiters.push(KeyWaiter {
			task_id,
			target,
			key,
			waker,
		});
	}

	fn wait_for_text_edit(&mut self, task_id: TaskId, target: Id, waker: Waker) {
		if let Some(waiter) = self
			.text_edit_waiters
			.iter_mut()
			.find(|waiter| waiter.task_id == task_id && waiter.target == target)
		{
			waiter.waker = waker;
			return;
		}

		self.text_edit_waiters.push(TextEditWaiter { task_id, target, waker });
	}

	fn push_event(&mut self, event: UiEvent) {
		let mut i = 0;
		while i < self.event_waiters.len() {
			let waiter = &self.event_waiters[i];

			if waiter.target == event.target && waiter.kind == event.kind {
				let waiter = self.event_waiters.swap_remove(i);

				if let Some(task) = self.tasks.get_mut(waiter.task_id) {
					task.inbox.push_back(event.clone());
				}

				waiter.waker.wake();
			} else {
				i += 1;
			}
		}
	}

	fn push_key_event(&mut self, event: UiKeyEvent) {
		let mut i = 0;
		while i < self.key_waiters.len() {
			let waiter = &self.key_waiters[i];

			if waiter.target == event.target && waiter.key == event.key {
				let waiter = self.key_waiters.swap_remove(i);

				if let Some(task) = self.tasks.get_mut(waiter.task_id) {
					task.key_inbox.push_back(event);
				}

				waiter.waker.wake();
			} else {
				i += 1;
			}
		}
	}

	fn push_text_edit_event(&mut self, event: UiTextEditEvent) {
		let mut i = 0;
		while i < self.text_edit_waiters.len() {
			let waiter = &self.text_edit_waiters[i];

			if waiter.target == event.target {
				let waiter = self.text_edit_waiters.swap_remove(i);

				if let Some(task) = self.tasks.get_mut(waiter.task_id) {
					task.text_edit_inbox.push_back(event);
				}

				waiter.waker.wake();
			} else {
				i += 1;
			}
		}
	}

	fn take_event(&mut self, task_id: TaskId, target: Id, kind: Events) -> Option<UiEvent> {
		let inbox = &mut self.tasks.get_mut(task_id)?.inbox;
		let index = inbox.iter().position(|e| e.target == target && e.kind == kind)?;
		inbox.remove(index)
	}

	fn take_key_event(&mut self, task_id: TaskId, target: Id, key: Key) -> Option<UiKeyEvent> {
		let inbox = &mut self.tasks.get_mut(task_id)?.key_inbox;
		let index = inbox.iter().position(|e| e.target == target && e.key == key)?;
		inbox.remove(index)
	}

	fn take_text_edit_event(&mut self, task_id: TaskId, target: Id) -> Option<UiTextEditEvent> {
		let inbox = &mut self.tasks.get_mut(task_id)?.text_edit_inbox;
		let index = inbox.iter().position(|e| e.target == target)?;
		inbox.remove(index)
	}

	fn request_focus(&mut self, target: Id) {
		self.focus_stack.retain(|focused| *focused != target);
		self.focus_stack.push(target);
	}

	fn release_focus(&mut self, target: Id) {
		self.focus_stack.retain(|focused| *focused != target);
	}

	fn focused_target(&mut self, is_valid: impl Fn(Id) -> bool) -> Option<Id> {
		self.focus_stack.retain(|focused| is_valid(*focused));
		self.focus_stack.last().copied()
	}

	fn update_geometry(&mut self, elements: &[LayoutElement]) {
		self.geometry.clear();
		self.geometry.extend(
			elements
				.iter()
				.map(|element| (element.id, Geometry::new(element.position, element.size))),
		);
	}

	fn remove_targets(&mut self, targets: &[Id]) {
		self.event_waiters
			.retain(|waiter| !targets.iter().any(|target| *target == waiter.target));
		self.key_waiters
			.retain(|waiter| !targets.iter().any(|target| *target == waiter.target));
		self.text_edit_waiters
			.retain(|waiter| !targets.iter().any(|target| *target == waiter.target));
		self.focus_stack
			.retain(|focused| !targets.iter().any(|target| *target == *focused));
		self.geometry.retain(|id, _| !targets.iter().any(|target| *target == *id));

		for task in &mut self.tasks {
			task.inbox
				.retain(|event| !targets.iter().any(|target| *target == event.target));
			task.key_inbox
				.retain(|event| !targets.iter().any(|target| *target == event.target));
			task.text_edit_inbox
				.retain(|event| !targets.iter().any(|target| *target == event.target));
		}
	}
}

/// The `Render` struct preserves the visual data derived from a snapshot so UI primitives can be submitted to the renderer.
#[derive(Clone)]
pub struct Render {
	elements: Vec<RenderElement>,
	curve_elements: Vec<RenderCurveElement>,
	image_elements: Vec<RenderImageElement>,
	text_elements: Vec<RenderTextElement>,
	relations: Vec<(Id, Id)>,
}

impl Render {
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

pub(crate) struct VirtualViewport;

impl ElementHandle for VirtualViewport {
	fn id(&self) -> Id {
		unimplemented!()
	}
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiEvent {
	pub target: Id,
	pub kind: Events,
	pub delta: Option<Vector2>,
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
		atomic::{AtomicUsize, Ordering},
		Arc, Mutex as StdMutex,
	};
	use std::time::Duration;

	use super::*;
	use crate::ui::{
		animate,
		components::{container::Container, curve::CurvePath, shape::Shape, text_field::TextField},
		flow::{self, Location3},
		layout::{
			context::{ContainerContext, Context, ElementContext},
			Geometry, Sizing,
		},
		primitive::TextEdit,
		spring,
		style::{ConcreteLayer, ConcreteStyle, EdgeFeather, Layer, LayerKind},
		Depth,
	};

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
		engine.set_cursor_position(Vector2::zero());
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

		engine.set_cursor_position(Vector2::new(0.25, -0.5));
		engine.update_click_state(true);
		engine.mount(move |ctx| {
			let observed = Arc::clone(&observed_for_task);
			Box::pin(async move {
				*observed.lock().unwrap() = Some(ctx.pointer());
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(
			*observed.lock().unwrap(),
			Some(PointerState {
				position: Vector2::new(0.25, -0.5),
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
				observed.lock().unwrap().push(ctx.pointer());
				ctx.render().await;
				observed.lock().unwrap().push(ctx.pointer());
			})
		});

		engine.set_cursor_position(Vector2::new(-1.0, -1.0));
		engine.update_click_state(false);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(Vector2::new(0.75, 0.5));
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(
			*observed.lock().unwrap(),
			vec![
				PointerState {
					position: Vector2::new(-1.0, -1.0),
					pressed: false,
				},
				PointerState {
					position: Vector2::new(0.75, 0.5),
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
				*received.lock().unwrap() = event.delta;
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.set_cursor_position(Vector2::zero());
		engine.update_scroll_state(Vector2::new(0.0, -1.0));
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*received.lock().unwrap(), Some(Vector2::new(0.0, -1.0)));
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
		assert_eq!(first.relations, vec![(first_ids[0], first_ids[1])]);

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
		let child = render.elements().find(|element| element.id == 3).unwrap();

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
		let child = render.elements().find(|element| element.id == 3).unwrap();

		assert_eq!(child.position, Location3::new(70, 0, 2));
		assert_eq!(child.clip, Some(Geometry::new(Location3::new(0, 0, 0), Size::new(100, 100))));
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
		engine.set_cursor_position(Vector2::new(0.5, 0.8));
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
		engine.set_cursor_position(Vector2::new(0.5, 0.8));
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

		let toast = render.elements().find(|element| element.position.x() == 70).unwrap();
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

		let toast = render.elements().find(|element| element.position.x() == 70).unwrap();
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
		engine.set_cursor_position(Vector2::new(0.5, 0.8));
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
				*geometry.lock().unwrap() = button.geometry();
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		assert_eq!(*geometry.lock().unwrap(), None);

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		assert_eq!(
			*geometry.lock().unwrap(),
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
				*geometry.lock().unwrap() = button.geometry();
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(
			*geometry.lock().unwrap(),
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
				seen.lock().unwrap().push(ctx.ctx().value);

				ctx.element("child")
					.component(move |ctx: &mut EvaluationContext<TestContext>| {
						let seen = Arc::clone(&seen);
						Box::pin(async move {
							seen.lock().unwrap().push(ctx.ctx().value + 1);
						})
					});
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*seen.lock().unwrap(), vec![7, 8]);
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
				*result.lock().unwrap() = Some(value);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*result.lock().unwrap(), Some(11));
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
				*result.lock().unwrap() = Some(value);
			})
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);
		assert_eq!(first.elements.len(), 2);
		assert_eq!(*result.lock().unwrap(), None);

		engine.set_cursor_position(Vector2::zero());
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*result.lock().unwrap(), Some(42));
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

		engine.set_cursor_position(Vector2::zero());
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
				*result.lock().unwrap() = Some(value);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.update_key_state(Key::Escape, true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*result.lock().unwrap(), Some(Result::Cancelled));
	}

	#[test]
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
								ids.lock().unwrap().push(button.id());
								button.on(Events::Actuated).await;
							})
						})
						.await;
				}
			})
		});

		let first = engine.evaluate(Size::new(100, 100), &frame_allocator);
		assert_eq!(first.elements.len(), 2);

		engine.set_cursor_position(Vector2::zero());
		engine.update_click_state(true);
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		let second = engine.evaluate(Size::new(100, 100), &frame_allocator);
		assert_eq!(second.elements.len(), 2);

		let ids = ids.lock().unwrap();
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

		let modal = render.elements().find(|element| element.position.x() == 30).unwrap();
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
				frame.element("high").container(Container::default().depth(10));
				frame.element("low").container(Container::default());
			})
		});

		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);
		let render = engine.render(&mut snapshot);
		let depths = render.elements().map(|element| element.position.z()).collect::<Vec<_>>();

		assert_eq!(depths, vec![0, 1, 10]);
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

		assert_eq!(render.elements[0].style.layers().len(), 1);
		assert_eq!(render.elements[0].style.layers()[0].kind(), LayerKind::Fill);
		match Layer::fill(&render.elements[0].style.layers()[0]) {
			Color::Value(color) => assert_eq!(*color, RGBA::new(0.2, 0.3, 0.4, 1.0)),
			Color::Sample(_) => panic!("expected value color"),
		}
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

		assert_eq!(render.elements[0].style.layers().len(), 2);
		assert_eq!(render.elements[0].style.layers()[0].kind(), LayerKind::Fill);
		assert_eq!(render.elements[0].style.layers()[1].kind(), LayerKind::Stroke { width: 2.0 });
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
		let parent = render.elements().find(|element| element.id == 1).unwrap();
		let child = render.elements().find(|element| element.id == 2).unwrap();
		let text = render.texts().find(|text| text.id == 3).unwrap();
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
		let child = render.elements().find(|element| element.id == 2).unwrap();

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
		let child = render.elements().find(|element| element.id == 2).unwrap();

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
		let child = render.elements().find(|element| element.id == 2).unwrap();
		let mask = child.feather_mask.unwrap();

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
		let parent = render.elements().find(|element| element.id == 1).unwrap();
		let child = render.elements().find(|element| element.id == 2).unwrap();

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
		let child = render.elements().find(|element| element.id == 2).unwrap();

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
		let text = render.texts().next().unwrap();

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

		assert_eq!(render.elements[0].opacity, 0.4);
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

		assert_eq!(render.elements().find(|element| element.id == 2).unwrap().opacity, 0.0);
		assert_eq!(render.elements().find(|element| element.id == 3).unwrap().opacity, 1.0);
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

		assert_eq!(render.elements[0].corner_radius, 8.0);
		assert_eq!(render.elements[0].corner_exponent, 4.0);
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

		assert_eq!(render.elements[0].corner_radius, 6.0);
		assert_eq!(render.elements[0].corner_exponent, 4.0);
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
		let frame = render.elements().next().unwrap();

		assert_eq!(frame.position, Location3::new(5, 9, 0));
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
		let child = render.elements().find(|element| element.id == 2).unwrap();

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
		engine.set_cursor_position(Vector2::new(0.0, 0.0));
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
		engine.set_cursor_position(Vector2::new(0.0, 0.0));
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
		match Layer::fill(&render.elements[0].style.layers()[0]) {
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

		assert_eq!(render.elements[0].opacity, 0.25);
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
		let text = render.texts().next().unwrap();

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
		let text = render.texts().next().unwrap();

		assert_eq!(text.content, "Hello");
		assert!(text.size.x() > 0);
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
				*received.lock().unwrap() = Some(event.edit);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.input_character('a');
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*received.lock().unwrap(), Some(TextEdit::Inserted('a')));
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
				*received.lock().unwrap() = Some(event.edit);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.input_character('a');
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*received.lock().unwrap(), None);
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
				*received.lock().unwrap() = Some(event.edit);
			})
		});

		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);
		engine.delete_text_backward();
		let _ = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert_eq!(*received.lock().unwrap(), Some(TextEdit::Deleted('é')));
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
				let initial = content.lock().unwrap().clone();
				let mut field = ctx.element("field").text_field(TextField::new(initial));
				field.request_focus();
				let event = field.on_text_edit().await;
				{
					let mut content = content.lock().unwrap();
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
		let text = render.texts().next().unwrap();

		assert_eq!(*content.lock().unwrap(), "ab");
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
		assert!(second.elements[0].size.x() > 10);
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
		engine.set_cursor_position(Vector2::zero());
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

use math::{Base as _, Vector2};
use utils::{r#async::FusedFuture, sync::Mutex, RGBA};

use super::{
	context::{Context, ElementContext, ElementSlot, MountedUiFuture, UiFuture},
	element::{ElementHandle, Id},
	flow::{Location3, Size},
	layout_elements,
	retained_tree::RetainedTree,
	snapshot::Snapshot,
	visual_transform::Affine2,
	ConcreteElement, FeatherMask, Geometry, IdedElement, LayoutElement, PathSegment, RenderCurveElement, RenderElement,
	RenderImageElement, RenderTextElement,
};
use crate::ui::{
	components::{curve::Curve, image::Image, shape::Shape, text_field::TextField},
	font::TextSystem,
	intersection::build_mouse_click_acceleration,
	primitive::{Events, Key, Primitive as _, Primitives, Shapes, TextEdit},
	style::{Color, EdgeFeather, Layer as _},
	Container, Depth, Text, Transform,
};
