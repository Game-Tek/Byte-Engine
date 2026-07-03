//! Broadcast channels used to connect engine systems without direct ownership.
//!
//! Create a [`DefaultChannel`], clone it for producers, and create one
//! [`crate::core::listener::DefaultListener`] per consumer. Use
//! [`crate::core::factory::Factory`] instead when messages represent entity
//! creation and require stable handles.

use trotcast::error::BlockingSendError;
use trotcast::Channel as Sender;
use trotcast::Receiver;

use crate::core::listener::DefaultListener;
use crate::core::listener::Listener;
use crate::core::message::Message;

/// The [`Channel`] trait defines message publication independently of the
/// underlying transport.
pub trait Channel<M> {
	fn send(&self, message: M);
}

#[derive(Clone)]
/// The [`DefaultChannel`] struct provides the engine's in-process broadcast
/// transport.
pub struct DefaultChannel<M>(pub(super) Sender<M>);

impl<M: Clone> Default for DefaultChannel<M> {
	fn default() -> Self {
		Self::new()
	}
}

impl<M: Clone> DefaultChannel<M> {
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

impl<M: Clone> Channel<M> for DefaultChannel<M> {
	fn send(&self, message: M) {
		if let Err(BlockingSendError::Disconnected(_)) = self.0.blocking_send(message) {
			log::debug!("No listeners for the message!");
		}
	}
}
