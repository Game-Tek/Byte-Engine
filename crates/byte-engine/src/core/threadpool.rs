/// The `LanePool` struct keeps one worker thread per lane alive between dispatches, so that a dispatch only has to
/// hand each worker a job.
///
/// Call [`Self::run`] to run one job on a range of workers alongside work on the calling thread. A dispatch allocates
/// nothing: the job stays on the caller's stack, and a worker that is still waiting from the last dispatch starts it
/// without a system call. Mutable access keeps dispatches from overlapping, because lane collectives need every
/// worker of a dispatch to themselves and could otherwise deadlock.
pub struct LanePool {
	shared: Arc<Shared>,
	/// Each worker's thread, to wake the worker when it sleeps.
	threads: Vec<Thread>,
	handles: Vec<JoinHandle<()>>,
	/// How many dispatches the pool has handed out. A worker tells its next job from its last one by this number.
	dispatches: u64,
}

impl LanePool {
	/// Creates one worker lane for each hardware thread available to this process.
	///
	/// Call [`Self::run`] to dispatch work.
	pub fn new() -> Self {
		let parallelism = std::thread::available_parallelism()
			.expect("Lane-pool initialization failed. The operating system did not report available parallelism.")
			.get();
		Self::with_parallelism(parallelism)
	}

	/// Creates an owned pool with `count` persistent worker lanes.
	///
	/// Call [`Self::run`] to dispatch work.
	pub fn with_parallelism(count: usize) -> Self {
		assert!(
			count > 0,
			"Lane-pool initialization failed. The requested parallelism is zero, so no worker could execute jobs."
		);

		let shared = Arc::new(Shared {
			workers: (0..count).map(|_| Worker::default()).collect(),
			caller_sleeping: Padded(AtomicBool::new(false)),
			caller: Mutex::new(None),
		});
		let handles: Vec<JoinHandle<()>> = (0..count)
			.map(|index| {
				let shared = Arc::clone(&shared);
				// Named threads show up as the alley's lanes in profilers.
				std::thread::Builder::new()
					.name(format!("lane {index}"))
					.spawn(move || work(&shared, index))
					.expect("Lane-pool initialization failed. The operating system refused to start a worker thread.")
			})
			.collect();
		let threads = handles.iter().map(|handle| handle.thread().clone()).collect();

		Self {
			shared,
			threads,
			handles,
			dispatches: 0,
		}
	}

	/// Returns the number of persistent worker lanes.
	pub fn parallelism(&self) -> usize {
		self.threads.len()
	}

	/// Runs `job` once on each worker in `workers`, with that worker's index, while the calling thread runs `caller`,
	/// and returns once all of them finished.
	///
	/// Every worker finishes before a panic is returned, including one of `caller`, so `job` and whatever it borrows
	/// outlive their use. A worker's panic is returned before the caller's. Neither `caller` nor its result has to be
	/// `Send`.
	///
	/// # Panics
	///
	/// Panics if `workers` names a worker the pool does not have.
	#[allow(
		unsafe_code,
		reason = "Workers run a job that borrows from this call, which waits for all of them to finish."
	)]
	pub fn run<F, C>(&mut self, workers: Range<usize>, job: &F, caller: impl FnOnce() -> C) -> ThreadResult<C>
	where
		F: Fn(usize) + Sync,
	{
		assert!(
			workers.start <= workers.end && workers.end <= self.parallelism(),
			"Lane-pool dispatch rejected. The dispatch names workers {workers:?}, but the pool has {}. The most likely cause is a dispatch with more lanes than the alley's parallelism.",
			self.parallelism()
		);
		if workers.is_empty() {
			// Without workers there is nothing to hand over or wait for, so the dispatch touches none of their state.
			return catch_unwind(AssertUnwindSafe(caller));
		}
		let handed = &self.shared.workers[workers.clone()];
		self.dispatches += 1;
		let dispatch = self.dispatches;
		let job = Job::new(job);
		for worker in handed {
			// SAFETY: The worker finished its last job, which the last dispatch waited for, and reads this one only
			// once it sees the dispatch handed over below.
			unsafe { *worker.job.get() = job };
			// Sequentially consistent with the worker's check before it sleeps: either it sees this dispatch, or the
			// check below sees it asleep.
			worker.handed.store(dispatch, Ordering::SeqCst);
		}
		let mut woke = false;
		for (worker, thread) in handed.iter().zip(&self.threads[workers]) {
			if worker.sleeping.load(Ordering::SeqCst) {
				thread.unpark();
				woke = true;
			}
		}

		// The workers borrow from this call, so a panic of the caller waits for them before it leaves.
		let caller = catch_unwind(AssertUnwindSafe(caller));
		// Workers that just woke may wait for this thread's core, so it does not spin when it woke any.
		let waiting = if woke { Waiting::yielding() } else { Waiting::default() };
		wait_for(&self.shared, handed, dispatch, waiting);

		let mut panic = None;
		for worker in handed {
			// SAFETY: The worker finished, and writes its panic only while it runs a job.
			let payload = unsafe { (*worker.panic.get()).take() };
			if panic.is_none() {
				panic = payload;
			}
		}
		match panic {
			Some(payload) => Err(payload),
			None => caller,
		}
	}
}

