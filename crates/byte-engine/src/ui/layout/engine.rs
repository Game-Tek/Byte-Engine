//! UI retained tree evaluation, interaction state, and render snapshots.

use std::{
	boxed::Box,
	cell::RefCell,
	collections::{HashMap, HashSet, VecDeque},
	future::Future,
	pin::Pin,
	rc::Rc,
	sync::Arc,
	task::{Context as TaskContext, Poll, Wake, Waker},
};

use math::{Base as _, Vector2};
use utils::{r#async::FusedFuture, sync::Mutex, RGBA};

use super::{
	context::{Context, ElementContext, ElementSlot, UiFuture},
	element::{ElementHandle, Id},
	flow::Size,
	layout_elements,
	snapshot::Snapshot,
	ConcreteElement, IdedElement, RenderElement, RenderTextElement,
};
use crate::ui::{
	components::shape::Shape,
	flow::Location,
	font::TextSystem,
	intersection::{build_mouse_click_acceleration, MouseClickAcceleration},
	primitive::{Events, Primitives, Shapes},
	style::{self, Color},
	Container, Text,
};

/// The [`Engine`] struct owns UI evaluation state, text shaping, and pointer
/// interaction across viewports.
pub struct Engine {
	viewports: Vec<VirtualViewport>,
	state: Rc<RefCell<EngineState>>,
	cursor_position: Vector2,
	is_clicking: bool,
	clicks: Vec<bool>,
	text_system: TextSystem,
	runtime: Rc<RefCell<Runtime>>,
}

pub(super) struct EngineState {
	element_ids: HashSet<Id>,
	cursor: Option<Id>,
}

#[derive(Default)]
struct RetainedTree {
	elements: Vec<IdedElement>,
	element_indices: HashMap<Id, usize>,
	relations: Vec<(Id, Id)>,
	path_counts: HashMap<String, u32>,
	path_ids: HashMap<String, Id>,
	next_id: u32,
}

impl RetainedTree {
	fn begin_frame(&mut self) {
		self.path_counts.clear();
	}

	fn element_path(&mut self, parent_path: &[String], name: &'static str) -> String {
		let parent = parent_path.join("/");
		let key = if parent.is_empty() {
			name.to_string()
		} else {
			format!("{parent}/{name}")
		};
		let count = self.path_counts.entry(key).or_insert(0);
		*count += 1;

		let mut path = parent_path.to_vec();
		path.push(format!("{name}#{count}"));
		path.join("/")
	}

	fn scope_path(&mut self, parent_path: &[String], name: &'static str) -> Vec<String> {
		self.element_path(parent_path, name)
			.split('/')
			.map(ToOwned::to_owned)
			.collect()
	}

	fn id_for_path(&mut self, path: String) -> Id {
		if let Some(id) = self.path_ids.get(&path) {
			return *id;
		}

		let id = Id::new(self.next_id).expect("UI id counter must stay non-zero");
		self.next_id += 1;
		self.path_ids.insert(path, id);
		id
	}

	fn add_element(
		&mut self,
		parent: Option<Id>,
		parent_path: &[String],
		name: &'static str,
		element: ConcreteElement,
	) -> (Id, Vec<String>) {
		let path_string = self.element_path(parent_path, name);
		let id = self.id_for_path(path_string.clone());

		if self.element_indices.contains_key(&id) {
			return (id, path_string.split('/').map(ToOwned::to_owned).collect());
		}

		self.element_indices.insert(id, self.elements.len());
		self.elements.push(IdedElement { id, element });

		if let Some(parent) = parent {
			if !self.relations.iter().any(|relation| *relation == (parent, id)) {
				self.relations.push((parent, id));
			}
		}

		let path = if path_string.is_empty() {
			Vec::new()
		} else {
			path_string.split('/').map(ToOwned::to_owned).collect()
		};

		(id, path)
	}

	fn element_mut(&mut self, id: Id) -> Option<&mut IdedElement> {
		let index = *self.element_indices.get(&id)?;
		self.elements.get_mut(index)
	}
}

/// Context owned by a mounted async UI task.
pub struct EvaluationContext {
	id: Id,
	parent: Option<Id>,
	path: Vec<String>,
	runtime: Rc<RefCell<Runtime>>,
	tree: Rc<RefCell<RetainedTree>>,
	task_id: TaskId,
}

impl EvaluationContext {
	fn new_root(runtime: Rc<RefCell<Runtime>>, tree: Rc<RefCell<RetainedTree>>, task_id: TaskId) -> Self {
		Self {
			id: Id::new(1).unwrap(),
			parent: None,
			path: Vec::new(),
			runtime,
			tree,
			task_id,
		}
	}

	fn new_child(
		runtime: Rc<RefCell<Runtime>>,
		tree: Rc<RefCell<RetainedTree>>,
		task_id: TaskId,
		id: Id,
		path: Vec<String>,
	) -> Self {
		Self {
			id,
			parent: Some(id),
			path,
			runtime,
			tree,
			task_id,
		}
	}

	fn add_element(&mut self, name: &'static str, element: ConcreteElement) -> EvaluationContext {
		let (id, path) = self.tree.borrow_mut().add_element(self.parent, &self.path, name, element);
		EvaluationContext::new_child(Rc::clone(&self.runtime), Rc::clone(&self.tree), self.task_id, id, path)
	}
}

impl Context for EvaluationContext {
	fn id(&self) -> Id {
		self.id
	}

	fn element<'a>(&'a mut self, name: &'static str) -> ElementSlot<'a> {
		ElementSlot { parent: self, name }
	}

	fn render(&mut self) -> RenderFuture {
		RenderFuture {
			runtime: Rc::clone(&self.runtime),
			frame_seen: None,
			complete: false,
		}
	}
}

impl ElementContext for ElementSlot<'_> {
	fn container(self, element: Container) -> EvaluationContext {
		self.parent.add_element(self.name, ConcreteElement::container(element))
	}

	fn text(self, text: Text) -> EvaluationContext {
		self.parent.add_element(self.name, ConcreteElement::text(text))
	}

	fn shape(self, shape: Shape) -> EvaluationContext {
		self.parent.add_element(self.name, ConcreteElement::shape(shape))
	}

	fn component<F>(self, component: F)
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext) -> UiFuture<'ctx> + 'static,
	{
		let runtime = Rc::clone(&self.parent.runtime);
		let tree = Rc::clone(&self.parent.tree);
		let task_id = Runtime::spawn_placeholder(Rc::clone(&runtime));
		let path = tree.borrow_mut().scope_path(&self.parent.path, self.name);
		let ctx = EvaluationContext {
			id: self.parent.id,
			parent: Some(self.parent.id),
			path,
			runtime: Rc::clone(&runtime),
			tree,
			task_id,
		};

		let ctx = Box::leak(Box::new(ctx));
		let future = component(ctx);
		Runtime::replace_task_future(runtime, task_id, future);
	}
}

