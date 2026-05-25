use crate::core::{channel::DefaultChannel, factory::Handle, listener::FilteredListener};

pub trait Message {}

#[derive(Debug, Clone)]
/// The `DeleteMessage` struct exists to broadcast entity removal intent across world systems.
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
