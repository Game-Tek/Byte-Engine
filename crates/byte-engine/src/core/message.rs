use crate::core::{channel::DefaultChannel, factory::Handle, listener::FilteredListener, targeted_message::TargetedMessage};

/// The `Message` trait marks values that engine publishers can send.
pub trait Message {}

/// The `DeleteMessage` struct carries a terminal entity removal request across
/// world systems.
///
/// After sending this message, create a new factory value when you need another
/// entity. Do not derive a new representation from the deleted handle.
#[derive(Debug, Clone)]
pub struct DeleteMessage {
	handle: Handle,
}

impl DeleteMessage {
	pub fn new(handle: Handle) -> Self {
		Self { handle }
	}

	pub fn handle(&self) -> &Handle {
		&self.handle
	}

	pub fn into_handle(self) -> Handle {
		self.handle
	}
}

impl Message for DeleteMessage {}

impl TargetedMessage for DeleteMessage {
	type Payload = ();

	fn from_handle_and_payload(handle: Handle, (): Self::Payload) -> Self {
		Self::new(handle)
	}
}
