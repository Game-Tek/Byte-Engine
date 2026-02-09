use trotcast::Channel as Sender;
use trotcast::Receiver as Receiver;

use crate::core::listener::DefaultListener;
use crate::core::listener::Listener;
use crate::core::message::Message;

pub struct Channel<M>(Sender<M>);

impl <M: Clone> Channel<M> {
	pub fn new() -> Self {
		let sender = Sender::new(4);
		Channel(sender)
	}

	pub fn with_expected_listeners(capacity: usize) -> Self {
		let sender = Sender::new(capacity);
		Channel(sender)
	}

	pub fn send(&self, message: M) {
		self.0.blocking_send(message).unwrap()
	}

	pub fn listener(&self) -> DefaultListener<M> {
		DefaultListener(self.0.spawn_rx())
	}
}
