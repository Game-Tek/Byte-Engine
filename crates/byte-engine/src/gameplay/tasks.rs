//! Gameplay tasks: async gameplay logic, such as a character's life or a round's phases, that the application's tick
//! polls.
//!
//! A [`TaskRuntime`] is a plain value the application owns and steps by hand, like the other systems. Each tick it
//! advances the simulation clock with [`TaskRuntime::advance`], then polls the woken tasks with
//! [`TaskRuntime::poll`], lending them the application's own state. Between polls the application steps its systems
//! and routes their output to the tasks with [`Tasks::send`], so a task can wait for the animation step or a ray
//! query's answer and continue in the same tick.
//!
//! Tasks wait on four things, all through [`Tasks`]:
//! - [`Tasks::tick`] resolves at the next tick with the simulation time since the wait began.
//! - [`Tasks::sleep`] resolves once that much simulation time has passed.
//! - [`Tasks::received`] resolves with the next message [sent](Tasks::send) to an entity, of one type.
//! - [`Tasks::with`] resolves on its first poll with the state lent to the poll.
//!
//! Tasks never read wall-clock time. The clock moves only by the deltas [`TaskRuntime::advance`] receives, so
//! pausing, slowing, or fixing the step happens in one place, and every task polled in a tick reads the same time.
//!
//! # Waits and cancellation
//!
//! A wait stays registered only while its task keeps polling the future that made it: every poll numbers the task,
//! and a wait that the latest poll did not register again no longer wakes the task or receives messages. Dropping a
//! wait future, as the losing branch of a `select!` is, cancels it that way, without a destructor. Combinators that
//! poll every live branch on each poll, such as `select!` and `join!`, keep their branches registered. Do not hold a
//! pending wait in a combinator that polls only woken branches.
//!
//! A [`Tasks::tick`] first polled in a poll takes over the tick wait the same task dropped in that poll, so a task
//! that selects over messages and ticks in a loop still counts every tick while messages keep winning, and a task
//! that switches from one loop to another mid-tick resolves the new loop's tick in the same poll.
//!
//! Next, create a runtime with [`TaskRuntime::new`] and spawn the first tasks with [`Tasks::spawn_on`].

use std::{
	any::{Any, TypeId},
	cell::RefCell,
	cmp::{Ordering, Reverse},
	collections::{BinaryHeap, VecDeque},
	future::Future,
	marker::PhantomData,
	pin::Pin,
	rc::Rc,
	sync::Arc,
	task::{ContextBuilder, Poll, Wake, Waker},
};

