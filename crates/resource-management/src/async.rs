use std::future::Future;
use std::pin::Pin;

pub use compio::fs::{File, read, write};
pub use compio::process::Command;
pub use compio::runtime::Runtime as Executor;
pub use compio::runtime::spawn;
pub use compio::runtime::spawn_blocking;
pub use compio::test;
pub use spawn_blocking as offload;
pub use spawn_blocking as spawn_cpu_task;

pub fn future<'a, T, F>(f: F) -> BoxedFuture<'a, T>
where
	F: Future<Output = T> + 'a,
{
	Box::pin(f)
}

pub type BoxedFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Returns control to the executor once, so futures joined with the caller make progress.
///
/// Await it between chunks of synchronous work inside a joined future, such as between the primitives of a mesh
/// while its materials bake, so the other futures can keep dispatching and collecting their work.
pub(crate) async fn yield_now() {
	let mut yielded = false;
	std::future::poll_fn(|context| {
		if yielded {
			return std::task::Poll::Ready(());
		}
		yielded = true;
		context.waker().wake_by_ref();
		std::task::Poll::Pending
	})
	.await
}
