use trotcast::Channel as Sender;
use trotcast::Receiver as Receiver;
use trotcast::error::BlockingSendError;

use crate::core::listener::DefaultListener;
use crate::core::listener::Listener;
use crate::core::message::Message;

pub trait Channel<M> {
	fn send(&self, message: M);
}

pub struct DefaultChannel<M>(Sender<M>);

impl <M: Clone> DefaultChannel<M> {
	pub fn new() -> Self {
		let sender = Sender::new(128);
		DefaultChannel(sender)
	}

	pub fn with_expected_listeners(capacity: usize) -> Self {
		let sender = Sender::new(capacity);
		DefaultChannel(sender)
	}

	pub fn listener(&self) -> DefaultListener<M> {
		DefaultListener(self.0.spawn_rx())
	}
}

impl <M: Clone> Channel<M> for DefaultChannel<M> {
	fn send(&self, message: M) {
		match self.0.blocking_send(message) {
			Err(BlockingSendError::Disconnected(_)) => {
				log::debug!("No listeners for the message!");
			},
			_ => (),
		}
	}
}
