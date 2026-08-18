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

type Job<'scope> = Box<dyn FnOnce() + Send + 'scope>;

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

use std::{
	panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
	thread::Result as ThreadResult,
};

use kanal::Sender;

#[cfg(test)]
mod tests {
	use std::{
		panic::{catch_unwind, AssertUnwindSafe},
		sync::Barrier,
		thread::scope,
	};

	use super::ScopedThreadPool;

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
}
