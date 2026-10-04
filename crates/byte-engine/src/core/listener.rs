use std::{collections::VecDeque, marker::PhantomData, panic::Location};

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
