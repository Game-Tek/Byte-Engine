//! Task scheduling, event delivery, focus, and retained runtime state.

use std::{
	sync::atomic::{AtomicBool, Ordering},
	task::ContextBuilder,
	time::Instant,
};

use super::*;

pub(super) type BoxedUiFuture = Pin<Box<dyn Future<Output = ()> + 'static>>;

/// The `UiTask` struct keeps one spawned component alive until its future completes or its owning scope is removed.
///
/// The task also owns the waits its futures registered. A wait stays registered only while the task keeps polling
/// the future that made it: after each poll, [`Runtime::poll_ready_tasks`] drops the waits that poll did not
/// register again. See [`UiPoll`] for how a wait future reaches its task.
#[derive(Default)]
pub(super) struct UiTask {
	/// Empty while the task is being polled.
	pub(super) future: Option<BoxedUiFuture>,
	/// Reused across polls; its queued flag coalesces concurrent wake requests.
	pub(super) waker: Option<Arc<TaskWaker>>,
	pub(super) owner: ScopeId,
	/// The path the task was declared at, so removing an element it was declared under ends it.
	pub(super) path: u64,
	pub(super) events: Channel<UiEvent>,
	pub(super) keys: Channel<UiKeyEvent>,
	pub(super) text_edits: Channel<UiTextEditEvent>,
	pub(super) frame_waits: Vec<FrameWait>,
	pub(super) timer_waits: Vec<Registration<Instant>>,
}

impl UiTask {
	/// Empties an ended task's storage, keeping its capacity and waker for the next spawned task.
	fn clear(&mut self) {
		self.future = None;
		self.events.clear();
		self.keys.clear();
		self.text_edits.clear();
		self.frame_waits.clear();
		self.timer_waits.clear();
	}

	/// Wakes the task so the next [`Runtime::poll_ready_tasks`] polls it.
	fn wake(&self) {
		if let Some(waker) = &self.waker {
			Wake::wake_by_ref(waker);
		}
	}

	/// Drops every wait the poll numbered `poll` did not register again.
	///
	/// A future that its task stopped polling, such as the losing branch of a selection, was dropped or put aside,
	/// so its wait must not deliver input or keep the engine ticking.
	fn keep_waits_from(&mut self, poll: u64) {
		self.frame_waits.retain(|wait| wait.polled == poll);
		self.events.keep_waits_from(poll);
		self.keys.keep_waits_from(poll);
		self.text_edits.keep_waits_from(poll);
		self.timer_waits.retain(|wait| wait.polled == poll);
	}
}

/// The `Registration` struct records one wait a task's future made and the poll that last made it.
pub(super) struct Registration<K> {
	pub(super) key: K,
	/// The number of the task poll that last registered this wait.
	pub(super) polled: u64,
}

/// Registers `key` for the current poll, reusing the entry an earlier poll left.
fn register<K: PartialEq>(waits: &mut Vec<Registration<K>>, key: K, poll: u64) {
	match waits.iter_mut().find(|wait| wait.key == key) {
		Some(wait) => wait.polled = poll,
		None => waits.push(Registration { key, polled: poll }),
	}
}

/// The `UiInput` trait describes one kind of input a component waits for on an element, such as a pointer event or
/// a key.
///
/// Wait for one with [`ContainerContext::on`], [`ContainerContext::on_key`], or [`ContainerContext::on_text_edit`];
/// each returns an [`InputFuture`] that resolves with the next matching input for its element.
///
/// [`ContainerContext::on`]: crate::ui::ContainerContext::on
/// [`ContainerContext::on_key`]: crate::ui::ContainerContext::on_key
/// [`ContainerContext::on_text_edit`]: crate::ui::ContainerContext::on_text_edit
pub trait UiInput: Clone + 'static {
	/// What a wait selects among the inputs for its element, such as the pointer event kind.
	type Filter: PartialEq + Copy;

	/// Returns the element this input is addressed to.
	fn target(&self) -> Id;

	/// Reports whether a wait for `filter` receives this input.
	fn matches(&self, filter: Self::Filter) -> bool;
}