use smallvec::SmallVec;
use utils::{StableVec, StableVecHandle, r#async::FusedFuture, hash::HashMap, sync::Mutex};

use crate::{
	MediaTime,
	application::Time,
	core::{
		factory::Handle,
		listener::{DefaultListener, Listener as _},
		message::DeleteMessage,
		message_bus::MessageScope,
	},
	gameplay::World as _,
};

type BoxedTask = Pin<Box<dyn Future<Output = ()> + 'static>>;

/// Task slots are reused, so a wake or a wait left by an ended task finds nothing instead of its successor.
type TaskId = StableVecHandle;

/// The `TaskRuntime` struct runs an application's gameplay tasks from its tick, lending them the state `S` while they
/// are polled.
///
/// Create it on the world scope with [`Self::new`], spawn tasks through [`Self::tasks`], and from each tick call
/// [`Self::advance`] once and [`Self::poll`] after every stage whose output tasks wait for. Tasks run on the thread
/// that polls, one at a time, in the order they were woken, so the same inputs replay the same way.
pub struct TaskRuntime<S> {
	tasks: Tasks<S>,
	/// The futures of the live tasks, by slot index. They live outside the shared state so a poll never holds it.
	running: Vec<Option<Running>>,
	/// Receives the wakes of foreign futures, which may come from any thread.
	remote: Arc<RemoteWakes>,
	/// Reused to drain [`Self::remote`] without allocating.
	woken: Vec<TaskId>,
	deletions: DefaultListener<DeleteMessage>,
}

/// The `Running` struct keeps one live task's future beside the waker its polls hand out.
struct Running {
	future: BoxedTask,
	waker: Waker,
}

impl<S: 'static> TaskRuntime<S> {
	/// Creates a runtime whose tasks end with the entities of `world` they were spawned on.
	///
	/// Next, spawn tasks through [`Self::tasks`].
	pub fn new(world: &MessageScope) -> Self {
		Self {
			tasks: Tasks {
				state: Rc::new(RefCell::new(State::default())),
				scene: PhantomData,
			},
			running: Vec::new(),
			remote: Arc::new(RemoteWakes::default()),
			woken: Vec::new(),
			deletions: world.deletions_listener(),
		}
	}

	/// Returns the handle tasks spawn, wait, and send through. Clone it into the tasks that need it.
	pub fn tasks(&self) -> &Tasks<S> {
		&self.tasks
	}

	/// Starts a tick: ends the tasks of deleted entities, moves the clock by `time`'s delta, and wakes the tasks
	/// waiting for a tick and those whose sleep is due.
	///
	/// Call it once per tick, before the tick's first [`Self::poll`]. Only the delta counts, so a paused tick that
	/// passes a zero delta still resolves tick waits, with zero, while sleeps hold.
	pub fn advance(&mut self, time: Time) {
		self.end_deleted();

		let mut state = self.tasks.state.borrow_mut();
		let state = &mut *state;
		state.clock.tick += 1;
		state.clock.elapsed += time.delta();

		let mut ticking = std::mem::take(&mut state.ticking);
		for id in ticking.drain(..) {
			let Some(task) = state.tasks.get_mut(id) else { continue };
			task.listed = false;
			// Only the latest poll's waits count; the rest belong to futures the task dropped.
			let poll = task.poll;
			task.ticks.retain(|wait| wait.poll == poll);
			if !task.ticks.is_empty() {
				wake(&mut state.ready, task, id);
			}
		}
		state.ticking = ticking;

		let elapsed = state.clock.elapsed;
		while let Some(Reverse(deadline)) = state.deadlines.peek()
			&& deadline.at <= elapsed
		{
			let Reverse(deadline) = state.deadlines.pop().expect("the deadline was just peeked");
			let Some(task) = state.tasks.get_mut(deadline.task) else {
				continue;
			};
			let Some(index) = task.sleeps.iter().position(|wait| wait.token == deadline.token) else {
				continue;
			};
			if task.sleeps[index].poll == task.poll {
				wake(&mut state.ready, task, deadline.task);
			} else {
				task.sleeps.swap_remove(index);
			}
		}
	}

	/// Polls every woken task, including the tasks those polls spawn or wake, lending each poll `scene`.
	///
	/// Call it after [`Self::advance`] and again after each stage whose output reaches tasks through
	/// [`Tasks::send`], such as an animation step whose root motion a character waits for.
	pub fn poll(&mut self, scene: &mut S) {
		loop {
			self.adopt();

			let id = {
				let mut state = self.tasks.state.borrow_mut();
				let state = &mut *state;
				let Some(id) = state.ready.pop_front() else { break };
				// Generational handles reject wakes left by ended tasks.
				let Some(task) = state.tasks.get_mut(id) else { continue };
				task.queued = false;
				state.polls += 1;
				task.poll = state.polls;
				state.current = Some(id);
				id
			};

			let running = self.running[id.index()]
				.as_mut()
				.expect("a live task's future was adopted before its first poll");
			let result = {
				let mut context = ContextBuilder::from_waker(&running.waker).ext(&mut *scene).build();
				running.future.as_mut().poll(&mut context)
			};
			self.tasks.state.borrow_mut().current = None;

			if result.is_ready() {
				self.tasks.state.borrow_mut().end(id);
				self.release(id);
			}
		}
	}

	/// Moves the tasks spawned since the last poll into [`Self::running`] and queues the wakes foreign futures sent.
	fn adopt(&mut self) {
		let mut state = self.tasks.state.borrow_mut();
		let state = &mut *state;
		for (id, future) in state.spawned.drain(..) {
			// A task can end before its first poll when the world deletes its owner.
			if !state.tasks.contains_handle(id) {
				continue;
			}
			if self.running.len() <= id.index() {
				self.running.resize_with(id.index() + 1, || None);
			}
			let waker = Waker::from(Arc::new(TaskWaker {
				task: id,
				remote: Arc::clone(&self.remote),
			}));
			self.running[id.index()] = Some(Running { future, waker });
		}

		// `wake` skips tasks already queued, so repeated foreign wakes poll a task once.
		std::mem::swap(&mut self.woken, &mut *self.remote.ready.lock());
		for id in self.woken.drain(..) {
			if let Some(task) = state.tasks.get_mut(id) {
				wake(&mut state.ready, task, id);
			}
		}
	}

	/// Ends the tasks spawned on entities the world deleted and drops the messages waiting for those entities.
	fn end_deleted(&mut self) {
		while let Some(deletion) = self.deletions.read() {
			let handle = *deletion.handle();
			let owned = {
				let mut state = self.tasks.state.borrow_mut();
				for mailbox in state.mailboxes.values_mut() {
					mailbox.forget(handle);
				}
				let owned = state.owned.remove(&handle).unwrap_or_default();
				for &id in &owned {
					state.end(id);
				}
				owned
			};
			for id in owned {
				self.release(id);
			}
		}
	}

	/// Drops the future of an ended task. Callers release the shared state first, since a destructor may reach it.
	fn release(&mut self, id: TaskId) {
		if let Some(slot) = self.running.get_mut(id.index()) {
			*slot = None;
		}
	}
}

