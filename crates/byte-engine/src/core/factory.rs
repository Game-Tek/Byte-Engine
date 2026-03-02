//! The Factory is an special Channel that handles the creation of new entities.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::core::{
	channel::{Channel as _, DefaultChannel},
	listener::{DefaultListener, Listener},
	message::Message,
};

pub struct Factory<T: Clone + ?Sized>(DefaultChannel<CreateMessage<T>>);

static COUNTER: AtomicU32 = AtomicU32::new(0);

impl<T: Clone> Factory<T> {
	pub fn new() -> Self {
		let sender = DefaultChannel::new();
		Factory(sender)
	}

	pub fn create(&mut self, data: T) -> Handle {
		let id = COUNTER.fetch_add(1, Ordering::Relaxed);

		let handle = Handle(id);

		self.0.send(CreateMessage::new(handle, data));

		Handle(id)
	}

	pub fn derive(&mut self, handle: Handle, data: T) {
		self.0.send(CreateMessage::new(handle, data));
	}

	pub fn listener(&self) -> DefaultListener<CreateMessage<T>> {
		self.0.listener()
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
