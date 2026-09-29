//! Fluent publishing for messages addressed to one engine handle.
//!
//! Import [`TargetedMessagePublisher`] and [`MessageTargeter`], call
//! [`TargetedMessagePublisher::set`], then finish with [`MessageTargeter::on`].

/// The `TargetedMessage` trait associates a message with the payload sent to one handle.
pub trait TargetedMessage: Message {
	type Payload;

	/// Whether publishing this message ends the target entity's lifecycle.
	const ENDS_TARGET_LIFECYCLE: bool = false;

	fn from_handle_and_payload(handle: Handle, payload: Self::Payload) -> Self
	where
		Self: Sized;
}

/// The `TargetedMessagePublisher` trait provides the first step for publishing a payload to one handle.
///
/// After calling [`TargetedMessagePublisher::set`], call [`MessageTargeter::on`] to select the
/// destination and publish the message.
pub trait TargetedMessagePublisher<P>: Publisher<Self::Message> {
	/// Selects the targeted message associated with this payload type.
	type Message: TargetedMessage<Payload = P>;

	/// Sets the payload for a targeted message.
	///
	/// Next, call [`MessageTargeter::on`] to select the destination and publish the message.
	fn set(&self, payload: P) -> PendingTargetedMessage<'_, Self, Self::Message> {
		PendingTargetedMessage {
			publisher: self,
			payload,
			message: PhantomData,
		}
	}
}

/// The `PendingTargetedMessage` struct holds a payload until you select its destination.
///
/// Call [`MessageTargeter::on`] to construct and publish the targeted message.
pub struct PendingTargetedMessage<'a, W: ?Sized, M: TargetedMessage> {
	publisher: &'a W,
	payload: M::Payload,
	message: PhantomData<fn() -> M>,
}

/// The `MessageTargeter` trait completes a pending targeted message with its destination.
pub trait MessageTargeter {
	/// Publishes the pending message on `handle`.
	fn on(self, handle: Handle);
}

impl<M, W: Publisher<M> + ?Sized> MessageTargeter for PendingTargetedMessage<'_, W, M>
where
	M: TargetedMessage,
{
	fn on(self, handle: Handle) {
		self.publisher.publish(M::from_handle_and_payload(handle, self.payload));
	}
}

use std::marker::PhantomData;

use crate::core::{factory::Handle, message::Message, publisher::Publisher};