impl Default for LanePool {
	fn default() -> Self {
		Self::new()
	}
}

impl Drop for LanePool {
	fn drop(&mut self) {
		// Every worker leaves its loop once it sees the pool shut down, whether it spins or sleeps.
		for worker in &self.shared.workers {
			worker.handed.store(SHUTDOWN, Ordering::Release);
		}
		for thread in &self.threads {
			thread.unpark();
		}
		for worker in self.handles.drain(..) {
			worker
				.join()
				.expect("Lane-pool shutdown failed. A worker panicked outside user-job handling.");
		}
	}
}

/// The handed-out dispatch that tells workers to leave.
const SHUTDOWN: u64 = u64::MAX;

/// How many times a worker checks for its next job before it yields, and a caller checks for finished workers
/// before it yields, a couple of microseconds.
///
/// The spin is short because a thread that spins keeps its core: when the machine has fewer free cores than
/// spinning threads, a worker that the system set aside runs again only once another thread stops spinning. A thread
/// that yields still checks again at once when no other thread wants its core, so dispatches that follow each other
/// closely find the workers awake either way, and handing them a job costs no system call.
const SPINS: u32 = 1 << 7;

/// How long a waiting thread stays awake, yielding its core to other threads, before it sleeps.
///
/// It outlasts the time a sleeping thread takes to wake, so that one slow wake does not make the next dispatch slow
/// too, and keep every dispatch after it waking threads that just fell asleep. It is a time rather than a count of
/// yields because a yield takes long when other threads want the core, and threads that stay awake that long take
/// time from the thread that runs the application.
const AWAKE: Duration = Duration::from_micros(100);

/// The `Shared` struct holds what the pool's threads hand each other.
struct Shared {
	workers: Box<[Worker]>,
	/// Whether the calling thread sleeps until the workers finish, so that a worker that finishes wakes it.
	caller_sleeping: Padded<AtomicBool>,
	/// The thread that sleeps until the workers finish. Only a sleeping caller and the workers that wake it lock it.
	caller: Mutex<Option<Thread>>,
}

/// The `Worker` struct holds one worker's side of a dispatch, on cache lines of its own.
#[repr(align(128))]
struct Worker {
	/// The last dispatch handed to the worker, or [`SHUTDOWN`].
	handed: AtomicU64,
	/// The last dispatch the worker finished.
	finished: AtomicU64,
	/// Whether the worker sleeps, so that a dispatch wakes it.
	sleeping: AtomicBool,
	/// The job of the last dispatch handed to the worker.
	job: UnsafeCell<Job>,
	/// What the worker's job panicked with in the last dispatch.
	panic: UnsafeCell<Option<Box<dyn Any + Send>>>,
}