/// The `Tasks` struct is the handle gameplay tasks spawn, wait, and send messages through.
///
/// Get it from [`TaskRuntime::tasks`] and clone it into each task; clones share one runtime. Its wait futures must be
/// polled by the runtime that made it, inside one of its tasks.
pub struct Tasks<S> {
	state: Rc<RefCell<State>>,
	scene: PhantomData<fn(&mut S)>,
}

impl<S> Clone for Tasks<S> {
	fn clone(&self) -> Self {
		Self {
			state: Rc::clone(&self.state),
			scene: PhantomData,
		}
	}
}

impl<S: 'static> Tasks<S> {
	/// Starts `future` as a task that runs until it returns. It is first polled in the current or next
	/// [`TaskRuntime::poll`].
	pub fn spawn(&self, future: impl Future<Output = ()> + 'static) {
		self.state.borrow_mut().spawn(None, Box::pin(future));
	}

	/// Starts `future` as a task that also ends, dropping its future, when the world deletes `owner`.
	///
	/// Spawn an entity's behavior on the entity, so deleting it cancels whatever the behavior was waiting for.
	pub fn spawn_on(&self, owner: Handle, future: impl Future<Output = ()> + 'static) {
		self.state.borrow_mut().spawn(Some(owner), Box::pin(future));
	}

	/// Returns the simulation time: the sum of every delta [`TaskRuntime::advance`] received.
	pub fn now(&self) -> MediaTime {
		self.state.borrow().clock.elapsed
	}

	/// Returns a future that resolves at the next tick with the simulation time since the wait began.
	///
	/// A task that waited on something else for a few ticks gets all of them, so time-based motion moves by the time
	/// that passed rather than the last tick's delta.
	pub fn tick(&self) -> Tick<S> {
		Tick {
			tasks: self.clone(),
			wait: None,
			done: false,
		}
	}

	/// Returns a future that resolves once `duration` of simulation time has passed since its first poll.
	pub fn sleep(&self, duration: MediaTime) -> Sleep<S> {
		Sleep {
			tasks: self.clone(),
			duration,
			wait: None,
			done: false,
		}
	}

	/// Returns a future that resolves with the next `M` [sent](Self::send) to `handle` while the task waits.
	///
	/// Messages sent while nobody waits are dropped. Messages that reach the task before it takes them queue, so
	/// several sent in one tick are read one wait at a time.
	pub fn received<M: 'static>(&self, handle: Handle) -> Received<S, M> {
		Received {
			tasks: self.clone(),
			handle,
			done: false,
			message: PhantomData,
		}
	}

	/// Delivers `message` to every task waiting for an `M` sent to `handle`, and wakes them. Returns whether any task
	/// was waiting.
	///
	/// The application routes its systems' output with it between polls, and tasks call it to signal each other.
	pub fn send<M: Clone + 'static>(&self, handle: Handle, message: M) -> bool {
		let mut state = self.state.borrow_mut();
		let state = &mut *state;
		let Some(mailbox) = state.mailboxes.get_mut(&TypeId::of::<M>()) else {
			return false;
		};
		let mailbox = mailbox
			.as_any_mut()
			.downcast_mut::<Mailbox<M>>()
			.expect("a mailbox is keyed by its message type");
		let Some(waits) = mailbox.waits.get_mut(&handle) else {
			return false;
		};

		let mut delivered = false;
		waits.retain(|wait| {
			let Some(task) = state.tasks.get_mut(wait.task) else {
				return false;
			};
			if wait.poll != task.poll {
				// A wait the task dropped receives nothing, but keeps what it received before.
				return !wait.inbox.is_empty();
			}
			wait.inbox.push_back(message.clone());
			wake(&mut state.ready, task, wait.task);
			delivered = true;
			true
		});
		if waits.is_empty() {
			mailbox.waits.remove(&handle);
		}
		delivered
	}

	/// Returns a future that runs `read` on its first poll with the state the runtime lends its tasks.
	///
	/// The future is ready at once; awaiting it only reaches the state. Keep the borrow inside `read`: the state is
	/// lent to one poll at a time.
	pub fn with<R>(&self, read: impl FnOnce(&mut S) -> R) -> impl Future<Output = R> {
		let mut read = Some(read);
		std::future::poll_fn(move |context| {
			let read = read.take().expect(
				"A task state read was polled after it completed. The most likely cause is polling a finished future again.",
			);
			let scene = context.ext().downcast_mut::<S>().expect(
				"A task reached for state its poll did not lend. The most likely cause is awaiting it outside the runtime that made its `Tasks`.",
			);
			Poll::Ready(read(scene))
		})
	}
}

