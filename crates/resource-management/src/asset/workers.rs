//! The worker threads an [`AssetManager`] bakes on, which live exactly as long as the call that opens them.
//!
//! [`AssetManager::with_workers`] starts the threads inside a [`std::thread::scope`] and returns once every task
//! queued on them has finished, so a task may borrow the manager and nothing a bake started is still running when the
//! manager is dropped.

/// The `AssetWorkers` struct is a handle to the worker threads of one [`AssetManager::with_workers`] call, through
/// which bakes run on every core.
///
/// The body of [`AssetManager::with_workers`] receives one and asks it for bakes, such as with [`Self::bake`]. Every
/// task holds its own handle, so a task queues its dependencies on the same threads. The threads stop once no handle
/// is left, which is when the body returned and every task finished.
pub struct AssetWorkers<'s> {
	pub(in crate::asset) manager: &'s AssetManager,
	tasks: flume::Sender<Task<'s>>,
}

/// One queued piece of work. The closure crosses to a worker thread, which builds the future there, so the future
/// itself need not be [`Send`].
type Task<'s> = Box<dyn FnOnce() -> BoxedFuture<'s, ()> + Send + 's>;

impl Clone for AssetWorkers<'_> {
	fn clone(&self) -> Self {
		Self {
			manager: self.manager,
			tasks: self.tasks.clone(),
		}
	}
}

impl<'s> AssetWorkers<'s> {
	/// Returns the manager these workers bake for.
	pub fn manager(&self) -> &'s AssetManager {
		self.manager
	}

	/// Runs `task` on a worker thread and returns the receiver of its result.
	///
	/// The task gets a handle of its own, so it can queue more work. Dropping the receiver doesn't stop the task: it
	/// runs to its end before [`AssetManager::with_workers`] returns.
	pub(crate) fn spawn<F, R>(&self, task: impl FnOnce(AssetWorkers<'s>) -> F + Send + 's) -> oneshot::Receiver<R>
	where
		F: Future<Output = R> + 's,
		R: Send + 's,
	{
		let (sender, receiver) = oneshot::channel();
		let workers = self.clone();
		let task: Task<'s> = Box::new(move || {
			Box::pin(async move {
				let _ = sender.send(task(workers).await);
			})
		});
		// The scope keeps the receiving side until every handle is gone, and this handle is alive, so the send succeeds.
		let _ = self.tasks.send(task);
		receiver
	}
}

impl AssetManager {
	/// Starts this manager's worker threads, runs `body` with a handle to them, and returns once `body` and every task
	/// it started have finished.
	///
	/// Every bake runs inside such a call, such as `manager.with_workers(async |workers| workers.bake("mesh.gltf").await)`.
	/// `body` runs on the calling thread under a runtime of its own, while the bakes it asks for run on the workers.
	/// A tool that bakes once, like BELD, opens one call for the whole run; an application keeps one open on a thread of
	/// its own for as long as it serves requests. After the call returns, nothing borrows the manager, so dropping it
	/// closes its stores.
	pub fn with_workers<T>(&self, body: impl AsyncFnOnce(&AssetWorkers<'_>) -> T) -> T {
		let (sender, receiver) = flume::unbounded::<Task<'_>>();

		// Every runtime here shares one pool for blocking file operations, as the threads of one dispatcher would.
		let mut proactor = compio::driver::ProactorBuilder::new();
		proactor.force_reuse_thread_pool();

		std::thread::scope(|threads| {
			for index in 0..self.worker_count.get() {
				let (receiver, proactor) = (&receiver, proactor.clone());
				std::thread::Builder::new()
					.name(format!("Asset Worker {index}"))
					.spawn_scoped(threads, move || work(receiver, proactor))
					.expect("Failed to start an asset worker. The most likely cause is that the system ran out of threads.");
			}

			let runtime = compio::runtime::Runtime::builder().with_proactor(proactor.clone()).build().expect(
				"Failed to start the asset bake runtime. The most likely cause is that the platform I/O driver could not be initialized.",
			);
			let workers = AssetWorkers {
				manager: self,
				tasks: sender,
			};
			let result = runtime.block_on(body(&workers));

			// The workers stop once the last handle is gone; the body's goes first, then each task's as it finishes.
			drop(workers);

			result
		})
	}
}

/// Runs queued tasks on this thread until no handle to the workers is left, then finishes the tasks still running.
///
/// Every task runs to its end on the thread that took it, interleaved with the others that thread took, so a thread
/// waiting on I/O for one task takes the next.
fn work(tasks: &flume::Receiver<Task<'_>>, proactor: compio::driver::ProactorBuilder) {
	let runtime = compio::runtime::Runtime::builder().with_proactor(proactor).build().expect(
		"Failed to start an asset worker runtime. The most likely cause is that the platform I/O driver could not be initialized.",
	);

	runtime.block_on(async {
		let mut running = FuturesUnordered::new();

		loop {
			// Polling the running tasks first lets a finished one end before the thread takes more work.
			select_biased! {
				() = running.select_next_some() => {}
				task = tasks.recv_async().fuse() => match task {
					Ok(task) => running.push(task()),
					Err(_) => break,
				},
			}
		}

		// No handle is left, so no running task can queue more; the ones still here only finish.
		while running.next().await.is_some() {}
	});
}

use std::future::Future;

use utils::r#async::{FutureExt as _, StreamExt as _, oneshot, select_biased, stream::FuturesUnordered};

use super::manager::AssetManager;
use crate::r#async::BoxedFuture;