impl Default for Worker {
	fn default() -> Self {
		Self {
			handed: AtomicU64::new(0),
			finished: AtomicU64::new(0),
			sleeping: AtomicBool::new(false),
			job: UnsafeCell::new(Job::NONE),
			panic: UnsafeCell::new(None),
		}
	}
}

// SAFETY: The caller writes a worker's job only before it hands the worker a dispatch, and the worker reads it only
// once it sees that dispatch and until it reports it finished, which the caller waits for before it writes the next
// one. The same handoff keeps the worker's write of its panic apart from the caller's read. Jobs are `Sync`, so the
// worker may call them from its thread, and panics are `Send`.
#[allow(unsafe_code, reason = "Dispatch handoffs keep the threads' accesses apart.")]
unsafe impl Sync for Worker {}

// SAFETY: A worker's job and panic only move between threads through the handoffs described for `Sync`.
#[allow(unsafe_code, reason = "Dispatch handoffs keep the threads' accesses apart.")]
unsafe impl Send for Worker {}

/// The `Job` struct points a worker at the job of a dispatch, which lives on the caller's stack until every worker
/// finished it.
#[derive(Clone, Copy)]
struct Job {
	data: *const (),
	call: unsafe fn(*const (), usize),
}

impl Job {
	/// The job of a worker that was never handed one.
	const NONE: Self = Self {
		data: std::ptr::null(),
		call: call_nothing,
	};

	fn new<F: Fn(usize) + Sync>(job: &F) -> Self {
		Self {
			data: std::ptr::from_ref(job).cast(),
			call: call_job::<F>,
		}
	}
}

/// Calls the job at `data`, an `F`, with a worker's index.
///
/// # Safety
///
/// `data` must point to a live `F`.
#[allow(unsafe_code, reason = "Workers call a job through the pointer the caller handed them.")]
unsafe fn call_job<F: Fn(usize) + Sync>(data: *const (), index: usize) {
	// SAFETY: The caller guarantees that `data` points to a live `F`, which is `Sync`, so this thread may share it.
	let job = unsafe { &*data.cast::<F>() };
	job(index);
}

/// Stands in for the job of a worker that was never handed one. Workers only call the job of a dispatch handed to
/// them, so this never runs.
fn call_nothing(_: *const (), _: usize) {}

/// Runs the jobs handed to worker `index` until the pool shuts down.
#[allow(unsafe_code, reason = "A worker reads the job the caller handed it.")]
fn work(shared: &Shared, index: usize) {
	let worker = &shared.workers[index];
	let mut last = 0;
	loop {
		let handed = next_dispatch(worker, last);
		if handed == SHUTDOWN {
			return;
		}
		last = handed;
		// SAFETY: The caller wrote the job before it handed the dispatch over, and keeps it alive and unchanged until
		// this worker reports it finished.
		let job = unsafe { *worker.job.get() };
		// SAFETY: The job's data lives until this worker reports it finished.
		if let Err(payload) = catch_unwind(AssertUnwindSafe(|| unsafe { (job.call)(job.data, index) })) {
			// SAFETY: The caller reads the panic only after this worker reports it finished.
			unsafe { *worker.panic.get() = Some(payload) };
		}
		// Sequentially consistent with the caller's check before it sleeps: either it sees this worker finished, or
		// this sees it asleep and wakes it.
		worker.finished.store(handed, Ordering::SeqCst);
		if shared.caller_sleeping.0.load(Ordering::SeqCst)
			&& let Some(caller) = &*shared.caller.lock().unwrap_or_else(PoisonError::into_inner)
		{
			caller.unpark();
		}
	}
}