/// The `Tick` struct is the future [`Tasks::tick`] returns.
pub struct Tick<S> {
	tasks: Tasks<S>,
	/// The wait's token and the tick and time it counts from, once first polled.
	wait: Option<(u64, u64, MediaTime)>,
	done: bool,
}

impl<S> Unpin for Tick<S> {}

impl<S: 'static> Future for Tick<S> {
	type Output = MediaTime;

	fn poll(self: Pin<&mut Self>, _: &mut std::task::Context<'_>) -> Poll<MediaTime> {
		// Split the borrow so the shared state stays borrowed through the fields this poll sets.
		let this = self.get_mut();
		if this.done {
			return Poll::Pending;
		}
		let mut state = this.tasks.state.borrow_mut();
		let state = &mut *state;
		let (id, poll) = state.current();
		let clock = state.clock;
		let task = state.tasks.get_mut(id).expect("the polled task is live");

		let (token, since, since_elapsed) = match this.wait {
			Some(wait) => wait,
			None => {
				state.tokens += 1;
				// Take over the tick wait of a future this task dropped in this poll, so its ticks still count.
				let replaced = task
					.ticks
					.iter()
					.enumerate()
					.filter(|(_, wait)| wait.poll != poll)
					.min_by_key(|(_, wait)| wait.since)
					.map(|(index, _)| index);
				let (since, since_elapsed) = match replaced {
					Some(index) => {
						let wait = task.ticks.swap_remove(index);
						(wait.since, wait.since_elapsed)
					}
					None => (clock.tick, clock.elapsed),
				};
				(state.tokens, since, since_elapsed)
			}
		};

		if clock.tick > since {
			task.ticks.retain(|wait| wait.token != token);
			this.done = true;
			return Poll::Ready(clock.elapsed - since_elapsed);
		}

		match task.ticks.iter_mut().find(|wait| wait.token == token) {
			Some(wait) => wait.poll = poll,
			None => task.ticks.push(TickWait {
				token,
				since,
				since_elapsed,
				poll,
			}),
		}
		if !task.listed {
			task.listed = true;
			state.ticking.push(id);
		}
		this.wait = Some((token, since, since_elapsed));
		Poll::Pending
	}
}

impl<S: 'static> FusedFuture for Tick<S> {
	fn is_terminated(&self) -> bool {
		self.done
	}
}

/// The `Sleep` struct is the future [`Tasks::sleep`] returns.
pub struct Sleep<S> {
	tasks: Tasks<S>,
	duration: MediaTime,
	/// The wait's token and deadline, once first polled.
	wait: Option<(u64, MediaTime)>,
	done: bool,
}

impl<S> Unpin for Sleep<S> {}