impl super::context::ContainerContext for EvaluationContext {
	fn on(&mut self, event: Events) -> EventFuture {
		EventFuture {
			runtime: Rc::clone(&self.runtime),
			task_id: self.task_id,
			target: self.id,
			kind: event,
			complete: false,
		}
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

	pub(super) fn set_cursor(&mut self, cursor: Option<Id>) -> Option<Id> {
		self.cursor = cursor.filter(|id| self.element_ids.contains(id));
		self.cursor
	}

	fn cursor(&self) -> Option<Id> {
		self.cursor
	}
}

impl Default for Engine {
	fn default() -> Self {
		Self::new()
	}
}

impl Engine {
	pub fn new() -> Self {
		Self {
			viewports: Vec::new(),
			state: Rc::new(RefCell::new(EngineState::new())),
			cursor_position: Vector2::zero(),
			is_clicking: false,
			clicks: Vec::new(),
			text_system: TextSystem::new(),
			runtime: Rc::new(RefCell::new(Runtime::new())),
		}
	}

	pub(crate) fn add_viewport(&mut self, viewport: VirtualViewport) {
		self.viewports.push(viewport);
	}

	pub fn mount<F>(&mut self, root: F)
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext) -> UiFuture<'ctx> + 'static,
	{
		let runtime = Rc::clone(&self.runtime);
		let tree = Rc::clone(&runtime.borrow().tree);
		let task_id = Runtime::spawn_placeholder(Rc::clone(&runtime));
		let ctx = EvaluationContext::new_root(Rc::clone(&runtime), tree, task_id);
		let ctx = Box::leak(Box::new(ctx));
		let future = root(ctx);
		Runtime::replace_task_future(runtime, task_id, future);
	}

