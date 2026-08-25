//! Task scheduling, event delivery, focus, and retained runtime state.

use super::*;

type BoxedUiFuture = Pin<Box<dyn Future<Output = ()> + 'static>>;

pub(super) struct UiTask {
	pub(super) future: Option<BoxedUiFuture>,
	pub(super) inbox: VecDeque<UiEvent>,
	pub(super) key_inbox: VecDeque<UiKeyEvent>,
	pub(super) text_edit_inbox: VecDeque<UiTextEditEvent>,
	pub(super) complete: bool,
}

pub(super) type TaskId = usize;

pub(super) struct EventWaiter {
	pub(super) task_id: TaskId,
	pub(super) target: Id,
	pub(super) kind: Events,
	pub(super) waker: Waker,
}

pub(super) struct KeyWaiter {
	pub(super) task_id: TaskId,
	pub(super) target: Id,
	pub(super) key: Key,
	pub(super) waker: Waker,
}

pub(super) struct TextEditWaiter {
	pub(super) task_id: TaskId,
	pub(super) target: Id,
	pub(super) waker: Waker,
}

pub(super) fn effective_opacity(id: Id, tree: &RetainedTree, effective_opacities: &mut HashMap<Id, f32>) -> f32 {
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

pub(super) fn sanitize_opacity(opacity: f32) -> f32 {
	if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 }
}

pub struct Runtime {
	pub(super) tasks: Vec<UiTask>,
	pub(super) ready: Arc<Mutex<VecDeque<TaskId>>>,
	pub(super) frame_waiters: Vec<Waker>,
	pub(super) event_waiters: Vec<EventWaiter>,
	pub(super) key_waiters: Vec<KeyWaiter>,
	pub(super) text_edit_waiters: Vec<TextEditWaiter>,
	pub(super) focus_stack: Vec<Id>,
	pub(super) geometry: HashMap<Id, Geometry>,
	pub(super) pointer: PointerState,
	pub(super) frame: u64,
	pub(super) tree: Rc<RefCell<RetainedTree>>,
}

pub(super) struct TaskWaker {
	pub(super) task: TaskId,
	pub(super) ready: Arc<Mutex<VecDeque<TaskId>>>,
}

impl Wake for TaskWaker {
	fn wake(self: Arc<Self>) {
		self.ready.lock().push_back(self.task);
	}

	fn wake_by_ref(self: &Arc<Self>) {
		self.ready.lock().push_back(self.task);
	}
}

pub(super) fn task_waker(task: TaskId, ready: Arc<Mutex<VecDeque<TaskId>>>) -> Waker {
	Waker::from(Arc::new(TaskWaker { task, ready }))
}

impl Runtime {
	pub(super) fn new() -> Self {
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

	pub(super) fn spawn_placeholder(runtime: Rc<RefCell<Self>>) -> TaskId {
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

	pub(super) fn replace_task_future(runtime: Rc<RefCell<Self>>, id: TaskId, future: UiFuture<'static>) {
		let mut runtime = runtime.borrow_mut();
		runtime.tasks[id].future = Some(future);
		runtime.tasks[id].complete = false;
		runtime.ready.lock().push_back(id);
	}

	pub(super) fn begin_frame(runtime: Rc<RefCell<Self>>) {
		let mut runtime = runtime.borrow_mut();
		runtime.frame += 1;
		runtime.tree.borrow_mut().begin_frame();
		crate::ui::timer::wake_due_timers(std::time::Instant::now());

		for waker in runtime.frame_waiters.drain(..) {
			waker.wake();
		}
	}

	pub(super) fn poll_ready_tasks(runtime: Rc<RefCell<Self>>) {
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

	pub(super) fn wait_for_event(&mut self, task_id: TaskId, target: Id, kind: Events, waker: Waker) {
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

	pub(super) fn wait_for_key(&mut self, task_id: TaskId, target: Id, key: Key, waker: Waker) {
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

	pub(super) fn wait_for_text_edit(&mut self, task_id: TaskId, target: Id, waker: Waker) {
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

	pub(super) fn push_event(&mut self, event: UiEvent) {
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

	pub(super) fn push_key_event(&mut self, event: UiKeyEvent) {
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

	pub(super) fn push_text_edit_event(&mut self, event: UiTextEditEvent) {
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

	pub(super) fn take_event(&mut self, task_id: TaskId, target: Id, kind: Events) -> Option<UiEvent> {
		let inbox = &mut self.tasks.get_mut(task_id)?.inbox;
		let index = inbox.iter().position(|e| e.target == target && e.kind == kind)?;
		inbox.remove(index)
	}

	pub(super) fn take_key_event(&mut self, task_id: TaskId, target: Id, key: Key) -> Option<UiKeyEvent> {
		let inbox = &mut self.tasks.get_mut(task_id)?.key_inbox;
		let index = inbox.iter().position(|e| e.target == target && e.key == key)?;
		inbox.remove(index)
	}

	pub(super) fn take_text_edit_event(&mut self, task_id: TaskId, target: Id) -> Option<UiTextEditEvent> {
		let inbox = &mut self.tasks.get_mut(task_id)?.text_edit_inbox;
		let index = inbox.iter().position(|e| e.target == target)?;
		inbox.remove(index)
	}

	pub(super) fn request_focus(&mut self, target: Id) {
		self.focus_stack.retain(|focused| *focused != target);
		self.focus_stack.push(target);
	}

	pub(super) fn release_focus(&mut self, target: Id) {
		self.focus_stack.retain(|focused| *focused != target);
	}

	pub(super) fn focused_target(&mut self, is_valid: impl Fn(Id) -> bool) -> Option<Id> {
		self.focus_stack.retain(|focused| is_valid(*focused));
		self.focus_stack.last().copied()
	}

	pub(super) fn update_geometry(&mut self, elements: &[LayoutElement]) {
		self.geometry.clear();
		self.geometry.extend(
			elements
				.iter()
				.map(|element| (element.id, Geometry::new(element.position, element.size))),
		);
	}

	pub(super) fn remove_targets(&mut self, targets: &[Id]) {
		self.event_waiters.retain(|waiter| !targets.contains(&waiter.target));
		self.key_waiters.retain(|waiter| !targets.contains(&waiter.target));
		self.text_edit_waiters.retain(|waiter| !targets.contains(&waiter.target));
		self.focus_stack.retain(|focused| !targets.contains(focused));
		self.geometry.retain(|id, _| !targets.contains(id));

		for task in &mut self.tasks {
			task.inbox.retain(|event| !targets.contains(&event.target));
			task.key_inbox.retain(|event| !targets.contains(&event.target));
			task.text_edit_inbox.retain(|event| !targets.contains(&event.target));
		}
	}
}