/// The `Routed` trait finds the [`Channel`] of an input kind in a task, so delivery code is written once.
///
/// [`Runtime::push_event`] delivers every kind through it. A new input kind implements [`UiInput`] and `Routed` and
/// adds one [`Channel`] to [`UiTask`].
pub(super) trait Routed: UiInput {
	fn channel(task: &mut UiTask) -> &mut Channel<Self>;
}

impl UiInput for UiEvent {
	type Filter = Events;

	fn target(&self) -> Id {
		self.target
	}

	fn matches(&self, kind: Events) -> bool {
		self.kind == kind
	}
}

impl Routed for UiEvent {
	fn channel(task: &mut UiTask) -> &mut Channel<Self> {
		&mut task.events
	}
}

impl UiInput for UiKeyEvent {
	type Filter = Key;

	fn target(&self) -> Id {
		self.target
	}

	fn matches(&self, key: Key) -> bool {
		self.key == key
	}
}

impl Routed for UiKeyEvent {
	fn channel(task: &mut UiTask) -> &mut Channel<Self> {
		&mut task.keys
	}
}

/// A text-edit wait receives every edit of its element, so it has nothing to select.
impl UiInput for UiTextEditEvent {
	type Filter = ();

	fn target(&self) -> Id {
		self.target
	}

	fn matches(&self, _: ()) -> bool {
		true
	}
}

impl Routed for UiTextEditEvent {
	fn channel(task: &mut UiTask) -> &mut Channel<Self> {
		&mut task.text_edits
	}
}

/// The `Channel` struct holds one task's waits for one input kind and the inputs delivered to them.
///
/// An input is kept only when the task waits for it; see [`Runtime::push_event`].
pub(super) struct Channel<E: UiInput> {
	/// Delivered inputs the task has not taken yet, in delivery order.
	inbox: VecDeque<E>,
	waits: Vec<Registration<(Id, E::Filter)>>,
}

impl<E: UiInput> Default for Channel<E> {
	fn default() -> Self {
		Self {
			inbox: VecDeque::new(),
			waits: Vec::new(),
		}
	}
}

impl<E: UiInput> Channel<E> {
	/// Moves `input` into the inbox and ends the waits it satisfies. Returns `false`, keeping nothing, when no wait
	/// selects it.
	fn deliver(&mut self, input: &E) -> bool {
		let waiting = self.waits.len();
		self.waits
			.retain(|wait| !(wait.key.0 == input.target() && input.matches(wait.key.1)));
		let delivered = self.waits.len() != waiting;
		if delivered {
			self.inbox.push_back(input.clone());
		}
		delivered
	}

	/// Takes the oldest delivered input for `target` that a wait for `filter` receives.
	fn take(&mut self, target: Id, filter: E::Filter) -> Option<E> {
		let index = self
			.inbox
			.iter()
			.position(|input| input.target() == target && input.matches(filter))?;
		self.inbox.remove(index)
	}

	fn keep_waits_from(&mut self, poll: u64) {
		self.waits.retain(|wait| wait.polled == poll);
	}

	fn clear(&mut self) {
		self.inbox.clear();
		self.waits.clear();
	}

	/// Drops the waits and inputs of removed elements.
	fn remove_targets(&mut self, targets: &utils::hash::HashSet<Id>) {
		self.waits.retain(|wait| !targets.contains(&wait.key.0));
		self.inbox.retain(|input| !targets.contains(&input.target()));
	}
}

/// The `FrameWait` struct keeps one pending [`RenderFuture`] and the frame it counts from.
///
/// A frame wait its task stopped polling, as the losing branch of a selection is, stays until the poll ends. A new
/// frame wait first polled in that poll takes over its frame, so a component that selects over events and frames in a
/// loop still sees every frame while events keep winning.
pub(super) struct FrameWait {
	/// Identifies the [`RenderFuture`] that owns this wait.
	pub(super) token: u64,
	/// The frame the wait counts from. It resolves once a later frame begins.
	pub(super) seen: u64,
	/// The number of the task poll that last registered this wait.
	pub(super) polled: u64,
}

/// Task slots are reused, so a handle from a removed task finds nothing instead of its successor.
pub(super) type TaskId = StableVecHandle;

