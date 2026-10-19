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
//! and a wait the latest poll did not register again no longer wakes the task or receives messages. Dropping a wait
//! future, as the losing branch of a `select!` is, cancels it that way, and the runtime removes the waits a poll
//! dropped once that poll ends. A task that ends leaves no waits behind. Combinators that
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

/// Indexes [`State::mailboxes`]. Waits name their mailbox by slot rather than by [`TypeId`], which keeps each task's
/// list of waits small.
type MailboxSlot = u32;

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
	/// Swapped with [`State::spawned`] so spawned futures are adopted, or dropped, with the shared state released.
	adopting: Vec<(TaskId, BoxedTask)>,
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
			adopting: Vec::new(),
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
			// Only the latest poll's waits count; the rest belong to futures the task dropped or stopped polling.
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
				state.sleeping -= 1;
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
			let mut state = self.tasks.state.borrow_mut();
			state.current = None;
			let sweep = std::mem::take(&mut state.sweep_current);
			if result.is_ready() {
				state.end(id);
				drop(state);
				self.release(id);
			} else if sweep {
				state.sweep(id);
			}
		}
	}

	/// Moves the tasks spawned since the last poll into [`Self::running`] and queues the wakes foreign futures sent.
	fn adopt(&mut self) {
		std::mem::swap(&mut self.adopting, &mut self.tasks.state.borrow_mut().spawned);
		for (id, future) in self.adopting.drain(..) {
			// A task can end before its first poll when the world deletes its owner. Its future drops here, with the
			// shared state released, since its destructor may reach it.
			if !self.tasks.state.borrow().tasks.contains_handle(id) {
				drop(future);
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

		let mut state = self.tasks.state.borrow_mut();
		let state = &mut *state;
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
				for mailbox in &mut state.mailboxes {
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

impl<S> Drop for TaskRuntime<S> {
	/// Drops every future and message the runtime still keeps.
	///
	/// Tasks not yet polled and queued messages live in the shared state, and they may hold [`Tasks`] handles to it.
	/// That cycle would keep them alive after the runtime is gone, so they are dropped here, with the shared state
	/// released, until their destructors leave nothing behind.
	fn drop(&mut self) {
		self.running.clear();
		loop {
			let (spawned, mailboxes) = {
				let mut state = self.tasks.state.borrow_mut();
				// `Tasks` handles may outlive the runtime, so they must find no slots for the mailboxes taken here.
				state.mailbox_slots.clear();
				(std::mem::take(&mut state.spawned), std::mem::take(&mut state.mailboxes))
			};
			if spawned.is_empty() && mailboxes.is_empty() {
				break;
			}
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
			slot: None,
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
		let Some(&slot) = state.mailbox_slots.get(&TypeId::of::<M>()) else {
			return false;
		};
		let mailbox = State::mailbox_at::<M>(&mut state.mailboxes, slot);
		let Some(waits) = mailbox.waits.get_mut(&handle) else {
			return false;
		};

		let mut delivered = false;
		waits.retain(|wait| {
			let Some(task) = state.tasks.get_mut(wait.task) else {
				return false;
			};
			// A wait its task has not registered again in a poll still under way receives nothing; the sweep after that
			// poll removes it unless the poll registers it.
			if wait.poll == task.poll {
				wait.inbox.push_back(message.clone());
				wake(&mut state.ready, task, wait.task);
				delivered = true;
			}
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
		let id = state.current();
		let clock = state.clock;
		let task = state.tasks.get_mut(id).expect("the polled task is live");
		let poll = task.poll;

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
		let id = state.current();
		let elapsed = state.clock.elapsed;
		let task = state.tasks.get_mut(id).expect("the polled task is live");
		let poll = task.poll;
		let (token, deadline) = match this.wait {
			Some(wait) => wait,
			None => {
				state.tokens += 1;
				(state.tokens, elapsed + this.duration)
			}
		};

		if elapsed >= deadline {
			if let Some(index) = task.sleeps.iter().position(|wait| wait.token == token) {
				task.sleeps.swap_remove(index);
				state.sleeping -= 1;
			}
			this.done = true;
			return Poll::Ready(());
		}

		match task.sleeps.iter_mut().find(|wait| wait.token == token) {
			Some(wait) => wait.poll = poll,
			// A first poll, or a sleep its task stopped polling for a while, which a sweep removed with its deadline.
			None => {
				task.sleeps.push(SleepWait { token, poll });
				state.sleeping += 1;
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

impl<S> Drop for Sleep<S> {
	fn drop(&mut self) {
		if !self.done && self.wait.is_some() {
			flag_dropped_wait(&self.tasks.state);
		}
	}
}

/// The `Received` struct is the future [`Tasks::received`] returns.
pub struct Received<S, M> {
	tasks: Tasks<S>,
	handle: Handle,
	/// The mailbox of `M`, once first polled, so later polls skip the lookup by type.
	slot: Option<MailboxSlot>,
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
		let state = &mut *state;
		let slot = *this.slot.get_or_insert_with(|| state.mailbox_slot::<M>());
		let id = state.current();
		let task = state.tasks.get_mut(id).expect("the polled task is live");
		let poll = task.poll;
		// The task lists the mailboxes it waits in, one entry per wait, so sweeps and its end reach its waits without
		// searching every mailbox.
		let key = (slot, this.handle);
		let waits = State::mailbox_at::<M>(&mut state.mailboxes, slot)
			.waits
			.entry(this.handle)
			.or_default();

		match waits.iter().position(|wait| wait.task == id) {
			Some(index) => {
				if let Some(message) = waits[index].inbox.pop_front() {
					// Keep the emptied wait so the task's next wait reuses its inbox instead of allocating one each
					// message. Until a poll registers it again it receives nothing; a sweep removes it if the task moved
					// on.
					this.done = true;
					return Poll::Ready(message);
				}
				waits[index].poll = poll;
			}
			None => {
				waits.push(Wait {
					task: id,
					poll,
					inbox: VecDeque::new(),
				});
				task.mail.push(key);
				// Entries whose waits are gone, or that the task stopped registering, stay listed until a sweep; sweep
				// once they could make up half the list.
				if task.mail.len() > 2 * task.swept_mail as usize + 4 {
					state.sweep_current = true;
				}
			}
		}
		Poll::Pending
	}
}

impl<S: 'static, M: 'static> FusedFuture for Received<S, M> {
	fn is_terminated(&self) -> bool {
		self.done
	}
}

impl<S, M> Drop for Received<S, M> {
	fn drop(&mut self) {
		if !self.done && self.slot.is_some() {
			flag_dropped_wait(&self.tasks.state);
		}
	}
}

/// Asks the runtime to sweep the polled task once its poll ends, since a wait future it registered was dropped there.
///
/// A wait future dropped outside a poll belongs to a task that is ending, and the task's end removes its waits.
fn flag_dropped_wait(state: &RefCell<State>) {
	if let Ok(mut state) = state.try_borrow_mut()
		&& state.current.is_some()
	{
		state.sweep_current = true;
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
	/// The sleeps tasks wait on, so [`Self::compact_deadlines`] knows how many of the deadlines still count.
	sleeping: usize,
	/// Whether the task being polled left waits behind that [`Self::sweep`] should remove once its poll ends: it dropped
	/// a wait future, or its mailbox list doubled. Polls that leave nothing behind skip the sweep.
	sweep_current: bool,
	/// One [`Mailbox`] per message type, by the slot [`Self::mailbox_slots`] gives the type.
	mailboxes: Vec<Box<dyn ErasedMailbox>>,
	mailbox_slots: HashMap<TypeId, MailboxSlot>,
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

	/// Forgets an ended task and removes its waits.
	fn end(&mut self, id: TaskId) {
		let Some(task) = self.tasks.remove(id) else { return };
		for (slot, handle) in task.mail {
			if let Some(mailbox) = self.mailboxes.get_mut(slot as usize) {
				mailbox.remove(handle, id);
			}
		}
		self.sleeping -= task.sleeps.len();
		self.compact_deadlines();
		if let Some(owner) = task.owner
			&& let Some(owned) = self.owned.get_mut(&owner)
		{
			owned.retain(|owned| *owned != id);
			if owned.is_empty() {
				self.owned.remove(&owner);
			}
		}
	}

	/// Removes the waits that the latest poll of a pending task did not register again, since they belong to futures it
	/// dropped or stopped polling.
	///
	/// The runtime calls it after a poll that dropped a wait future or grew the task's mailbox list; see
	/// [`Self::sweep_current`]. A tick wait it removes has done its job: a tick first polled in the same poll already
	/// took its time over.
	fn sweep(&mut self, id: TaskId) {
		let Some(task) = self.tasks.get_mut(id) else { return };
		let poll = task.poll;
		task.ticks.retain(|wait| wait.poll == poll);
		let sleeps = task.sleeps.len();
		task.sleeps.retain(|wait| wait.poll == poll);
		let swept_sleeps = sleeps - task.sleeps.len();
		let mailboxes = &mut self.mailboxes;
		task.mail.retain(|&mut (slot, handle)| {
			mailboxes
				.get_mut(slot as usize)
				.is_some_and(|mailbox| mailbox.sweep(handle, id, poll))
		});
		task.swept_mail = u32::try_from(task.mail.len()).unwrap_or(u32::MAX);
		if swept_sleeps > 0 {
			self.sleeping -= swept_sleeps;
			self.compact_deadlines();
		}
	}

	/// Rebuilds the deadline heap once most of its entries belong to sleeps that ended, so long sleeps of ended or
	/// cancelled waits don't pile up until they fall due.
	fn compact_deadlines(&mut self) {
		if self.deadlines.len() <= 2 * self.sleeping + 64 {
			return;
		}
		let tasks = &self.tasks;
		self.deadlines.retain(|Reverse(deadline)| {
			tasks
				.get(deadline.task)
				.is_some_and(|task| task.sleeps.iter().any(|wait| wait.token == deadline.token))
		});
	}

	/// Returns the task being polled.
	fn current(&self) -> TaskId {
		self.current.expect(
			"A gameplay task wait was polled outside a task poll. The most likely cause is awaiting it on another executor instead of inside a task its runtime polls.",
		)
	}

	/// Returns the slot of `M`'s mailbox, creating the mailbox the first time a task waits for an `M`.
	fn mailbox_slot<M: 'static>(&mut self) -> MailboxSlot {
		let mailboxes = &mut self.mailboxes;
		*self.mailbox_slots.entry(TypeId::of::<M>()).or_insert_with(|| {
			mailboxes.push(Box::new(Mailbox::<M>::default()));
			MailboxSlot::try_from(mailboxes.len() - 1).expect("there are fewer message types than `u32::MAX`")
		})
	}

	fn mailbox_at<M: 'static>(mailboxes: &mut [Box<dyn ErasedMailbox>], slot: MailboxSlot) -> &mut Mailbox<M> {
		mailboxes[slot as usize]
			.as_any_mut()
			.downcast_mut()
			.expect("a mailbox slot is given to one message type")
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
	/// The mailboxes, by message type and entity, the task has a wait in.
	mail: SmallVec<[(MailboxSlot, Handle); 1]>,
	/// How long [`Self::mail`] was after the last sweep, so the list is swept again once it has doubled.
	swept_mail: u32,
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

	/// Removes `task`'s wait for `handle` unless the poll numbered `poll` registered it, and reports whether the wait
	/// is left.
	fn sweep(&mut self, handle: Handle, task: TaskId, poll: u64) -> bool;

	/// Removes `task`'s wait for `handle`, with the messages it held.
	fn remove(&mut self, handle: Handle, task: TaskId);
}

impl<M: 'static> ErasedMailbox for Mailbox<M> {
	fn as_any_mut(&mut self) -> &mut dyn Any {
		self
	}

	fn forget(&mut self, handle: Handle) {
		self.waits.remove(&handle);
	}

	fn sweep(&mut self, handle: Handle, task: TaskId, poll: u64) -> bool {
		let registered = self
			.waits
			.get(&handle)
			.and_then(|waits| waits.iter().find(|wait| wait.task == task))
			.map(|wait| wait.poll == poll);
		if registered == Some(false) {
			self.remove(handle, task);
		}
		registered == Some(true)
	}

	fn remove(&mut self, handle: Handle, task: TaskId) {
		let Some(waits) = self.waits.get_mut(&handle) else { return };
		waits.retain(|wait| wait.task != task);
		if waits.is_empty() {
			self.waits.remove(&handle);
		}
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
