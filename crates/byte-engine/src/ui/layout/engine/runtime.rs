//! Task scheduling, event delivery, focus, and retained runtime state.

use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

type BoxedUiFuture = Pin<Box<dyn Future<Output = ()> + 'static>>;

/// The `UiTask` struct keeps one spawned component alive until its future completes or its owning scope is removed.
pub(super) struct UiTask {
	/// Empty while the task is being polled or before its future is started.
	pub(super) future: Option<BoxedUiFuture>,
	/// Reused across polls; its queued flag coalesces concurrent wake requests.
	pub(super) waker: Option<Arc<TaskWaker>>,
	pub(super) owner: ScopeId,
	/// The structural path the task was declared under, so removing an element ends it.
	pub(super) path: usize,
	pub(super) inbox: VecDeque<UiEvent>,
	pub(super) key_inbox: VecDeque<UiKeyEvent>,
	pub(super) text_edit_inbox: VecDeque<UiTextEditEvent>,
	/// The frame a dropped frame wait was counting from, for the wait that replaces it in the same poll.
	///
	/// A frame wait that loses a selection is dropped while the frame it waited for may already
	/// have begun. Its replacement continues from here instead of waiting for one more frame.
	pub(super) carried_frame: Option<u64>,
}

/// Task slots are reused, so a handle from a removed task finds nothing instead of its successor.
pub(super) type TaskId = StableVecHandle;

/// The `ScopeId` struct identifies one mounted scope so the tasks spawned inside it end with it.
///
/// Scope paths can repeat between live mounts, so ownership uses this identity instead,
/// which an engine never hands out twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScopeId(u64);

impl ScopeId {
	/// The scope of the root component, which is never removed.
	pub(super) const ROOT: Self = Self(0);
}

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

pub(super) fn sanitize_opacity(opacity: f32) -> f32 {
	if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 }
}

pub struct Runtime {
	pub(super) tasks: StableVec<UiTask>,
	next_scope: u64,
	pub(super) ready: Arc<Mutex<VecDeque<TaskId>>>,
	/// Runs the tick that polls woken tasks; see [`super::Engine::set_waker`].
	pub(super) host: Arc<Mutex<Option<Waker>>>,
	pub(super) frame_waiters: StableVec<Option<Waker>>,
	pub(super) event_waiters: Vec<EventWaiter>,
	pub(super) key_waiters: Vec<KeyWaiter>,
	pub(super) text_edit_waiters: Vec<TextEditWaiter>,
	pub(super) focus_stack: Vec<Id>,
	pub(super) geometry: HashMap<Id, Geometry>,
	pub(super) pointer: PointerState,
	pub(super) drag: Drag,
	pub(super) frame: u64,
	pub(super) tree: Rc<RefCell<RetainedTree>>,
}

/// The `TaskWaker` struct keeps a live task scheduled at most once between polls.
pub(super) struct TaskWaker {
	pub(super) task: TaskId,
	queued: AtomicBool,
	pub(super) ready: Arc<Mutex<VecDeque<TaskId>>>,
	/// Runs the tick that polls the task when it is woken from outside the engine's own evaluation.
	pub(super) host: Arc<Mutex<Option<Waker>>>,
}

impl Wake for TaskWaker {
	fn wake(self: Arc<Self>) {
		self.wake_by_ref();
	}

	fn wake_by_ref(self: &Arc<Self>) {
		if !self.queued.swap(true, Ordering::AcqRel) {
			self.ready.lock().push_back(self.task);
			// A wake from another thread must reach the host so the task is polled in its next tick.
			if let Some(host) = self.host.lock().as_ref() {
				host.wake_by_ref();
			}
		}
	}
}

impl Runtime {
	pub(super) fn new() -> Self {
		// Most screens start a handful of components together. Reserve their scheduler storage once.
		const TASK_CAPACITY: usize = 16;
		Self {
			tasks: StableVec::with_capacity(TASK_CAPACITY),
			next_scope: ScopeId::ROOT.0,
			ready: Arc::new(Mutex::new(VecDeque::with_capacity(TASK_CAPACITY))),
			host: Arc::new(Mutex::new(None)),
			frame_waiters: StableVec::with_capacity(TASK_CAPACITY),
			event_waiters: Vec::with_capacity(TASK_CAPACITY),
			key_waiters: Vec::new(),
			text_edit_waiters: Vec::new(),
			focus_stack: Vec::new(),
			geometry: HashMap::new(),
			pointer: PointerState::default(),
			drag: Drag::new(DRAG_THRESHOLD),
			frame: 0,
			tree: Rc::new(RefCell::new(RetainedTree::new())),
		}
	}

	/// Reports whether a component waits for the next frame or to be polled.
	pub(super) fn needs_tick(&self) -> bool {
		self.frame_waiters.iter().any(Option::is_some) || !self.ready.lock().is_empty()
	}

	/// Returns an identity for a newly mounted scope.
	pub(super) fn next_scope(&mut self) -> ScopeId {
		self.next_scope += 1;
		ScopeId(self.next_scope)
	}

	/// Reserves a task owned by `owner` and declared at `path` so its context can name it before the future exists.
	///
	/// Next, call [`Self::start_task`] with the future built from that context.
	pub(super) fn reserve_task(&mut self, owner: ScopeId, path: usize) -> TaskId {
		self.tasks.push(UiTask {
			future: None,
			waker: None,
			owner,
			path,
			inbox: VecDeque::new(),
			key_inbox: VecDeque::new(),
			text_edit_inbox: VecDeque::new(),
			carried_frame: None,
		})
	}