/// The `ScopeId` struct identifies one mounted scope so the tasks spawned inside it end with it.
///
/// Scope paths can repeat between live mounts, so ownership uses this identity instead,
/// which an engine never hands out twice.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScopeId(u64);

impl ScopeId {
	/// The scope of the root component, which is never removed.
	pub(super) const ROOT: Self = Self(0);
}

pub(super) fn sanitize_opacity(opacity: f32) -> f32 {
	if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 }
}

pub struct Runtime {
	pub(super) tasks: StableVec<UiTask>,
	/// Ended tasks whose storage and waker the next spawned tasks reuse, so a screen that opens again allocates none.
	spare_tasks: Vec<UiTask>,
	/// Reused by [`Self::end_tasks`] to collect the tasks it ends.
	ending: Vec<TaskId>,
	next_scope: u64,
	/// Shared with every [`TaskWaker`] so a wake from any thread schedules its task.
	pub(super) wakes: Arc<WakeQueue>,
	/// Numbers task polls so a wait can tell whether the current poll registered it.
	polls: u64,
	/// Hands every [`RenderFuture`] a distinct [`FrameWait::token`].
	next_frame_token: u64,
	pub(super) focus_stack: Vec<Id>,
	/// Keyed by id, which is already a well-mixed hash, so the fast hasher is enough.
	pub(super) geometry: utils::hash::HashMap<Id, Geometry>,
	pub(super) pointer: PointerState,
	pub(super) drag: Drag,
	pub(super) frame: u64,
}

/// The `WakeQueue` struct lets task wakers schedule polls without reaching into the runtime.
///
/// [`Wake`] requires an [`Arc`], so the runtime and its [`TaskWaker`]s share this queue.
pub(super) struct WakeQueue {
	pub(super) ready: Mutex<VecDeque<TaskId>>,
	/// Runs the tick that polls woken tasks; see [`super::Engine::set_waker`].
	pub(super) host: Mutex<Option<Waker>>,
}

/// The `TaskWaker` struct keeps a live task scheduled at most once between polls.
pub(super) struct TaskWaker {
	pub(super) task: TaskId,
	queued: AtomicBool,
	queue: Arc<WakeQueue>,
}

impl Wake for TaskWaker {
	fn wake(self: Arc<Self>) {
		self.wake_by_ref();
	}

