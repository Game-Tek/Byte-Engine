use std::{collections::VecDeque, marker::PhantomData};

use trotcast::Receiver;

use crate::core::channel::DefaultChannel;

/// The `Listener` trait lets consumers receive messages without depending on a
/// specific transport.
pub trait Listener<M> {
	fn read(&mut self) -> Option<M>;

	// fn iter(&mut self) -> ListenerIterator<'_, Self, M>
	// where
	//     Self: Sized,
	// {
	//     ListenerIterator::new(self)
	// }

	fn to_vec(&mut self) -> Vec<M> {
		let mut vec = Vec::new();
		while let Some(message) = self.read() {
			vec.push(message);
		}
		vec
	}
}

/// The `ListenerIterator` struct adapts a [`Listener`] for use in iterator-based
/// message processing.
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

impl<'a, L: ?Sized, M> Iterator for ListenerIterator<'a, L, M>
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

/// The `DefaultListener` struct reads broadcast messages from a `trotcast` receiver.
///
/// Use [`Self::new_listener`] to add a consumer. The new listener receives future
/// messages but does not inherit messages already queued for this listener.
#[derive(Clone)]
pub struct DefaultListener<M>(pub(super) Receiver<M>);

impl<M: Clone> DefaultListener<M> {
	/// Creates another listener for future messages on the same channel.
	///
	/// The new listener does not receive messages already queued for this listener.
	pub fn new_listener(&self) -> Self {
		DefaultListener(self.0.clone())
	}

	pub fn filtered<F>(&self, filter: F) -> FilteredListener<DefaultListener<M>, M, F>
	where
		F: Fn(&M) -> bool,
	{
		FilteredListener(DefaultListener(self.0.clone()), filter, PhantomData)
	}

	pub fn clone_channel(&self) -> DefaultChannel<M> {
		DefaultChannel(self.0.clone_channel())
	}
}

impl<M: Clone> Listener<M> for DefaultListener<M> {
	fn read(&mut self) -> Option<M> {
		self.0.try_recv().ok()
	}
}

/// The `FilteredListener` struct limits a listener to messages accepted by a predicate.
pub struct FilteredListener<L, M: Clone, F>(L, F, PhantomData<M>)
where
	L: Listener<M>,
	F: Fn(&M) -> bool;

impl<R, M: Clone, F> FilteredListener<R, M, F>
where
	R: Listener<M>,
	F: Fn(&M) -> bool,
{
}

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
	use std::collections::VecDeque;

	use super::Listener;
	use crate::core::channel::{Channel, DefaultChannel};

	#[test]
	fn each_listener_observes_the_same_order_without_consuming_for_others() {
		let channel = DefaultChannel::new();
		let mut first = channel.listener();
		let mut second = channel.listener();

		for value in [1, 2, 3] {
			channel.send(value);
		}

		assert_eq!(first.to_vec(), [1, 2, 3]);
		assert_eq!(second.to_vec(), [1, 2, 3]);
	}

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

	#[test]
	fn collection_listeners_have_explicit_stack_and_queue_ordering() {
		let mut stack = vec![1, 2, 3];
		let mut queue = VecDeque::from([1, 2, 3]);

		assert_eq!(stack.to_vec(), [3, 2, 1]);
		assert_eq!(queue.to_vec(), [1, 2, 3]);
	}

	#[test]
	fn cloned_channel_publishes_into_the_same_broadcast_stream() {
		let channel = DefaultChannel::with_expected_listeners(2);
		let listener = channel.listener();
		let cloned_channel = listener.clone_channel();
		let mut observer = listener.new_listener();

		cloned_channel.send("from clone");
		assert_eq!(observer.read(), Some("from clone"));
	}
}