	/// Starts a reserved task with one reusable, coalescing waker.
	pub(super) fn start_task(&mut self, id: TaskId, future: UiFuture<'static>) {
		let waker = Arc::new(TaskWaker {
			task: id,
			ready: Arc::clone(&self.ready),
			host: Arc::clone(&self.host),
			queued: AtomicBool::new(false),
		});
		let task = self.tasks.get_mut(id).expect(
			"A UI task was started after it was removed. The most likely cause is starting a task outside the call that reserved it.",
		);
		task.future = Some(future);
		task.waker = Some(Arc::clone(&waker));
		waker.wake_by_ref();
	}

	/// Ends every task owned by a removed scope.
	///
	/// Futures are detached while the runtime is borrowed and dropped after the borrow
	/// ends, because a dropped task's own mounted scopes end through this runtime again.
	pub(super) fn end_scope(runtime: &Rc<RefCell<Self>>, owner: ScopeId) {
		let detached = runtime.borrow_mut().detach_tasks(|task| task.owner == owner);
		drop(detached);
	}

	/// Removes every task the predicate selects and hands their futures to the caller.
	///
	/// Drop the returned tasks after releasing the runtime borrow: a dropped future may
	/// own mounted scopes that end through this runtime again.
	pub(super) fn detach_tasks(&mut self, select: impl Fn(&UiTask) -> bool) -> Vec<UiTask> {
		let selected = self
			.tasks
			.handled_iter()
			.filter(|(_, task)| select(task))
			.map(|(id, _)| id)
			.collect::<Vec<_>>();
		if selected.is_empty() {
			return Vec::new();
		}
		let detached = selected
			.into_iter()
			.filter_map(|id| self.tasks.remove(id))
			.collect::<Vec<_>>();
		self.forget_removed_tasks();
		detached
	}

	/// Drops waiters left by removed tasks so no queue grows with tasks that no longer exist.
	fn forget_removed_tasks(&mut self) {
		let Self {
			tasks,
			event_waiters,
			key_waiters,
			text_edit_waiters,
			..
		} = self;
		event_waiters.retain(|waiter| tasks.contains_handle(waiter.task_id));
		key_waiters.retain(|waiter| tasks.contains_handle(waiter.task_id));
		text_edit_waiters.retain(|waiter| tasks.contains_handle(waiter.task_id));
	}

	pub(super) fn begin_frame(runtime: Rc<RefCell<Self>>) {
		let mut runtime = runtime.borrow_mut();
		runtime.frame += 1;
		runtime.tree.borrow_mut().begin_frame();
		crate::ui::timer::wake_due_timers(std::time::Instant::now());

		for waker in runtime.frame_waiters.iter_mut().filter_map(Option::take) {
			waker.wake();
		}
	}

	/// Polls ready tasks outside the runtime borrow so components can update their tree.
	pub(super) fn poll_ready_tasks(runtime: Rc<RefCell<Self>>) {
		loop {
			let (id, waker, mut future) = {
				let mut runtime = runtime.borrow_mut();
				let Some(id) = runtime.ready.lock().pop_front() else {
					return;
				};
				// Generational handles reject wakes left by removed tasks.
				let Some(task) = runtime.tasks.get_mut(id) else { continue };
				let Some(future) = task.future.take() else { continue };
				let waker = task
					.waker
					.as_ref()
					.expect("A running UI task has no waker. The task was not started by its runtime.");
				// Acquire preceding wakes, then clear before polling so a new wake schedules another poll.
				waker.queued.swap(false, Ordering::AcqRel);
				(id, Waker::from(Arc::clone(waker)), future)
			};

			let mut cx = TaskContext::from_waker(&waker);
			let poll = future.as_mut().poll(&mut cx);

			// A finished future, or one whose task was removed while it ran, is dropped
			// outside the borrow because dropping it may end mounted scopes.
			let finished = match poll {
				Poll::Ready(()) => {
					drop(future);
					let mut runtime = runtime.borrow_mut();
					runtime.tasks.remove(id);
					runtime.forget_removed_tasks();
					None
				}
				Poll::Pending => match runtime.borrow_mut().tasks.get_mut(id) {
					Some(task) => {
						// A task that ends its poll without waiting for a frame again starts its next wait fresh,
						// so a wait after a long idle still sees the layout of the frame that follows it.
						task.carried_frame = None;
						task.future = Some(future);
						None
					}
					None => Some(future),
				},
			};
			drop(finished);
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

	/// Removes input and geometry owned by the deleted elements.
	pub(super) fn remove_targets(&mut self, targets: &HashSet<Id>) {
		self.event_waiters.retain(|waiter| !targets.contains(&waiter.target));
		self.key_waiters.retain(|waiter| !targets.contains(&waiter.target));
		self.text_edit_waiters.retain(|waiter| !targets.contains(&waiter.target));
		self.focus_stack.retain(|focused| !targets.contains(focused));
		// Delete known keys instead of searching the removal list for every live entry.
		for id in targets {
			self.geometry.remove(id);
		}

		for task in self.tasks.iter_mut() {
			task.inbox.retain(|event| !targets.contains(&event.target));
			task.key_inbox.retain(|event| !targets.contains(&event.target));
			task.text_edit_inbox.retain(|event| !targets.contains(&event.target));
		}
	}
}
