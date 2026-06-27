//! UI retained tree evaluation, interaction state, and render snapshots.

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
	flow::Size,
	layout_elements,
	snapshot::Snapshot,
	ConcreteElement, IdedElement, RenderElement, RenderTextElement,
};
use crate::ui::{
	components::shape::Shape,
	font::TextSystem,
	intersection::build_mouse_click_acceleration,
	primitive::{Events, Primitive as _, Primitives, Shapes},
	style::Color,
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
		let path = if path_string.is_empty() {
			Vec::new()
		} else {
			path_string.split('/').map(ToOwned::to_owned).collect()
		};

		self.elements.push(IdedElement {
			id,
			element,
			path: path.clone(),
		});

		if let Some(parent) = parent {
			if !self.relations.iter().any(|relation| *relation == (parent, id)) {
				self.relations.push((parent, id));
			}
		}

		(id, path)
	}

	fn element_mut(&mut self, id: Id) -> Option<&mut IdedElement> {
		let index = *self.element_indices.get(&id)?;
		self.elements.get_mut(index)
	}

	fn element(&self, id: Id) -> Option<&IdedElement> {
		let index = *self.element_indices.get(&id)?;
		self.elements.get(index)
	}

	fn remove_scope(&mut self, scope: &[String]) -> Vec<Id> {
		if scope.is_empty() {
			return Vec::new();
		}

		let mut removed = HashSet::new();
		self.elements.retain(|element| {
			let should_remove = element.path.starts_with(scope);
			if should_remove {
				removed.insert(element.id);
			}
			!should_remove
		});

		if removed.is_empty() {
			return Vec::new();
		}

		self.relations
			.retain(|(parent, child)| !removed.contains(parent) && !removed.contains(child));
		self.rebuild_element_indices();
		removed.into_iter().collect()
	}

	fn rebuild_element_indices(&mut self) {
		self.element_indices.clear();
		for (index, element) in self.elements.iter().enumerate() {
			self.element_indices.insert(element.id, index);
		}
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

	fn mount<F, T>(self, component: F) -> MountedComponentFuture<F, T>
	where
		F: for<'ctx> FnOnce(&'ctx mut EvaluationContext) -> MountedUiFuture<'ctx, T> + 'static,
	{
		MountedComponentFuture {
			component: Some(component),
			future: None,
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

type BoxedMountedUiFuture<T> = Pin<Box<dyn Future<Output = T> + 'static>>;

pub struct MountedComponentFuture<F, T> {
	component: Option<F>,
	future: Option<BoxedMountedUiFuture<T>>,
	runtime: Rc<RefCell<Runtime>>,
	tree: Rc<RefCell<RetainedTree>>,
	parent: Id,
	parent_path: Vec<String>,
	name: &'static str,
	task_id: TaskId,
	scope: Option<Vec<String>>,
	complete: bool,
	output: PhantomData<T>,
}

impl<F, T> Unpin for MountedComponentFuture<F, T> {}

impl<F, T> MountedComponentFuture<F, T> {
	fn cleanup_scope(&mut self) {
		let Some(scope) = self.scope.take() else {
			return;
		};

		let removed = self.tree.borrow_mut().remove_scope(&scope);
		if !removed.is_empty() {
			self.runtime.borrow_mut().remove_events_for_targets(&removed);
		}
	}
}

impl<F, T> MountedComponentFuture<F, T>
where
	F: for<'ctx> FnOnce(&'ctx mut EvaluationContext) -> MountedUiFuture<'ctx, T> + 'static,
{
	fn start(&mut self) {
		if self.future.is_some() {
			return;
		}

		let Some(component) = self.component.take() else {
			return;
		};

		let scope = self.tree.borrow_mut().scope_path(&self.parent_path, self.name);
		let ctx = EvaluationContext {
			id: self.parent,
			parent: Some(self.parent),
			path: scope.clone(),
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

impl<F, T> Future for MountedComponentFuture<F, T>
where
	F: for<'ctx> FnOnce(&'ctx mut EvaluationContext) -> MountedUiFuture<'ctx, T> + 'static,
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

impl<F, T> Drop for MountedComponentFuture<F, T> {
	fn drop(&mut self) {
		if !self.complete {
			self.cleanup_scope();
		}
	}
}

impl<F, T> FusedFuture for MountedComponentFuture<F, T>
where
	F: for<'ctx> FnOnce(&'ctx mut EvaluationContext) -> MountedUiFuture<'ctx, T> + 'static,
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
		let mut elements = Vec::new();
		let mut text_elements = Vec::new();
		let tree = Rc::clone(&self.runtime.borrow().tree);
		let tree = tree.borrow();

		for element in &mut snapshot.elements {
			let Some(retained_element) = tree.element(element.id) else {
				continue;
			};

			let layer = &retained_element.element.primitive.style().layer;
			let color = match &layer.color {
				Color::Value(rgba) => *rgba,
				Color::Sample(_) => RGBA::white(),
			};

			match &retained_element.element.primitive {
				Primitives::Container(container) => elements.push(RenderElement {
					id: element.id.get(),
					position: element.position,
					size: element.size,
					color,
					corner_radius: container.corner_radius,
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

		elements.sort_by_key(|element| element.position.z());
		text_elements.sort_by_key(|element| element.position.z());

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

	fn remove_events_for_targets(&mut self, targets: &[Id]) {
		self.event_waiters
			.retain(|waiter| !targets.iter().any(|target| *target == waiter.target));

		for task in &mut self.tasks {
			task.inbox
				.retain(|event| !targets.iter().any(|target| *target == event.target));
		}
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
		Arc, Mutex as StdMutex,
	};
	use std::time::Duration;

	use super::*;
	use crate::ui::{
		components::container::Container,
		flow::{self, Location3},
		layout::context::{ContainerContext, Context, ElementContext},
		style::ConcreteLayer,
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
	fn empty_retained_tree_does_not_panic() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();
		let mut snapshot = engine.evaluate(Size::new(100, 100), &frame_allocator);

		assert!(snapshot.elements.is_empty());
		assert_eq!(engine.render(&mut snapshot).size(), 0);
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
	fn render_orders_elements_by_resolved_depth() {
		let frame_allocator = bumpalo::Bump::new();
		let mut engine = Engine::new();

		engine.mount(|ctx| {
			Box::pin(async move {
				let mut frame = ctx.element("frame").container(Container::default());
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

		assert_eq!(render.elements[0].color, RGBA::new(0.2, 0.3, 0.4, 1.0));
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
