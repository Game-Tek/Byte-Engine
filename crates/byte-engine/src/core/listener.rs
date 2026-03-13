use std::{collections::VecDeque, marker::PhantomData};

use trotcast::Receiver;

use crate::core::channel::DefaultChannel;

/// The `Listener` trait exists to decouple message consumption from transport details.
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

/// The `ListenerIterator` struct exists to provide iterator semantics for any listener implementation.
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

/// The `DefaultListener` struct exists to read messages from a `trotcast` receiver.
/// We do not allow cloning (directly) since it is easy to forget cloning the receiver does not carry over existing messages.
/// We provide an explicit method `new_listener` to create a new listener.
#[derive(Clone)]
pub struct DefaultListener<M>(pub(super) Receiver<M>);

impl<M: Clone> DefaultListener<M> {
	/// Create a new listener from a receiver.
	/// Equivalent to a clone operation.
	/// Remember cloning the receiver does not carry over existing messages.
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

/// The `FilteredListener` struct exists to compose message predicates over listeners.
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
