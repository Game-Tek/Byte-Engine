#![cfg_attr(
	test,
	expect(
		clippy::disallowed_types,
		reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"
	)
)]
use std::{
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

/// The `DefaultListener` struct owns one cursor in a typed message route.
///
/// Use [`Self::new_listener`] to add a consumer. The new listener starts at the
/// current tick's first message, so it does not inherit messages from earlier
/// ticks that are still queued for this listener. Call [`Listener::read`]
/// during the consumer's update, or await [`Self::recv`]. Drop the listener when the consumer
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

	pub(crate) fn from_token(token: ListenerToken<M>) -> Self {
		Self { token }
	}

	/// Returns a future that resolves with the next message, waiting until one is sent if none is queued.
	///
	/// Await it from a gameplay task or a UI component instead of polling [`Listener::read`] every tick. Messages
	/// sent while the future is not awaited stay queued for this listener, so dropping it, as the losing branch of a
	/// `select!` is, loses nothing. Every send on the channel wakes every waiting listener.
	pub fn recv(&mut self) -> Recv<'_, M> {
		Recv {
			listener: self,
			done: false,
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

#[cfg(test)]
mod tests {
	use std::{
		pin::pin,
		sync::{
			Arc,
			atomic::{AtomicUsize, Ordering},
		},
		task::{Context, Poll, Wake, Waker},
	};

	use super::Listener;
	use crate::core::channel::{Channel, DefaultChannel};

	/// Counts the wakes the listener's future asks for.
	struct Count(AtomicUsize);

	impl Wake for Count {
		fn wake(self: Arc<Self>) {
			self.0.fetch_add(1, Ordering::Relaxed);
		}
	}

	#[test]
	fn recv_resolves_with_a_queued_message_and_wakes_its_waiter_on_send() {
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
