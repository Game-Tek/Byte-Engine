use crate::core::{channel::DefaultChannel, factory::Handle, listener::FilteredListener};

pub trait Message {}

#[derive(Debug, Clone)]
/// The `DeleteMessage` struct carries an entity removal request across world systems.
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

#[cfg(test)]
mod tests {
	use super::DeleteMessage;
	use crate::core::factory::Factory;

	#[test]
	fn delete_message_preserves_the_exact_factory_handle() {
		let mut factory = Factory::new();
		let handle = factory.create("entity");
		let message = DeleteMessage::new(handle);

		assert_eq!(message.handle(), &handle);
		assert_eq!(message.into_handle(), handle);
	}
}