	/// Evaluates mounted UI tasks and returns a snapshot of the resulting layout.
	pub fn evaluate<'a>(&mut self, size: Size, frame_allocator: &'a bumpalo::Bump) -> Snapshot<'a> {
		Runtime::begin_frame(Rc::clone(&self.runtime));
		Runtime::poll_ready_tasks(Rc::clone(&self.runtime));

		let mut snapshot = self.build_snapshot_from_ui_tree(size, frame_allocator);
		self.route_input_events(&mut snapshot);

		Runtime::poll_ready_tasks(Rc::clone(&self.runtime));
		snapshot
	}

	fn build_snapshot_from_ui_tree<'a>(&mut self, size: Size, frame_allocator: &'a bumpalo::Bump) -> Snapshot<'a> {
		let (elements, relations) = {
			let tree = Rc::clone(&self.runtime.borrow().tree);
			let tree = tree.borrow();
			let mut relations = Vec::with_capacity_in(tree.relations.len(), frame_allocator);
			relations.extend_from_slice(&tree.relations);

			(
				layout_elements(&tree.elements, &tree.relations, size, &mut self.text_system, frame_allocator),
				relations,
			)
		};

		{
			let mut state = self.state.borrow_mut();
			state.set_element_ids(elements.iter().map(|element| element.id));
		}

		let acceleration = build_mouse_click_acceleration(&elements, frame_allocator);

		Snapshot {
			elements,
			relations,
			acceleration,
			cursor: self.state.borrow().cursor(),
			engine_state: Rc::clone(&self.state),
			size,
		}
	}

	fn route_input_events(&mut self, snapshot: &mut Snapshot<'_>) {
		while let Some(click) = self.clicks.pop() {
			if click {
				if let Some(target) = snapshot.click(self.cursor_position) {
					self.runtime.borrow_mut().push_event(UiEvent {
						target,
						kind: Events::Actuated,
					});
				}
			}
		}
	}

	/// Renders the given snapshot into a [`Render`] object.
	pub fn render(&mut self, snapshot: &mut Snapshot<'_>) -> Render {
		let size = snapshot.size();

		let mouse_pos = (self.cursor_position + 1.0) * 0.5;
		let mouse_pos = mouse_pos * Vector2::new(size.x() as f32, size.y() as f32);
		let mouse_pos = Vector2::new(mouse_pos.x, size.y() as f32 - mouse_pos.y);

		struct StyleContextImpl<'a> {
			acceleration: &'a MouseClickAcceleration<'a>,
			cursor: Option<Id>,
			self_id: Id,
			mouse_pos: Location,
		}

		impl style::ContextStyle for StyleContextImpl<'_> {
			fn id(&self) -> Id {
				self.self_id
			}

			fn is_hovered(&self, id: Id) -> bool {
				self.acceleration
					.query(self.mouse_pos)
					.map(|e| e == id.get())
					.unwrap_or(false)
			}

			fn is_focused(&self, id: Id) -> bool {
				self.cursor.map(|e| e == id).unwrap_or(false)
			}
		}

		let mut elements = Vec::new();
		let mut text_elements = Vec::new();
		let tree = Rc::clone(&self.runtime.borrow().tree);
		let mut tree = tree.borrow_mut();

		for element in &mut snapshot.elements {
			let state = StyleContextImpl {
				acceleration: &snapshot.acceleration,
				cursor: snapshot.cursor,
				self_id: element.id,
				mouse_pos: Location::new(mouse_pos.x as u32, mouse_pos.y as u32),
			};

			let Some(retained_element) = tree.element_mut(element.id) else {
				continue;
			};

			let style = match &mut retained_element.element.primitive {
				Primitives::Container(container) => container.styler.as_mut().map(|styler| styler(&state)).unwrap_or_default(),
				Primitives::Shape(shape) => shape.styler.as_mut().map(|styler| styler(&state)).unwrap_or_default(),
				Primitives::Text(text) => text.styler.as_mut().map(|styler| styler(&state)).unwrap_or_default(),
			};

			let layer = &style.layers[0];
			let color = match layer.color {
				Color::Value(rgba) => rgba,
				Color::Sample(_) => RGBA::white(),
			};

			match &retained_element.element.primitive {
				Primitives::Container(container) => elements.push(RenderElement {
					id: element.id.get(),
					position: element.position,
					size: element.size,
					color,
					corner_radius: container.settings.corner_radius,
				}),
				Primitives::Shape(shape) => {
					let corner_radius = match shape.shape {
						Shapes::Box { radius, .. } => radius,
						_ => 0.0,
					};

					elements.push(RenderElement {
						id: element.id.get(),
						position: element.position,
						size: element.size,
						color,
						corner_radius,
					});
				}
				Primitives::Text(text) => text_elements.push(RenderTextElement {
					id: element.id.get(),
					position: element.position,
					size: element.size,
					color,
					font_size: text.settings().font_size,
					content: text.content().to_string(),
				}),
			}
		}

		Render {
			elements,
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
}