	fn wake_by_ref(self: &Arc<Self>) {
		if !self.queued.swap(true, Ordering::AcqRel) {
			self.queue.ready.lock().push_back(self.task);
			// A wake from another thread must reach the host so the task is polled in its next tick.
			if let Some(host) = self.queue.host.lock().as_ref() {
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
			spare_tasks: Vec::new(),
			ending: Vec::new(),
			next_scope: ScopeId::ROOT.0,
			wakes: Arc::new(WakeQueue {
				ready: Mutex::new(VecDeque::with_capacity(TASK_CAPACITY)),
				host: Mutex::new(None),
			}),
			polls: 0,
			next_frame_token: 0,
			focus_stack: Vec::new(),
			geometry: utils::hash::HashMap::default(),
			pointer: PointerState::default(),
			drag: Drag::new(DRAG_THRESHOLD),
			frame: 0,
		}
	}

	/// Reports whether a component waits for the next frame or to be polled.
	pub(super) fn needs_tick(&self) -> bool {
		self.tasks.iter().any(|task| !task.frame_waits.is_empty()) || !self.wakes.ready.lock().is_empty()
	}

	/// Returns the earliest deadline a pending UI timer waits for.
	pub(super) fn next_deadline(&self) -> Option<Instant> {
		self.tasks
			.iter()
			.flat_map(|task| task.timer_waits.iter().map(|wait| wait.key))
			.min()
	}

	/// Returns an identity for a newly mounted scope.
	pub(super) fn next_scope(&mut self) -> ScopeId {
		self.next_scope += 1;
		ScopeId(self.next_scope)
	}

	/// Starts a task owned by `owner` and declared at `path`, and schedules its first poll.
	///
	/// The task reuses the storage of an ended one when there is one, and its waker too when nothing else holds it.
	pub(super) fn spawn(&mut self, owner: ScopeId, path: u64, future: BoxedUiFuture) {
		let mut task = self.spare_tasks.pop().unwrap_or_default();
		task.future = Some(future);
		task.owner = owner;
		task.path = path;
		let id = self.tasks.push(task);
		let task = self.tasks.get_mut(id).expect("A UI task was removed while it was spawned.");
		// The waker names its task, so it is set once the task has a slot. A waker something else still holds could
		// wake the new task for the old one, so only a uniquely held one is renamed.
		match task.waker.as_mut().and_then(Arc::get_mut) {
			Some(waker) => {
				waker.task = id;
				*waker.queued.get_mut() = false;
			}
			None => {
				task.waker = Some(Arc::new(TaskWaker {
					task: id,
					queue: Arc::clone(&self.wakes),
					queued: AtomicBool::new(false),
				}))
			}
		}
		task.wake();
	}

	/// Removes a task that ended and keeps its storage for the next spawned task.
	fn recycle(&mut self, id: TaskId) {
		if let Some(mut task) = self.tasks.remove(id) {
			task.clear();
			self.spare_tasks.push(task);
		}
	}

	/// Ends every task the predicate selects and drops its future.
	///
	/// A dropped future may own mounted scopes, which send the commands that end them; see
	/// [`UiPoll::apply_commands`].
	pub(super) fn end_tasks(&mut self, select: impl Fn(&UiTask) -> bool) {
		let mut ending = std::mem::take(&mut self.ending);
		ending.extend(self.tasks.handled_iter().filter(|(_, task)| select(task)).map(|(id, _)| id));
		for id in ending.drain(..) {
			self.recycle(id);
		}
		self.ending = ending;
	}

	/// Starts a frame: wakes the tasks that wait for one and the timers that are due.
	pub(super) fn begin_frame(&mut self) {
		self.frame += 1;
		self.wake_due_timers(Instant::now());

		for task in self.tasks.iter().filter(|task| !task.frame_waits.is_empty()) {
			task.wake();
		}
	}

	/// Wakes every task with a UI timer that is due at `now`.
	pub(super) fn wake_due_timers(&mut self, now: Instant) {
		for task in self.tasks.iter_mut() {
			let waiting = task.timer_waits.len();
			task.timer_waits.retain(|wait| wait.key > now);
			if task.timer_waits.len() != waiting {
				task.wake();
			}
		}
	}

	/// Starts a frame wait for a [`RenderFuture`] polled for the first time and returns its token and frame.
	///
	/// The wait takes over the frame of a wait its task has not polled in this poll yet, which is usually one it
	/// dropped. Frames that began while that wait was pending then still count. Otherwise it counts from the current
	/// frame.
	pub(super) fn start_frame_wait(&mut self, task: TaskId, poll: u64) -> (u64, u64) {
		self.next_frame_token += 1;
		let token = self.next_frame_token;
		let frame = self.frame;
		let Some(task) = self.tasks.get_mut(task) else {
			return (token, frame);
		};
		let replaced = task
			.frame_waits
			.iter()
			.enumerate()
			.filter(|(_, wait)| wait.polled != poll)
			.min_by_key(|(_, wait)| wait.seen)
			.map(|(index, _)| index);
		let seen = replaced.map_or(frame, |index| task.frame_waits.swap_remove(index).seen);
		(token, seen)
	}

	/// Keeps the frame wait `token` registered for the current poll.
	pub(super) fn keep_frame_wait(&mut self, task: TaskId, token: u64, seen: u64, poll: u64) {
		let Some(task) = self.tasks.get_mut(task) else { return };
		match task.frame_waits.iter_mut().find(|wait| wait.token == token) {
			Some(wait) => wait.polled = poll,
			None => task.frame_waits.push(FrameWait {
				token,
				seen,
				polled: poll,
			}),
		}
	}

	/// Removes the frame wait `token` once its future resolved.
	pub(super) fn end_frame_wait(&mut self, task: TaskId, token: u64) {
		if let Some(task) = self.tasks.get_mut(task) {
			task.frame_waits.retain(|wait| wait.token != token);
		}
	}

	/// Registers the current poll of `task` as waiting for `E` inputs to `target` that `filter` selects.
	pub(super) fn wait_for_event<E: Routed>(&mut self, task: TaskId, target: Id, filter: E::Filter, poll: u64) {
		if let Some(task) = self.tasks.get_mut(task) {
			register(&mut E::channel(task).waits, (target, filter), poll);
		}
	}

	pub(super) fn wait_until(&mut self, task: TaskId, deadline: Instant, poll: u64) {
		if let Some(task) = self.tasks.get_mut(task) {
			register(&mut task.timer_waits, deadline, poll);
		}
	}

	/// Delivers an input to every task that waits for it.
	///
	/// Inputs that fire while nobody awaits them are discarded instead of queuing for a later wait. An input already
	/// delivered to a task's inbox stays: several inputs can land in one frame, and a component reads them one wait
	/// at a time.
	pub(super) fn push_event<E: Routed>(&mut self, input: E) {
		for task in self.tasks.iter_mut() {
			if E::channel(task).deliver(&input) {
				task.wake();
			}
		}
	}

	/// Takes the oldest input `task` received for `target` that `filter` selects.
	pub(super) fn take_event<E: Routed>(&mut self, task: TaskId, target: Id, filter: E::Filter) -> Option<E> {
		E::channel(self.tasks.get_mut(task)?).take(target, filter)
	}

	pub(super) fn request_focus(&mut self, target: Id) {
		self.focus_stack.retain(|focused| *focused != target);
		self.focus_stack.push(target);
	}

	pub(super) fn release_focus(&mut self, target: Id) {
		self.focus_stack.retain(|focused| *focused != target);
	}

	/// Returns the element key and text input reach: the top of the focus stack among the laid-out elements.
	///
	/// Entries for elements the last layout did not place are dropped, so focus falls back to the element focused
	/// before them.
	pub(super) fn focused(&mut self) -> Option<Id> {
		let geometry = &self.geometry;
		self.focus_stack.retain(|focused| geometry.contains_key(focused));
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
	pub(super) fn remove_targets(&mut self, targets: &utils::hash::HashSet<Id>) {
		self.focus_stack.retain(|focused| !targets.contains(focused));
		// Delete known keys instead of searching the removal list for every live entry.
		for id in targets {
			self.geometry.remove(id);
		}

		for task in self.tasks.iter_mut() {
			task.events.remove_targets(targets);
			task.keys.remove_targets(targets);
			task.text_edits.remove_targets(targets);
		}
	}
}

/// Polls every woken task, applying the removals of mounts each poll dropped before polling the next one.
///
/// Each poll lends `core` to the task's futures through [`TaskContext::ext`], so their writes land during the poll;
/// see [`UiPoll`]. After the poll, the task keeps only the waits that poll registered, which cancels the waits of
/// futures it dropped or stopped polling.
pub(super) fn poll_ready_tasks<C: 'static>(core: &mut UiPoll<C>) {
	loop {
		let runtime = &mut core.runtime;
		let Some(id) = runtime.wakes.ready.lock().pop_front() else {
			return;
		};
		runtime.polls += 1;
		let poll = runtime.polls;
		// Generational handles reject wakes left by removed tasks.
		let Some(task) = runtime.tasks.get_mut(id) else { continue };
		let Some(mut future) = task.future.take() else { continue };
		let waker = task
			.waker
			.as_ref()
			.expect("A running UI task has no waker. The task was not started by its runtime.");
		// Acquire preceding wakes, then clear before polling so a new wake schedules another poll.
		waker.queued.swap(false, Ordering::AcqRel);
		let waker = Waker::from(Arc::clone(waker));

		core.current = Some((id, poll));
		let result = {
			let mut cx = ContextBuilder::from_waker(&waker).ext(&mut *core).build();
			future.as_mut().poll(&mut cx)
		};
		core.current = None;

		let runtime = &mut core.runtime;
		match result {
			Poll::Ready(()) => {
				drop(future);
				runtime.recycle(id);
			}
			Poll::Pending => {
				if let Some(task) = runtime.tasks.get_mut(id) {
					task.keep_waits_from(poll);
					task.future = Some(future);
				}
			}
		}
		core.apply_commands();
	}
}
