use std::{
	collections::VecDeque,
	marker::PhantomData,
	panic::Location,
	pin::Pin,
	task::{Context, Poll},
};

use utils::r#async::FusedFuture;

use crate::core::message_bus::ListenerToken;

/// The `Listener` trait lets consumers receive messages without depending on a specific transport.
pub trait Listener<M> {
	fn read(&mut self) -> Option<M>;

	fn to_vec(&mut self) -> Vec<M> {
		let mut vec = Vec::new();
		while let Some(message) = self.read() {
			vec.push(message);
		}
		vec
	}
}

/// The `ListenerIterator` struct adapts a [`Listener`] for iterator-based message processing.
pub struct ListenerIterator<'a, L: ?Sized, M>
where
	L: Listener<M>,
{
	listener: &'a mut L,
	_marker: PhantomData<M>,
}

impl<'a, L: ?Sized, M> ListenerIterator<'a, L, M>
where
	L: Listener<M>,
{
	fn new(listener: &'a mut L) -> Self {
		Self {
			listener,
			_marker: PhantomData,
		}
	}
}

impl<L: ?Sized, M> Iterator for ListenerIterator<'_, L, M>
where
	L: Listener<M>,
{
	type Item = M;

	fn next(&mut self) -> Option<Self::Item> {
		self.listener.read()
	}
}

impl<'a, M> IntoIterator for &'a mut (dyn Listener<M> + 'a) {
	type Item = M;
	type IntoIter = ListenerIterator<'a, dyn Listener<M> + 'a, M>;

	fn into_iter(self) -> Self::IntoIter {
		ListenerIterator::new(self)
	}
}

/// The `DefaultListener` struct owns one cursor in a typed message route.
///
/// Use [`Self::new_listener`] to add a consumer. The new listener starts at the
/// current tick's first message, so it does not inherit messages from earlier
/// ticks that are still queued for this listener. Call [`Listener::read`]
/// during the consumer's update, or use [`Self::filtered`] first when the
/// consumer needs only part of the stream. Drop the listener when the consumer
/// stops reading, so the storage behind it returns to the pool.
pub struct DefaultListener<M>
where
	M: Clone + Send + Sync + 'static,
{
	token: ListenerToken<M>,
}

impl<M> DefaultListener<M>
where
	M: Clone + Send + Sync + 'static,
{
	/// Creates another listener on the same channel, starting at the current tick's first message.
	#[track_caller]
	pub fn new_listener(&self) -> Self {
		Self {
			token: self.token.new_listener(Location::caller()),
		}
	}

	#[track_caller]
	pub fn filtered<F>(&self, filter: F) -> FilteredListener<DefaultListener<M>, M, F>
	where
		F: Fn(&M) -> bool,
	{
		FilteredListener(self.new_listener(), filter, PhantomData)
	}

	pub(crate) fn from_token(token: ListenerToken<M>) -> Self {
		Self { token }
	}

	/// Returns a future that resolves with the next message, waiting until one is sent if none is queued.
	///
	/// Await it from a gameplay task or a UI component instead of polling [`Listener::read`] every tick. Messages
	/// sent while the future is not awaited stay queued for this listener, so dropping it, as the losing branch of a
	/// `select!` is, loses nothing. Every send on the channel wakes every waiting listener; to wait for one entity's
	/// messages, use [`Self::recv_where`].
	pub fn recv(&mut self) -> Recv<'_, M> {
		Recv {
			listener: self,
			done: false,
		}
	}

	/// Waits for the next message `accept` returns `true` for, skipping the ones before it.
	///
	/// The skipped messages are consumed, so a listener used with this reads only what it waits for.
	pub async fn recv_where(&mut self, mut accept: impl FnMut(&M) -> bool) -> M {
		loop {
			let message = self.recv().await;
			if accept(&message) {
				return message;
			}
		}
	}
}

/// The `Recv` struct is the future [`DefaultListener::recv`] returns, so tasks and components can await a channel.
pub struct Recv<'a, M>
where
	M: Clone + Send + Sync + 'static,
{
	listener: &'a mut DefaultListener<M>,
	done: bool,
}

impl<M> Future for Recv<'_, M>
where
	M: Clone + Send + Sync + 'static,
{
	type Output = M;

	fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<M> {
		let this = self.get_mut();
		if this.done {
			return Poll::Pending;
		}
		let result = this.listener.token.poll_read(context);
		this.done = result.is_ready();
		result
	}
}

impl<M> FusedFuture for Recv<'_, M>
where
	M: Clone + Send + Sync + 'static,
{
	fn is_terminated(&self) -> bool {
		self.done
	}
}

impl<M> Listener<M> for DefaultListener<M>
where
	M: Clone + Send + Sync + 'static,
{
	fn read(&mut self) -> Option<M> {
		self.token.read()
	}
}

