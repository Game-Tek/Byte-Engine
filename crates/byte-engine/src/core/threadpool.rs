/// The `ScopedThreadPool` struct provides blocking execution on persistent scoped workers.
pub struct ScopedThreadPool<'scope> {
	sender: Option<Sender<Job<'scope>>>,
	parallelism: usize,
}

pub type Scope<'scope, 'b> = std::thread::Scope<'scope, 'b>;

impl<'scope> ScopedThreadPool<'scope> {
	pub fn with_parallelism(scope: &'scope Scope<'scope, '_>, count: usize) -> Self {
		assert!(
			count > 0,
			"Thread-pool parallelism must be greater than zero. No worker threads would be available to execute jobs."
		);

		let (sender, receiver) = kanal::unbounded::<Job<'scope>>();

		for _ in 0..count {
			let receiver = receiver.clone();

			scope.spawn(move || {
				while let Ok(job) = receiver.recv() {
					job();
				}
			});
		}

		Self {
			sender: Some(sender),
			parallelism: count,
		}
	}

	/// Runs `f` on one worker and returns after it finishes.
	pub fn execute<'job, F>(&'job self, f: F)
	where
		F: FnOnce() + Send + 'job,
	{
		self.execute_many(std::iter::once(f));
	}

	/// Runs one copy of `f` per logical worker and returns after every copy finishes.
	pub fn execute_on_all<'job, F>(&'job self, f: F)
	where
		F: FnOnce(usize) + Send + Clone + 'job,
	{
		let jobs = (0..self.parallelism).map(|idx| {
			let f = f.clone();
			move || f(idx)
		});

		self.execute_many(jobs);
	}

	/// Runs all `jobs` concurrently and returns after every submitted job finishes.
	#[allow(
		unsafe_code,
		reason = "Blocking completion keeps call-borrowed jobs alive while persistent workers execute them."
	)]
	pub fn execute_many<'job, I, F>(&'job self, jobs: I)
	where
		I: IntoIterator<Item = F>,
		F: FnOnce() + Send + 'job,
	{
		let (completion_sender, completion_receiver) = kanal::unbounded::<ThreadResult<()>>();
		let mut submitted = 0;

		// Catch iterator, cloning, and queue failures so already-submitted jobs remain borrowed
		// until they finish, even when submission cannot complete.
		let submission_result = catch_unwind(AssertUnwindSafe(|| {
			for job in jobs {
				let completion_sender = completion_sender.clone();
				let job: Job<'job> = Box::new(move || {
					let result = catch_unwind(AssertUnwindSafe(job));
					completion_sender
						.send(result)
						.expect("Thread-pool completion failed. The submitting thread stopped waiting for the job.");
				});

				// SAFETY: This method waits for every successfully submitted job before it
				// returns or resumes a submission panic, so `job` cannot outlive `'job`.
				let job: Job<'scope> = unsafe { erase_job_lifetime(job) };
				self.sender
					.as_ref()
					.expect("Thread pool is unavailable. Its job sender was removed during shutdown.")
					.send(job)
					.expect("Thread-pool submission failed. All worker receivers have disconnected.");
				submitted += 1;
			}
		}));
		drop(completion_sender);

		let mut job_panic = None;
		for _ in 0..submitted {
			match completion_receiver
				.recv()
				.expect("Thread-pool completion failed. A worker dropped a job without reporting completion.")
			{
				Ok(()) => {}
				Err(payload) if job_panic.is_none() => job_panic = Some(payload),
				Err(_) => {}
			}
		}

		if let Err(payload) = submission_result {
			resume_unwind(payload);
		}
		if let Some(payload) = job_panic {
			resume_unwind(payload);
		}
	}

	pub fn parallelism(&self) -> usize {
		self.parallelism
	}
}

impl<'scope> Drop for ScopedThreadPool<'scope> {
	fn drop(&mut self) {
		self.sender.take();
	}
}

/// The `LanePool` struct provides reusable owned workers for blocking collective dispatches.
pub struct LanePool {
	senders: Vec<Sender<LaneJob>>,
	workers: Vec<JoinHandle<()>>,
}

