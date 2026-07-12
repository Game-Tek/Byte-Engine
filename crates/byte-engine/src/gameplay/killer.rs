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

#[cfg(test)]
mod tests {
	use super::KillMessage;
	use crate::core::factory::Factory;

	#[test]
	fn kill_message_preserves_target_identity() {
		let mut factory = Factory::new();
		let handle = factory.create("target");
		assert_eq!(KillMessage::new(handle).handle(), handle);
	}
}