/// The `FilteredListener` struct limits a listener to messages accepted by a predicate.
pub struct FilteredListener<L, M: Clone, F>(L, F, PhantomData<M>)
where
	L: Listener<M>,
	F: Fn(&M) -> bool;

impl<L, M: Clone, F> Listener<M> for FilteredListener<L, M, F>
where
	L: Listener<M>,
	F: Fn(&M) -> bool,
{
	fn read(&mut self) -> Option<M> {
		// Drain pending messages until one satisfies the filter predicate.
		while let Some(message) = self.0.read() {
			if (self.1)(&message) {
				return Some(message);
			}
		}

		None
	}
}

impl<M: Clone> Listener<M> for Vec<M> {
	fn read(&mut self) -> Option<M> {
		self.pop()
	}
}

impl<M: Clone> Listener<M> for VecDeque<M> {
	fn read(&mut self) -> Option<M> {
		self.pop_front()
	}
}

impl<L, M: Clone, F> Iterator for FilteredListener<L, M, F>
where
	L: Listener<M>,
	F: Fn(&M) -> bool,
{
	type Item = M;

	fn next(&mut self) -> Option<Self::Item> {
		self.read()
	}
}

#[cfg(test)]
mod tests {
	use super::Listener;
	use crate::core::channel::{Channel, DefaultChannel};

	#[test]
	fn filtered_listener_drains_rejected_messages_and_preserves_match_order() {
		let channel = DefaultChannel::new();
		let listener = channel.listener();
		let mut even = listener.filtered(|value| value % 2 == 0);

		for value in 1..=6 {
			channel.send(value);
		}

		assert_eq!(even.by_ref().collect::<Vec<_>>(), [2, 4, 6]);
		assert_eq!(even.read(), None);
	}

	#[test]
	fn recv_resolves_with_a_queued_message_and_wakes_its_waiter_on_send() {
		use std::{
			pin::pin,
			sync::{
				Arc,
				atomic::{AtomicUsize, Ordering},
			},
			task::{Context, Poll, Wake, Waker},
		};

		/// Counts the wakes the listener's future asks for.
		struct Count(AtomicUsize);
		impl Wake for Count {
			fn wake(self: Arc<Self>) {
				self.0.fetch_add(1, Ordering::Relaxed);
			}
		}

		let channel = DefaultChannel::new();
		let mut listener = channel.listener();
		let count = Arc::new(Count(AtomicUsize::new(0)));
		let waker = Waker::from(Arc::clone(&count));
		let mut context = Context::from_waker(&waker);

		channel.send(1);
		assert_eq!(pin!(listener.recv()).poll(&mut context), Poll::Ready(1));

		let mut next = pin!(listener.recv());
		assert_eq!(next.as_mut().poll(&mut context), Poll::Pending);
		// Polling again before a send registers nothing more, so the send wakes once.
		assert_eq!(next.as_mut().poll(&mut context), Poll::Pending);
		channel.send(2);
		assert_eq!(count.0.load(Ordering::Relaxed), 1);
		assert_eq!(next.poll(&mut context), Poll::Ready(2));
	}

	#[test]
	fn dropping_a_waiting_listener_releases_its_waker() {
		use std::{
			pin::pin,
			sync::{
				Arc,
				atomic::{AtomicUsize, Ordering},
			},
			task::{Context, Poll, Wake, Waker},
		};

		/// Counts the wakes the listener's future asks for.
		struct Count(AtomicUsize);
		impl Wake for Count {
			fn wake(self: Arc<Self>) {
				self.0.fetch_add(1, Ordering::Relaxed);
			}
		}

		let channel = DefaultChannel::<u32>::new();
		let count = Arc::new(Count(AtomicUsize::new(0)));
		let waker = Waker::from(Arc::clone(&count));
		let mut context = Context::from_waker(&waker);
		let other = Arc::new(Count(AtomicUsize::new(0)));
		let other_waker = Waker::from(Arc::clone(&other));

		let mut listener = channel.listener();
		{
			let mut next = pin!(listener.recv());
			assert_eq!(next.as_mut().poll(&mut context), Poll::Pending);
			// Polling with another waker replaces the registration instead of adding one.
			assert_eq!(next.as_mut().poll(&mut Context::from_waker(&other_waker)), Poll::Pending);
		}
		drop(listener);
		drop(waker);
		drop(other_waker);

		assert_eq!(Arc::strong_count(&count), 1);
		assert_eq!(Arc::strong_count(&other), 1);
		channel.send(1);
		assert_eq!(count.0.load(Ordering::Relaxed), 0);
		assert_eq!(other.0.load(Ordering::Relaxed), 0);
	}

	#[test]
	fn listener_cloned_after_messages_starts_at_the_current_tail() {
		let channel = DefaultChannel::new();
		let mut original = channel.listener();
		channel.send(1);
		let mut late = original.new_listener();
		channel.send(2);

		assert_eq!(original.to_vec(), [1, 2]);
		assert_eq!(late.to_vec(), [2]);
	}
}