impl LanePool {
	/// Creates one worker lane for each hardware thread available to this process.
	///
	/// Call [`Self::dispatch_all`] or [`Self::dispatch_many`] to run a blocking batch.
	pub fn new() -> Self {
		let parallelism = std::thread::available_parallelism()
			.expect("Lane-pool initialization failed. The operating system did not report available parallelism.")
			.get();
		Self::with_parallelism(parallelism)
	}

	/// Creates an owned pool with `count` persistent worker lanes.
	///
	/// Call [`Self::dispatch_all`] or [`Self::dispatch_many`] to run a blocking batch.
	pub fn with_parallelism(count: usize) -> Self {
		assert!(
			count > 0,
			"Lane-pool initialization failed. The requested parallelism is zero, so no worker could execute jobs."
		);

		let mut senders = Vec::with_capacity(count);
		let mut workers = Vec::with_capacity(count);

		// Give each worker an independent mailbox so collective dispatch can address every lane.
		for _ in 0..count {
			let (sender, receiver) = kanal::unbounded::<LaneJob>();
			let worker = std::thread::spawn(move || {
				while let Ok(job) = receiver.recv() {
					job();
				}
			});
			senders.push(sender);
			workers.push(worker);
		}

		Self { senders, workers }
	}

	/// Runs one indexed copy of `f` on every lane and returns after all copies finish.
	///
	/// Mutable access prevents overlapping or nested dispatches on this pool. Gang-scheduled lane
	/// collectives require exclusive access to the worker set and could otherwise deadlock.
	pub fn dispatch_all<'job, F>(&'job mut self, f: F)
	where
		F: FnOnce(usize) + Clone + Send + 'job,
	{
		if let Err(payload) = self.try_dispatch_all(f) {
			resume_unwind(payload);
		}
	}

	/// Runs all `jobs` across distinct worker lanes and returns after every submitted job finishes.
	///
	/// The iterator must yield no more than [`Self::parallelism`] jobs for one gang dispatch.
	/// Mutable access prevents overlapping or nested dispatches on this pool because lane
	/// collectives need exclusive access to its worker set and could otherwise deadlock.
	pub fn dispatch_many<'job, I, F>(&'job mut self, jobs: I)
	where
		I: IntoIterator<Item = F>,
		F: FnOnce() + Send + 'job,
	{
		if let Err(payload) = self.try_dispatch_many(jobs) {
			resume_unwind(payload);
		}
	}

	/// Runs one indexed copy of `f` on every lane and returns ordered values or a captured panic.
	///
	/// Mutable access prevents overlapping or nested dispatches on this pool. Gang-scheduled lane
	/// collectives require exclusive access to the worker set and could otherwise deadlock.
	pub fn try_dispatch_all<'job, F, R>(&'job mut self, f: F) -> ThreadResult<Vec<R>>
	where
		F: FnOnce(usize) -> R + Clone + Send + 'job,
		R: Send + 'job,
	{
		let jobs = (0..self.parallelism()).map(|lane| {
			let f = f.clone();
			move || f(lane)
		});
		self.try_dispatch_many(jobs)
	}

	/// Runs all `jobs` and returns values in submission order or the first captured panic.
	///
	/// The iterator must yield no more than [`Self::parallelism`] jobs for one gang dispatch.
	/// Mutable access prevents overlapping or nested dispatches on this pool because lane
	/// collectives need exclusive access to its worker set and could otherwise deadlock.
	pub fn try_dispatch_many<'job, I, F, R>(&'job mut self, jobs: I) -> ThreadResult<Vec<R>>
	where
		I: IntoIterator<Item = F>,
		F: FnOnce() -> R + Send + 'job,
		R: Send + 'job,
	{
		self.try_dispatch_many_inner(jobs)
	}

	/// Returns the number of persistent worker lanes.
	pub fn parallelism(&self) -> usize {
		self.senders.len()
	}

	/// Submits one batch and waits for every accepted job before returning.
	#[allow(
		unsafe_code,
		reason = "Blocking completion keeps call-borrowed jobs alive in static worker mailboxes."
	)]
	fn try_dispatch_many_inner<'job, I, F, R>(&'job mut self, jobs: I) -> ThreadResult<Vec<R>>
	where
		I: IntoIterator<Item = F>,
		F: FnOnce() -> R + Send + 'job,
		R: Send + 'job,
	{
		let (completion_sender, completion_receiver) = kanal::unbounded::<(usize, ThreadResult<R>)>();
		let mut submitted = 0;

		// Catch iteration, capacity, cloning, lifetime erasure, and mailbox failures so prior jobs
		// stay borrowed until their completion messages arrive.
		let submission_result = catch_unwind(AssertUnwindSafe(|| {
			for job in jobs {
				assert!(
					submitted < self.parallelism(),
					"Lane-pool dispatch rejected. The batch contains more jobs than worker lanes; split it into gangs of at most {} jobs.",
					self.parallelism()
				);
				let completion_sender = completion_sender.clone();
				let job_index = submitted;
				let job: Job<'job> = Box::new(move || {
					let result = catch_unwind(AssertUnwindSafe(job));
					let _ = completion_sender.send((job_index, result));
				});

				// SAFETY: This method receives one completion for every accepted job before it
				// returns a value or panic payload, so the job cannot outlive `'job`.
				let job = unsafe { erase_lane_job_lifetime(job) };
				self.senders[submitted]
					.send(job)
					.expect("Lane-pool submission failed. The selected worker mailbox has disconnected.");
				submitted += 1;
			}
		}));
		drop(completion_sender);

		let mut values = std::iter::repeat_with(|| None).take(submitted).collect::<Vec<_>>();
		let mut job_panic = None;
		for _ in 0..submitted {
			let (job_index, result) = completion_receiver
				.recv()
				.expect("Lane-pool completion failed. A worker dropped an accepted job without reporting completion.");
			match result {
				Ok(value) => values[job_index] = Some(value),
				Err(payload) if job_panic.is_none() => job_panic = Some(payload),
				Err(_) => {}
			}
		}

		if let Err(payload) = submission_result {
			return Err(payload);
		}
		if let Some(payload) = job_panic {
			return Err(payload);
		}

		// Every accepted job produced one successful value when no panic payload was captured.
		Ok(values
			.into_iter()
			.map(|value| value.expect("Lane-pool completion failed. A successful job result is missing."))
			.collect())
	}
}

