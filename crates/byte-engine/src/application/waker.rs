//! Wakes the application loop while it waits for window events.
//!
//! With `render-on-demand`, an idle graphics application waits inside the platform event queue. State that
//! changes without a window event must end that wait through the application's [`LoopWaker`]: UI tasks woken by
//! other threads, inspector requests, and application worker threads.

use std::sync::{
	Arc, Mutex,
	atomic::{AtomicBool, Ordering},
};

/// The `LoopWaker` struct makes a waiting application loop run its next tick.
///
/// Take one with `GraphicsApplication::waker`, hand it to whatever changes state the loop cannot observe through
/// window events, and call [`Self::wake`] from any thread afterwards. Wakes are coalesced, and waking a loop that
/// is not waiting only costs two atomic operations. A `LoopWaker` also converts into a [`std::task::Waker`] for
/// futures that must run inside the loop's own tick.
#[derive(Clone, Default)]
pub struct LoopWaker(Arc<LoopWakerState>);

/// The state one application shares with every waker it handed out.
#[derive(Default)]
struct LoopWakerState {
	/// Whether the loop is between deciding to wait and returning from the wait.
	waiting: AtomicBool,
	/// Whether something woke the loop since it last started deciding to wait.
	pending: AtomicBool,
	/// Interrupts the platform wait; set once the application has a window system connection.
	platform: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl LoopWaker {
	/// Makes the application loop run its next tick without waiting for an event.
	pub fn wake(&self) {
		self.0.pending.store(true, Ordering::SeqCst);
		if self.0.waiting.load(Ordering::SeqCst)
			&& let Some(wake) = self.0.platform.lock().unwrap_or_else(|error| error.into_inner()).as_ref()
		{
			wake();
		}
	}

	/// Sets the function that interrupts the platform wait.
	#[cfg(feature = "headed")]
	pub(crate) fn set_platform_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
		*self.0.platform.lock().unwrap_or_else(|error| error.into_inner()) = Some(Box::new(wake));
	}

	/// Marks the loop as about to wait and reports whether a wake arrived since the last wait.
	///
	/// Call this before checking for pending work, then wait only when neither found any. A wake that races with
	/// the check either shows up here or interrupts the wait, so it is never lost. Pair it with [`Self::end_wait`].
	#[cfg(feature = "headed")]
	pub(crate) fn begin_wait(&self) -> bool {
		self.0.waiting.store(true, Ordering::SeqCst);
		self.0.pending.swap(false, Ordering::SeqCst)
	}

	/// Marks the loop as running again.
	#[cfg(feature = "headed")]
	pub(crate) fn end_wait(&self) {
		self.0.waiting.store(false, Ordering::SeqCst);
	}
}

impl std::task::Wake for LoopWakerState {
	fn wake(self: Arc<Self>) {
		self.wake_by_ref();
	}

	fn wake_by_ref(self: &Arc<Self>) {
		LoopWaker(Arc::clone(self)).wake();
	}
}

impl From<LoopWaker> for std::task::Waker {
	fn from(waker: LoopWaker) -> Self {
		Self::from(waker.0)
	}
}

#[cfg(all(test, feature = "headed"))]
mod tests {
	use super::*;

	#[test]
	fn a_wake_before_the_wait_is_reported_once() {
		let waker = LoopWaker::default();

		waker.wake();

		assert!(waker.begin_wait(), "A wake before the wait was lost.");
		waker.end_wait();
		assert!(!waker.begin_wait(), "A consumed wake was reported again.");
		waker.end_wait();
	}

	#[test]
	fn clones_and_task_wakers_share_one_application_state() {
		let waker = LoopWaker::default();
		let task_waker = std::task::Waker::from(waker.clone());

		task_waker.wake();

		assert!(waker.begin_wait(), "A task waker did not reach the application.");
		waker.end_wait();
	}
}
