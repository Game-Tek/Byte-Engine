//! The Factory is an special Channel that handles the creation of new entities.

use crate::core::{channel::Channel, listener::{DefaultListener, Listener}, message::Message};

pub struct Factory<T: Clone + ?Sized>(Channel<CreateMessage<T>>, u32);

impl <T: Clone> Factory<T> {
	pub fn new() -> Self {
		let sender = Channel::new();
		Factory(sender, 0)
	}

	pub fn create(&mut self, data: T) -> Handle {
		let id = self.1;

		let handle = Handle(id);

		self.0.send(CreateMessage::new(handle, data));

		self.1 += 1;

		Handle(id)
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

impl <T: Clone> CreateMessage<T> {
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

impl <T: Clone> Message for CreateMessage<T> {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Handle(u32);