impl<S: 'static> Future for Sleep<S> {
	type Output = ();

	fn poll(self: Pin<&mut Self>, _: &mut std::task::Context<'_>) -> Poll<()> {
		// Split the borrow so the shared state stays borrowed through the fields this poll sets.
		let this = self.get_mut();
		if this.done {
			return Poll::Pending;
		}
		let mut state = this.tasks.state.borrow_mut();
		let state = &mut *state;
		let (id, poll) = state.current();
		let elapsed = state.clock.elapsed;
		let (token, deadline) = match this.wait {
			Some(wait) => wait,
			None => {
				state.tokens += 1;
				(state.tokens, elapsed + this.duration)
			}
		};
		let task = state.tasks.get_mut(id).expect("the polled task is live");

		if elapsed >= deadline {
			task.sleeps.retain(|wait| wait.token != token);
			this.done = true;
			return Poll::Ready(());
		}

		match task.sleeps.iter_mut().find(|wait| wait.token == token) {
			Some(wait) => wait.poll = poll,
			// A first poll, or a wait that went stale while the task stopped polling it and lost its deadline.
			None => {
				task.sleeps.push(SleepWait { token, poll });
				state.deadlines.push(Reverse(Deadline {
					at: deadline,
					token,
					task: id,
				}));
			}
		}
		this.wait = Some((token, deadline));
		Poll::Pending
	}
}

impl<S: 'static> FusedFuture for Sleep<S> {
	fn is_terminated(&self) -> bool {
		self.done
	}
}

/// The `Received` struct is the future [`Tasks::received`] returns.
pub struct Received<S, M> {
	tasks: Tasks<S>,
	handle: Handle,
	done: bool,
	message: PhantomData<fn() -> M>,
}

impl<S, M> Unpin for Received<S, M> {}

impl<S: 'static, M: 'static> Future for Received<S, M> {
	type Output = M;

	fn poll(self: Pin<&mut Self>, _: &mut std::task::Context<'_>) -> Poll<M> {
		// Split the borrow so the shared state stays borrowed through the fields this poll sets.
		let this = self.get_mut();
		if this.done {
			return Poll::Pending;
		}
		let mut state = this.tasks.state.borrow_mut();
		let (id, poll) = state.current();
		let waits = state.mailbox::<M>().waits.entry(this.handle).or_default();

		match waits.iter().position(|wait| wait.task == id) {
			Some(index) => {
				if let Some(message) = waits[index].inbox.pop_front() {
					if waits[index].inbox.is_empty() {
						waits.remove(index);
					}
					this.done = true;
					return Poll::Ready(message);
				}
				waits[index].poll = poll;
			}
			None => waits.push(Wait {
				task: id,
				poll,
				inbox: VecDeque::new(),
			}),
		}
		Poll::Pending
	}
}

impl<S: 'static, M: 'static> FusedFuture for Received<S, M> {
	fn is_terminated(&self) -> bool {
		self.done
	}
}

/// The `State` struct is what the runtime and every [`Tasks`] handle share: the clock, the tasks' waits, and the
/// queue of woken tasks.
///
/// It is borrowed only inside runtime and future methods, never across a task's poll, so a task can spawn, send, and
/// wait from anywhere in its code.
#[derive(Default)]
struct State {
	clock: Clock,
	tasks: StableVec<TaskState>,
	ready: VecDeque<TaskId>,
	/// The task being polled. `None` between polls.
	current: Option<TaskId>,
	/// Numbers task polls so a wait can tell whether the latest poll of its task registered it.
	polls: u64,
	/// Hands every tick and sleep wait a distinct token.
	tokens: u64,
	/// The tasks with a tick wait, which the next [`TaskRuntime::advance`] wakes.
	ticking: Vec<TaskId>,
	deadlines: BinaryHeap<Reverse<Deadline>>,
	/// One [`Mailbox`] per message type, keyed by its type.
	mailboxes: HashMap<TypeId, Box<dyn ErasedMailbox>>,
	/// The tasks each entity owns; see [`Tasks::spawn_on`].
	owned: HashMap<Handle, SmallVec<[TaskId; 2]>>,
	/// Spawned tasks whose futures the runtime has not taken yet.
	spawned: Vec<(TaskId, BoxedTask)>,
}

impl State {
	fn spawn(&mut self, owner: Option<Handle>, future: BoxedTask) {
		let id = self.tasks.push(TaskState {
			owner,
			queued: true,
			..TaskState::default()
		});
		if let Some(owner) = owner {
			self.owned.entry(owner).or_default().push(id);
		}
		self.ready.push_back(id);
		self.spawned.push((id, future));
	}

	/// Forgets an ended task. Its waits stay where they are until a delivery or a deadline finds them dead.
	fn end(&mut self, id: TaskId) {
		let Some(task) = self.tasks.remove(id) else { return };
		if let Some(owner) = task.owner
			&& let Some(owned) = self.owned.get_mut(&owner)
		{
			owned.retain(|owned| *owned != id);
			if owned.is_empty() {
				self.owned.remove(&owner);
			}
		}
	}

