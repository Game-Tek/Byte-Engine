use crate::core::{channel::DefaultChannel, factory::Handle, listener::FilteredListener};

pub trait Message {}

#[derive(Debug, Clone)]
/// The `DeleteMessage` struct carries a terminal entity removal request across
/// world systems.
///
/// After sending this message, create a new factory value when you need another
/// entity. Do not derive a new representation from the deleted handle.
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
