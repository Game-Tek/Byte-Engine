use crate::core::{factory::Handle, message::Message, Entity, EntityHandle};

pub struct KillMessage {
	handle: Handle,
}

impl KillMessage {
	pub fn new(handle: Handle) -> Self {
		Self { handle }
	}

	pub fn handle(&self) -> Handle {
		self.handle
	}
}

impl Message for KillMessage {}
