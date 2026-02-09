//! The Factory is an special Channel that handles the creation of new entities.

use crate::core::{channel::Channel, listener::{DefaultListener, Listener}, message::Message};

pub struct Factory<T: Clone + ?Sized>(Channel<CreateMessage<T>>);

impl <T: Clone> Factory<T> {
    pub fn new() -> Self {
        let sender = Channel::new();
        Factory(sender)
    }

    pub fn create(&self, data: T) -> Handle {
        self.0.send(CreateMessage::new(data));

        Handle
    }

    pub fn listener(&self) -> DefaultListener<CreateMessage<T>> {
        self.0.listener()
    }
}

#[derive(Debug, Clone)]
pub struct CreateMessage<T: Clone> {
	data: T,
}

impl <T: Clone> CreateMessage<T> {
	pub fn new(data: T) -> Self {
		CreateMessage { data }
	}
}

impl <T: Clone> Message for CreateMessage<T> {}

#[derive(Clone, PartialEq, Eq)]
pub struct Handle;
