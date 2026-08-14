/// The `Publisher` trait provides a shared interface for sending engine messages.
pub trait Publisher<M: Message> {
	fn publish(&self, message: M);
}

use crate::core::message::Message;