type BoxedUiFuture = Pin<Box<dyn Future<Output = ()> + 'static>>;

struct UiTask {
	future: Option<BoxedUiFuture>,
	inbox: VecDeque<UiEvent>,
	complete: bool,
}

type TaskId = usize;

struct EventWaiter {
	task_id: TaskId,
	target: Id,
	kind: Events,
	waker: Waker,
}

pub struct Runtime {
	tasks: Vec<UiTask>,
	ready: Arc<Mutex<VecDeque<TaskId>>>,
	frame_waiters: Vec<Waker>,
	event_waiters: Vec<EventWaiter>,
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
			frame: 0,
			tree: Rc::new(RefCell::new(RetainedTree {
				next_id: 1,
				..RetainedTree::default()
			})),
		}
	}

	fn spawn_placeholder(runtime: Rc<RefCell<Self>>) -> TaskId {
		let mut runtime = runtime.borrow_mut();
		let id = runtime.tasks.len();
		runtime.tasks.push(UiTask {
			future: Some(Box::pin(async {})),
			inbox: VecDeque::new(),
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

	fn take_event(&mut self, task_id: TaskId, target: Id, kind: Events) -> Option<UiEvent> {
		let inbox = &mut self.tasks.get_mut(task_id)?.inbox;
		let index = inbox.iter().position(|e| e.target == target && e.kind == kind)?;
		inbox.remove(index)
	}
}

/// The `Render` struct preserves the visual data derived from a snapshot so UI primitives can be submitted to the renderer.
#[derive(Clone)]
pub struct Render {
	elements: Vec<RenderElement>,
	text_elements: Vec<RenderTextElement>,
	relations: Vec<(Id, Id)>,
}

impl Render {
	pub(crate) fn root(&self) -> &RenderElement {
		self.elements.iter().find(|e| e.id == 1).unwrap()
	}

	pub(crate) fn size(&self) -> usize {
		self.elements.len() + self.text_elements.len()
	}

	pub(crate) fn elements(&self) -> impl Iterator<Item = &RenderElement> {
		self.elements.iter()
	}

	pub(crate) fn texts(&self) -> impl Iterator<Item = &RenderTextElement> {
		self.text_elements.iter()
	}
}

pub(crate) struct VirtualViewport;

impl ElementHandle for VirtualViewport {
	fn id(&self) -> Id {
		unimplemented!()
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiEvent {
	pub target: Id,
	pub kind: Events,
}

#[cfg(test)]
mod tests {
	use std::sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	};

	use super::*;
	use crate::ui::{
		components::container::ContainerSettings,
		flow,
		layout::context::{ContainerContext, ElementContext},
	};

	#[test]
	fn mounted_task_retains_markup_without_render_loop() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				ctx.element("root")
					.container(Container::new(ContainerSettings::default().flow(flow::column)));
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
				let mut button = ctx.element("button").container(Container::new(ContainerSettings::default()));
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
	fn nested_retained_components_attach_under_declaring_element_with_stable_ids() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx
					.element("frame")
					.container(Container::new(ContainerSettings::default().flow(flow::column)));
				frame.element("child").component(|ctx| {
					Box::pin(async move {
						ctx.element("button")
							.container(Container::new(ContainerSettings::default().size(20.into())));
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
	fn empty_retained_tree_does_not_panic() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert!(snapshot.elements.is_empty());
		assert_eq!(engine.render(&mut snapshot).size(), 0);
	}
}