impl Default for LanePool {
	fn default() -> Self {
		Self::new()
	}
}

impl Drop for LanePool {
	fn drop(&mut self) {
		// Close every mailbox before joining so idle workers can leave their receive loops.
		self.senders.clear();
		for worker in self.workers.drain(..) {
			worker
				.join()
				.expect("Lane-pool shutdown failed. A worker panicked outside user-job handling.");
		}
	}
}

type Job<'scope> = Box<dyn FnOnce() + Send + 'scope>;
type LaneJob = Job<'static>;

/// Extends a job's erased lifetime while a blocking submission method owns its real lifetime.
///
/// # Safety
///
/// The caller must not return or unwind beyond the job's original lifetime until the job has
/// finished and dropped all values borrowed for that lifetime.
#[allow(
	unsafe_code,
	reason = "The persistent queue requires lifetime erasure guarded by blocking completion."
)]
unsafe fn erase_job_lifetime<'job, 'scope>(job: Job<'job>) -> Job<'scope> {
	// SAFETY: The caller upholds the lifetime requirement above.
	unsafe { std::mem::transmute(job) }
}

/// Erases a lane job's call-borrowed lifetime for storage in an owned worker mailbox.
///
/// # Safety
///
/// The caller must block until the worker has run and dropped the job before ending the job's
/// original lifetime, including when submission or another job panics.
#[allow(
	unsafe_code,
	reason = "Blocking lane dispatch requires a call-borrowed job in a static worker mailbox."
)]
unsafe fn erase_lane_job_lifetime<'job>(job: Job<'job>) -> LaneJob {
	// SAFETY: The caller upholds the blocking completion requirement above.
	unsafe { std::mem::transmute(job) }
}

use std::{
	panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
	thread::{JoinHandle, Result as ThreadResult},
};

use kanal::Sender;

#[cfg(test)]
mod tests {
	use std::{
		panic::{AssertUnwindSafe, catch_unwind},
		sync::{
			Barrier, Mutex,
			atomic::{AtomicUsize, Ordering},
		},
		thread::{self, scope},
		time::Duration,
	};

	use super::{LanePool, ScopedThreadPool};

	#[test]
	fn execute_blocks_and_accepts_call_local_borrows() {
		scope(|scope| {
			let pool = ScopedThreadPool::with_parallelism(scope, 2);
			let mut value = 0;

			pool.execute(|| value = 42);

			assert_eq!(value, 42);
		});
	}