/// Waits until `worker` is handed a dispatch after `last`, and returns it.
///
/// The worker checks for a while and yields for a while, so that dispatches that follow each other closely find it
/// awake, and then sleeps until a dispatch wakes it.
fn next_dispatch(worker: &Worker, last: u64) -> u64 {
	let mut waiting = Waiting::default();
	loop {
		let handed = worker.handed.load(Ordering::Acquire);
		if handed != last {
			return handed;
		}
		if waiting.pause() {
			break;
		}
	}
	// Sequentially consistent with a dispatch's handover: either it sees this worker asleep and wakes it, or the check
	// below sees the dispatch.
	worker.sleeping.store(true, Ordering::SeqCst);
	loop {
		let handed = worker.handed.load(Ordering::SeqCst);
		if handed != last {
			worker.sleeping.store(false, Ordering::Relaxed);
			return handed;
		}
		std::thread::park();
	}
}

/// Waits until every one of `workers` finished `dispatch`.
///
/// The caller checks for a while and yields for a while, as `waiting` paces it, and then sleeps until a worker that
/// finishes wakes it.
fn wait_for(shared: &Shared, workers: &[Worker], dispatch: u64, mut waiting: Waiting) {
	let finished = |order| workers.iter().all(|worker| worker.finished.load(order) == dispatch);
	loop {
		if finished(Ordering::Acquire) {
			return;
		}
		if waiting.pause() {
			break;
		}
	}

	{
		let mut caller = shared.caller.lock().unwrap_or_else(PoisonError::into_inner);
		let current = std::thread::current();
		if caller.as_ref().is_none_or(|caller| caller.id() != current.id()) {
			*caller = Some(current);
		}
	}
	// Sequentially consistent with a worker that finishes: either it sees this caller asleep and wakes it, or the check
	// below sees it finished.
	shared.caller_sleeping.0.store(true, Ordering::SeqCst);
	loop {
		if finished(Ordering::SeqCst) {
			break;
		}
		std::thread::park();
	}
	shared.caller_sleeping.0.store(false, Ordering::Relaxed);
}

/// The `Waiting` struct paces a thread that waits for another one: it spins for [`SPINS`] checks, then yields its
/// core until [`AWAKE`] passed, and then says that the thread should sleep.
#[derive(Default)]
struct Waiting {
	spins: u32,
	/// When the thread started to yield. Reading the clock only once spinning is over keeps quick waits cheap.
	yielding_since: Option<Instant>,
}

impl Waiting {
	/// Returns a wait that yields from the start, for a thread whose core other threads need now.
	fn yielding() -> Self {
		Self {
			spins: SPINS,
			yielding_since: None,
		}
	}

	/// Waits briefly before the thread checks again, and returns whether the thread should sleep instead.
	fn pause(&mut self) -> bool {
		if self.spins < SPINS {
			self.spins += 1;
			spin_loop();
			return false;
		}
		let since = *self.yielding_since.get_or_insert_with(Instant::now);
		if since.elapsed() >= AWAKE {
			return true;
		}
		yield_now();
		false
	}
}

/// The `Padded` struct keeps a value on cache lines of its own, so that threads that write it do not slow down
/// threads that use what lies next to it.
#[repr(align(128))]
struct Padded<T>(T);

use std::{
	any::Any,
	cell::UnsafeCell,
	hint::spin_loop,
	ops::Range,
	panic::{AssertUnwindSafe, catch_unwind},
	sync::{
		Arc, Mutex, PoisonError,
		atomic::{AtomicBool, AtomicU64, Ordering},
	},
	thread::{JoinHandle, Result as ThreadResult, Thread, yield_now},
	time::{Duration, Instant},
};

#[cfg(test)]
mod tests {
	use std::{
		panic::{AssertUnwindSafe, catch_unwind},
		sync::{
			Barrier,
			atomic::{AtomicUsize, Ordering},
		},
		thread,
		time::Duration,
	};

	use super::LanePool;

	#[test]
	fn run_gives_every_worker_its_index_and_accepts_call_local_borrows() {
		let mut pool = LanePool::with_parallelism(4);
		let values = [1, 2, 3, 4].map(AtomicUsize::new);

		pool.run(
			0..4,
			&|index| {
				values[index].fetch_add(index * 10, Ordering::Relaxed);
			},
			|| (),
		)
		.unwrap();

		assert_eq!(values.map(AtomicUsize::into_inner), [1, 12, 23, 34]);
	}