	/// Returns the task being polled and the number of its poll.
	fn current(&self) -> (TaskId, u64) {
		let id = self.current.expect(
			"A gameplay task wait was polled outside a task poll. The most likely cause is awaiting it on another executor instead of inside a task its runtime polls.",
		);
		let poll = self.tasks.get(id).expect("the polled task is live").poll;
		(id, poll)
	}

	fn mailbox<M: 'static>(&mut self) -> &mut Mailbox<M> {
		self.mailboxes
			.entry(TypeId::of::<M>())
			.or_insert_with(|| Box::new(Mailbox::<M>::default()))
			.as_any_mut()
			.downcast_mut()
			.expect("a mailbox is keyed by its message type")
	}
}

#[derive(Default)]
struct TaskState {
	/// The number of the task's latest poll; see [`State::polls`].
	poll: u64,
	/// Whether the task is in [`State::ready`].
	queued: bool,
	/// Whether the task is in [`State::ticking`].
	listed: bool,
	owner: Option<Handle>,
	ticks: SmallVec<[TickWait; 1]>,
	sleeps: SmallVec<[SleepWait; 1]>,
}

/// Queues `task` for the next poll unless it already is.
fn wake(ready: &mut VecDeque<TaskId>, task: &mut TaskState, id: TaskId) {
	if !task.queued {
		task.queued = true;
		ready.push_back(id);
	}
}

#[derive(Clone, Copy, Default)]
struct Clock {
	/// Counts [`TaskRuntime::advance`] calls, so a tick wait knows whether a tick began since it started.
	tick: u64,
	elapsed: MediaTime,
}

struct TickWait {
	token: u64,
	/// The tick and time the wait counts from.
	since: u64,
	since_elapsed: MediaTime,
	poll: u64,
}

struct SleepWait {
	token: u64,
	poll: u64,
}

/// The `Deadline` struct orders pending sleeps by when they are due; the token breaks ties so equal times stay
/// deterministic.
struct Deadline {
	at: MediaTime,
	token: u64,
	task: TaskId,
}

impl PartialEq for Deadline {
	fn eq(&self, other: &Self) -> bool {
		self.cmp(other) == Ordering::Equal
	}
}

impl Eq for Deadline {}

impl PartialOrd for Deadline {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for Deadline {
	fn cmp(&self, other: &Self) -> Ordering {
		(self.at, self.token).cmp(&(other.at, other.token))
	}
}

/// The `Mailbox` struct holds the waits for one message type, by the entity the messages are sent to.
struct Mailbox<M> {
	waits: HashMap<Handle, SmallVec<[Wait<M>; 1]>>,
}

impl<M> Default for Mailbox<M> {
	fn default() -> Self {
		Self {
			waits: HashMap::default(),
		}
	}
}

/// The `Wait` struct is one task's wait for messages sent to one entity, and the messages it received.
struct Wait<M> {
	task: TaskId,
	/// The number of the task poll that last registered this wait.
	poll: u64,
	inbox: VecDeque<M>,
}

/// The `ErasedMailbox` trait lets the runtime keep every message type's mailbox in one map.
trait ErasedMailbox {
	fn as_any_mut(&mut self) -> &mut dyn Any;

	/// Drops the waits and messages for a deleted entity.
	fn forget(&mut self, handle: Handle);
}

impl<M: 'static> ErasedMailbox for Mailbox<M> {
	fn as_any_mut(&mut self) -> &mut dyn Any {
		self
	}

	fn forget(&mut self, handle: Handle) {
		self.waits.remove(&handle);
	}
}

/// The `RemoteWakes` struct collects wakes of foreign futures, such as channels, which may arrive from any thread.
#[derive(Default)]
struct RemoteWakes {
	ready: Mutex<Vec<TaskId>>,
}

/// The `TaskWaker` struct is the [`Waker`] a task's polls hand out, so futures such as channels can wake the task.
///
/// The runtime's own waits wake tasks directly; this only carries the wakes of futures that use the waker.
struct TaskWaker {
	task: TaskId,
	remote: Arc<RemoteWakes>,
}

impl Wake for TaskWaker {
	fn wake(self: Arc<Self>) {
		self.wake_by_ref();
	}

	fn wake_by_ref(self: &Arc<Self>) {
		self.remote.ready.lock().push(self.task);
	}
}

#[cfg(test)]
mod tests;