	#[test]
	fn execute_on_all_submits_every_job_before_waiting() {
		scope(|scope| {
			let pool = ScopedThreadPool::with_parallelism(scope, 4);
			let barrier = Barrier::new(pool.parallelism());

			pool.execute_on_all(|_| {
				barrier.wait();
			});
		});
	}

	#[test]
	fn execute_propagates_panics_and_keeps_workers_available() {
		scope(|scope| {
			let pool = ScopedThreadPool::with_parallelism(scope, 1);
			let panic = catch_unwind(AssertUnwindSafe(|| {
				pool.execute(|| panic!("expected worker panic"));
			}));

			assert!(panic.is_err());

			let mut completed = false;
			pool.execute(|| completed = true);

			assert!(completed);
		});
	}

	#[test]
	fn lane_dispatch_many_blocks_and_accepts_call_local_borrows() {
		let mut pool = LanePool::with_parallelism(4);
		let mut values = [1, 2, 3, 4];

		pool.dispatch_many(values.iter_mut().map(|value| move || *value *= 2));

		assert_eq!(values, [2, 4, 6, 8]);
	}

	#[test]
	fn lane_try_dispatch_returns_results_in_submission_order() {
		let mut pool = LanePool::with_parallelism(4);
		let next_to_finish = AtomicUsize::new(pool.parallelism() - 1);

		let values = pool
			.try_dispatch_all(|lane| {
				while next_to_finish.load(Ordering::Acquire) != lane {
					thread::yield_now();
				}
				next_to_finish.fetch_sub(1, Ordering::Release);
				lane * 10
			})
			.expect("lane jobs should succeed");

		assert_eq!(values, [0, 10, 20, 30]);
	}

	#[test]
	fn lane_dispatch_many_rejects_excess_jobs_and_keeps_workers_available() {
		let mut pool = LanePool::with_parallelism(2);
		let completed = Mutex::new(0);

		let panic = catch_unwind(AssertUnwindSafe(|| {
			pool.dispatch_many((0..3).map(|_| {
				|| {
					*completed.lock().unwrap() += 1;
				}
			}));
		}));

		let payload = panic.expect_err("excess jobs should panic");
		let message = payload
			.downcast_ref::<&str>()
			.copied()
			.or_else(|| payload.downcast_ref::<String>().map(String::as_str));

		assert_eq!(
			message,
			Some(
				"Lane-pool dispatch rejected. The batch contains more jobs than worker lanes; split it into gangs of at most 2 jobs."
			)
		);
		assert_eq!(*completed.lock().unwrap(), 2);

		pool.dispatch_many(std::iter::once(|| {
			*completed.lock().unwrap() += 1;
		}));

		assert_eq!(*completed.lock().unwrap(), 3);
	}

	#[test]
	fn lane_dispatch_all_submits_every_job_before_waiting() {
		let mut pool = LanePool::with_parallelism(4);
		let barrier = Barrier::new(pool.parallelism());

		pool.dispatch_all(|_| {
			barrier.wait();
		});
	}

	#[test]
	fn lane_dispatch_propagates_panics_and_keeps_workers_available() {
		let mut pool = LanePool::with_parallelism(2);
		let panic = catch_unwind(AssertUnwindSafe(|| {
			pool.dispatch_all(|lane| {
				if lane == 0 {
					panic!("expected lane panic");
				}
			});
		}));

		assert!(panic.is_err());

		let mut completed = false;
		pool.dispatch_many(std::iter::once(|| completed = true));

		assert!(completed);
	}

	#[test]
	fn lane_try_dispatch_returns_panic_after_waiting_and_keeps_workers_available() {
		let mut pool = LanePool::with_parallelism(4);
		let completed = AtomicUsize::new(0);

		let result = pool.try_dispatch_all(|lane| {
			if lane == 0 {
				panic!("expected captured lane panic");
			}
			thread::sleep(Duration::from_millis(10));
			completed.fetch_add(1, Ordering::Relaxed);
			lane
		});

		assert!(result.is_err());
		assert_eq!(completed.load(Ordering::Relaxed), pool.parallelism() - 1);
		assert_eq!(
			pool.try_dispatch_many(std::iter::once(|| 42))
				.expect("pool should remain reusable"),
			[42]
		);
	}
}
