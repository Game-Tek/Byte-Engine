//! Creation messages with stable handles.
//!
//! A [`Factory`] is the standard boundary between code that creates an object
//! and systems that mirror it. World factories use this pattern to notify
//! rendering and physics without giving those systems ownership of gameplay
//! objects.

use std::{
	cell::{Cell, RefCell},
	rc::Rc,
	sync::atomic::{AtomicU32, Ordering},
};

use crate::core::{
	channel::{Channel as _, DefaultChannel},
	listener::{DefaultListener, Listener},
	message::Message,
};

#[derive(Clone)]
/// The `Factory` struct exists to create entity messages while preserving setup-time history for the first system listener.
pub struct Factory<T: Clone + ?Sized> {
	channel: DefaultChannel<CreateMessage<T>>,
	created_before_listener: Rc<RefCell<Vec<CreateMessage<T>>>>,
	record_created_before_listener: Rc<Cell<bool>>,
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

impl<T: Clone> Default for Factory<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T: Clone> Factory<T> {
	pub fn new() -> Self {
		let sender = DefaultChannel::new();
		Factory {
			channel: sender,
			created_before_listener: Rc::new(RefCell::new(Vec::new())),
			record_created_before_listener: Rc::new(Cell::new(true)),
		}
	}

	pub fn create(&mut self, data: T) -> Handle {
		let id = COUNTER.fetch_add(1, Ordering::Relaxed);

		let handle = Handle(id);
		let message = CreateMessage::new(handle, data);

		self.record_creation_before_listener(&message);
		self.channel.send(message);

		Handle(id)
	}

	pub fn derive(&mut self, handle: Handle, data: T) {
		let message = CreateMessage::new(handle, data);

		self.record_creation_before_listener(&message);
		self.channel.send(message);
	}

	pub fn listener(&self) -> DefaultListener<CreateMessage<T>> {
		self.record_created_before_listener.set(false);
		self.channel.listener()
	}

	/// Drains messages created before the first listener was registered.
	pub fn drain_created_before_listener(&mut self) -> Vec<CreateMessage<T>> {
		std::mem::take(&mut *self.created_before_listener.borrow_mut())
	}

	fn record_creation_before_listener(&mut self, message: &CreateMessage<T>) {
		if self.record_created_before_listener.get() {
			self.created_before_listener.borrow_mut().push(message.clone());
		}
	}
}

#[derive(Debug, Clone)]
/// The [`CreateMessage`] struct carries a created value and the stable handle
/// shared by systems that mirror it.
pub struct CreateMessage<T: Clone> {
	handle: Handle,
	data: T,
}

impl<T: Clone> CreateMessage<T> {
	fn new(handle: Handle, data: T) -> Self {
		CreateMessage { handle, data }
	}

	pub fn data(&self) -> &T {
		&self.data
	}

	pub fn into_data(self) -> T {
		self.data
	}

	pub fn handle(&self) -> &Handle {
		&self.handle
	}
}

impl<T: Clone> Message for CreateMessage<T> {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// The [`Handle`] struct identifies one creation stream entry across consuming
/// systems.
pub struct Handle(u32);

#[cfg(test)]
mod tests {
	use super::Factory;
	use crate::core::listener::Listener;

	#[test]
	fn create_assigns_distinct_handles_and_broadcasts_in_creation_order() {
		let mut factory = Factory::new();
		let mut listener = factory.listener();

		let first = factory.create("first");
		let second = factory.create("second");
		let messages = listener.to_vec();

		assert_ne!(first, second);
		assert_eq!(messages.len(), 2);
		assert_eq!(messages[0].handle(), &first);
		assert_eq!(messages[0].data(), &"first");
		assert_eq!(messages[1].handle(), &second);
		assert_eq!(messages[1].data(), &"second");
	}

	#[test]
	fn derive_reuses_the_supplied_identity() {
		let mut factory = Factory::new();
		let mut listener = factory.listener();
		let handle = factory.create(String::from("source"));
		factory.derive(handle, String::from("derived"));

		let created = listener.read().expect("source creation");
		let derived = listener.read().expect("derived creation");
		assert_eq!(created.handle(), derived.handle());
		assert_eq!(derived.into_data(), "derived");
	}

	#[test]
	fn setup_history_stops_at_first_listener_and_drains_once() {
		let mut factory = Factory::new();
		let before_a = factory.create(10);
		let before_b = factory.create(20);
		let _listener = factory.listener();
		factory.create(30);

		let history = factory.drain_created_before_listener();
		assert_eq!(history.len(), 2);
		assert_eq!(history[0].handle(), &before_a);
		assert_eq!(history[1].handle(), &before_b);
		assert_eq!(history.iter().map(|message| *message.data()).collect::<Vec<_>>(), [10, 20]);
		assert!(factory.drain_created_before_listener().is_empty());
	}

	#[test]
	fn cloned_factories_share_creation_history_and_stream() {
		let original = Factory::new();
		let mut clone = original.clone();
		let mut listener = original.listener();

		let handle = clone.create(7);
		let message = listener.read().expect("clone publishes to shared channel");
		assert_eq!(message.handle(), &handle);
		assert_eq!(message.data(), &7);
	}
}
