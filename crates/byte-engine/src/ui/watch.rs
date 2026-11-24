//! A shared latest-value cell whose changes wake the UI components watching it.
//!
//! A [`Watch`] holds one value and a version that every write advances. A component that
//! [`subscribe`](Watch::subscribe)s awaits [`Subscriber::changed`], which resolves at once the first time, then
//! sleeps until the version moves past the one it last read, and hands out the current value. Writes that land between two reads collapse into one wake,
//! so a producer running far faster than the display never queues work for the UI; the component only ever sees
//! the newest state at its next poll. Writers may live on any thread, and a write from another thread reaches the
//! host through the engine's task wakers, so nothing polls while the value is idle.

#![expect(
	clippy::disallowed_types,
	reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"
)]

use std::{
	future::Future,
	pin::Pin,
	sync::{
		Arc,
		atomic::{AtomicU64, Ordering},
	},
	task::{Context, Poll, Waker},
};

use utils::{r#async::FusedFuture, sync::Mutex};

struct Inner<T> {
	value: Mutex<T>,
	/// Advanced by every write; read without the value lock on the wait's fast path.
	version: AtomicU64,
	wakers: Mutex<Vec<Waker>>,
}

/// The `Watch` struct shares a value between its writers and the components waiting for it to change.
pub struct Watch<T>(Arc<Inner<T>>);

impl<T> Clone for Watch<T> {
	fn clone(&self) -> Self {
		Self(Arc::clone(&self.0))
	}
}

impl<T: Default> Default for Watch<T> {
	fn default() -> Self {
		Self::new(T::default())
	}
}

impl<T> Watch<T> {
	pub fn new(value: T) -> Self {
		Self(Arc::new(Inner {
			value: Mutex::new(value),
			version: AtomicU64::new(0),
			wakers: Mutex::new(Vec::new()),
		}))
	}

	/// Mutates the value in place, then wakes every subscriber waiting on it.
	pub fn update<R>(&self, update: impl FnOnce(&mut T) -> R) -> R {
		let result = update(&mut self.0.value.lock());
		self.0.version.fetch_add(1, Ordering::Release);
		// Wakers are taken out of the lock before waking, so a woken task may subscribe again at once.
		let wakers = std::mem::take(&mut *self.0.wakers.lock());
		for waker in wakers {
			waker.wake();
		}
		result
	}

	pub fn set(&self, value: T) {
		self.update(|current| *current = value);
	}

	/// Reads the value without waiting or affecting any subscriber.
	pub fn read(&self) -> utils::sync::MutexGuard<'_, T> {
		self.0.value.lock()
	}

	/// Starts watching with nothing seen, so the first wait resolves at once with the current value.
	///
	/// Await [`Subscriber::changed`] in a loop, or hand the subscriber to [`crate::ui::Context::reactive`].
	pub fn subscribe(&self) -> Subscriber<T> {
		Subscriber {
			watch: self.clone(),
			// One behind the current version, which no write can bring back before the first read.
			seen: self.0.version.load(Ordering::Acquire).wrapping_sub(1),
		}
	}
}

/// The `Subscriber` struct tracks which version of a [`Watch`] one component has already seen.
pub struct Subscriber<T> {
	watch: Watch<T>,
	seen: u64,
}

