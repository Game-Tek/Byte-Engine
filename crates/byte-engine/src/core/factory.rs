//! The Factory is an special Channel that handles the creation of new entities.

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
pub struct Handle(u32);
