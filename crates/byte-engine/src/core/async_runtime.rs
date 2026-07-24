//! Small scheduling utilities shared by application-runtime workers.

/// Yields once so a ready loop cannot consume the application's whole per-tick
/// async-work budget.
pub(crate) async fn yield_now() {
	let mut yielded = false;
	std::future::poll_fn(move |context| {
		if yielded {
			std::task::Poll::Ready(())
		} else {
			yielded = true;
			context.waker().wake_by_ref();
			std::task::Poll::Pending
		}
	})
	.await;
}