	#[test]
	fn run_overlaps_the_caller_with_every_worker() {
		let mut pool = LanePool::with_parallelism(3);
		// The barrier opens only when every worker and the caller wait at it together.
		let barrier = Barrier::new(4);
		let caller_thread = thread::current().id();
		let local = std::rc::Rc::new(7);

		let returned = pool
			.run(
				0..3,
				&|_| {
					barrier.wait();
					assert_ne!(thread::current().id(), caller_thread);
				},
				|| {
					barrier.wait();
					local.clone()
				},
			)
			.unwrap();

		assert_eq!(*returned, 7);
	}

	#[test]
	fn run_leaves_the_workers_outside_its_range_alone() {
		let mut pool = LanePool::with_parallelism(4);
		let ran = [0, 0, 0, 0].map(AtomicUsize::new);

		pool.run(
			1..3,
			&|index| {
				ran[index].fetch_add(1, Ordering::Relaxed);
			},
			|| (),
		)
		.unwrap();

		assert_eq!(ran.map(AtomicUsize::into_inner), [0, 1, 1, 0]);
	}

	#[test]
	fn run_rejects_workers_the_pool_does_not_have() {
		let mut pool = LanePool::with_parallelism(2);

		let panic = catch_unwind(AssertUnwindSafe(|| pool.run(0..3, &|_| {}, || ()))).expect_err("excess workers should panic");
		let message = panic.downcast_ref::<String>().map(String::as_str).unwrap_or_default();

		assert!(message.starts_with("Lane-pool dispatch rejected."));
		assert_eq!(pool.run(0..2, &|_| {}, || 5).unwrap(), 5);
	}

	#[test]
	fn run_returns_a_worker_panic_after_waiting_and_keeps_workers_available() {
		let mut pool = LanePool::with_parallelism(4);
		let completed = AtomicUsize::new(0);

		let result = pool.run(
			0..4,
			&|index| {
				if index == 0 {
					panic!("expected captured worker panic");
				}
				thread::sleep(Duration::from_millis(10));
				completed.fetch_add(1, Ordering::Relaxed);
			},
			|| (),
		);

		assert!(result.is_err());
		assert_eq!(completed.load(Ordering::Relaxed), 3);
		assert_eq!(pool.run(0..4, &|_| {}, || 42).unwrap(), 42);
	}

	#[test]
	fn run_waits_for_the_workers_after_the_caller_panics() {
		let mut pool = LanePool::with_parallelism(2);
		let completed = AtomicUsize::new(0);

		let result = pool.run(
			0..2,
			&|_| {
				thread::sleep(Duration::from_millis(10));
				completed.fetch_add(1, Ordering::Relaxed);
			},
			|| panic!("expected caller panic"),
		);

		assert!(result.is_err());
		assert_eq!(completed.load(Ordering::Relaxed), 2);
	}

	#[test]
	fn workers_that_went_to_sleep_wake_for_the_next_dispatch() {
		let mut pool = LanePool::with_parallelism(3);
		let ran = AtomicUsize::new(0);

		for _ in 0..3 {
			// Long enough for every worker to stop checking for a job and sleep.
			thread::sleep(Duration::from_millis(20));
			pool.run(
				0..3,
				&|_| {
					ran.fetch_add(1, Ordering::Relaxed);
				},
				|| (),
			)
			.unwrap();
		}

		assert_eq!(ran.load(Ordering::Relaxed), 9);
	}

	#[test]
	fn a_caller_that_sleeps_wakes_when_the_workers_finish() {
		let mut pool = LanePool::with_parallelism(2);

		// The workers take long enough for the caller to stop checking and sleep.
		let returned = pool.run(0..2, &|_| thread::sleep(Duration::from_millis(20)), || 3).unwrap();

		assert_eq!(returned, 3);
	}
}