impl<T> Subscriber<T> {
	/// Resolves with the value once it has changed since the last read through this subscriber.
	pub fn changed(&mut self) -> Changed<'_, T> {
		Changed { subscriber: Some(self) }
	}

	/// Reads the current value and marks it seen, so the next wait only resolves on a later write.
	pub fn read(&mut self) -> utils::sync::MutexGuard<'_, T> {
		self.seen = self.watch.0.version.load(Ordering::Acquire);
		self.watch.0.value.lock()
	}

	/// Resolves once the version moved past the one last read, without reading or marking it seen. Otherwise it
	/// registers `cx`'s waker with the watch.
	fn poll_unseen(&self, cx: &mut Context<'_>) -> Poll<()> {
		let inner = &self.watch.0;
		if inner.version.load(Ordering::Acquire) != self.seen {
			return Poll::Ready(());
		}
		let mut wakers = inner.wakers.lock();
		// A write may have landed between the version check and taking the waker lock.
		if inner.version.load(Ordering::Acquire) != self.seen {
			return Poll::Ready(());
		}
		// Components reuse their waker across polls, so one entry per task is enough.
		if !wakers.iter().any(|waker| waker.will_wake(cx.waker())) {
			wakers.push(cx.waker().clone());
		}
		Poll::Pending
	}
}

/// A subscriber is the reactive source of a watch: each write is a change, and the value is cloned into the
/// reactive value's own slot, reusing its storage, so no lock is held while the component works with it.
impl<T: Clone + 'static, C: 'static> crate::ui::reactive::Source<C> for Subscriber<T> {
	type Value = T;

	fn poll_update(&mut self, latest: &mut Option<T>, cx: &mut Context<'_>) -> Poll<()> {
		if self.poll_unseen(cx).is_pending() {
			return Poll::Pending;
		}
		let value = self.read();
		match latest {
			Some(latest) => latest.clone_from(&value),
			None => *latest = Some(value.clone()),
		}
		Poll::Ready(())
	}
}

/// The `Changed` struct is the wait for the next write to a watched value.
pub struct Changed<'a, T> {
	/// Taken when the wait resolves, so the returned guard borrows the subscriber for the full `'a`.
	subscriber: Option<&'a mut Subscriber<T>>,
}

impl<T> Unpin for Changed<'_, T> {}

impl<'a, T> Future for Changed<'a, T> {
	type Output = utils::sync::MutexGuard<'a, T>;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let Some(subscriber) = self.subscriber.as_deref_mut() else {
			return Poll::Pending;
		};
		if subscriber.poll_unseen(cx).is_pending() {
			return Poll::Pending;
		}
		Poll::Ready(self.subscriber.take().unwrap().read())
	}
}

impl<T> FusedFuture for Changed<'_, T> {
	fn is_terminated(&self) -> bool {
		self.subscriber.is_none()
	}
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::AtomicUsize;

	use super::*;

	#[derive(Default)]
	struct WakeCount(AtomicUsize);

	impl std::task::Wake for WakeCount {
		fn wake(self: Arc<Self>) {
			self.0.fetch_add(1, Ordering::Relaxed);
		}
	}

	fn poll<T>(changed: &mut Changed<'_, T>, waker: &Waker) -> bool {
		Pin::new(changed).poll(&mut Context::from_waker(waker)).is_ready()
	}

	#[test]
	fn a_wait_sleeps_until_a_write_and_then_reads_the_newest_value() {
		let watch = Watch::new(0u32);
		let mut subscriber = watch.subscribe();
		let count = Arc::new(WakeCount::default());
		let waker = Waker::from(Arc::clone(&count));
		assert!(poll(&mut subscriber.changed(), &waker), "the first wait must resolve at once");
		{
			let mut changed = subscriber.changed();
			assert!(!poll(&mut changed, &waker));
			assert!(!poll(&mut changed, &waker));
		}
		watch.set(1);
		watch.set(2);
		assert_eq!(
			count.0.load(Ordering::Relaxed),
			1,
			"writes between polls must coalesce into one wake"
		);
		let mut changed = subscriber.changed();
		assert!(poll(&mut changed, &waker));
	}

	#[test]
	fn a_read_marks_the_value_seen() {
		let watch = Watch::new(0u32);
		let mut subscriber = watch.subscribe();
		watch.set(1);
		assert_eq!(*subscriber.read(), 1);
		let waker = Waker::from(Arc::new(WakeCount::default()));
		assert!(!poll(&mut subscriber.changed(), &waker));
	}
}
