//! Protocol-to-world message posting for runtime debugging tools.

use facet::Facet;
use serde_json::Value;

use super::Inspector;
use crate::core::{channel::Channel, factory::Handle, message_bus::MessageRouteError, targeted_message::TargetedMessage};

pub(super) type SerializableMessagePoster = Box<dyn Fn(Handle, &Value) -> Result<(), String> + Send + Sync + 'static>;

/// The stable inspection-protocol name for a transform replacement message.
pub const TRANSFORMATION_UPDATE_MESSAGE_TYPE: &str = "TransformationUpdate";

impl Inspector {
	/// Registers one reflected targeted message for protocol posting.
	///
	/// `message_type` becomes the stable protocol `type`, and the reflected shape
	/// of `M::Payload` defines its JSON payload. Register every supported message
	/// before sharing the inspector with a protocol server.
	pub fn register_message<M>(&mut self, message_type: &'static str) -> Result<(), String>
	where
		M: TargetedMessage + Clone + Send + Sync + 'static,
		M::Payload: Facet<'static>,
	{
		if message_type.is_empty() {
			return Err(
				"Inspector message could not be registered. The most likely cause is that its protocol type name is empty."
					.to_string(),
			);
		}
		if self.serializable_messages.contains_key(message_type) {
			return Err(format!(
				"Inspector message could not be registered. The most likely cause is that protocol type name '{message_type}' is already registered."
			));
		}

		let channel = self
			.messages
			.try_channel::<M>()
			.map_err(|error| registration_error(message_type, &error))?;
		self.serializable_messages.insert(
			message_type,
			Box::new(move |target, payload| {
				let json = payload.to_string();
				let payload = facet_json::from_str::<M::Payload>(&json).map_err(|error| {
					format!(
						"Inspector message payload is invalid. The most likely cause is that '{message_type}' payload does not match its reflected shape: {error}"
					)
				})?;
				channel.send(M::from_handle_and_payload(target, payload));
				Ok(())
			}),
		);
		Ok(())
	}

	/// Publishes one registered targeted world message from its protocol name and reflected JSON payload.
	///
	/// Call [`Self::register_message`] for the message type before accepting posts.
	pub fn post_message(&self, message_type: &str, target: Handle, payload: &Value) -> Result<(), String> {
		self.serializable_messages
			.get(message_type)
			.ok_or_else(|| {
				format!(
				"Inspector message type is unsupported. The most likely cause is that '{message_type}' is not registered for inspector posting."
				)
			})?(target, payload)
	}
}

fn registration_error(message_type: &str, error: &MessageRouteError) -> String {
	format!(
		"Inspector message could not be registered. The most likely cause is that the typed route for '{message_type}' is unavailable: {error}"
	)
}

#[cfg(test)]
mod tests {
	use math::{Orientation, Point, Scale};

	use super::*;
	use crate::{
		configuration::Configuration,
		core::{channel::DefaultChannel, factory::Factory, listener::Listener, message::Message, message_bus::MessageBus},
		gameplay::TransformationUpdate,
	};

	/// Creates an inspector whose transform route already has a listener.
	fn test_inspector() -> (Inspector, crate::core::listener::DefaultListener<TransformationUpdate>) {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("inspector-test-world");
		let transforms = messages.channel();
		let listener = transforms.listener();

		let mut inspector = Inspector::new(DefaultChannel::new(), Configuration::new(), messages);
		inspector
			.register_message::<TransformationUpdate>(TRANSFORMATION_UPDATE_MESSAGE_TYPE)
			.expect("register reflected transformation update");

		(inspector, listener)
	}

	#[test]
	fn transformation_update_posts_the_selected_target_and_complete_payload() {
		let (inspector, mut transforms) = test_inspector();
		let factory = Factory::new();
		let _factory_listener = factory.listener();
		let target = factory.create(());

		inspector
			.post_message(
				TRANSFORMATION_UPDATE_MESSAGE_TYPE,
				target,
				&serde_json::json!({
					"position": [1.0, 2.0, 3.0],
					"scale": [2.0, 3.0, 4.0],
					"orientation": [0.0, 0.0, 0.0, 1.0]
				}),
			)
			.expect("post transform update");

		let update = transforms.read().expect("posted transform update");
		assert_eq!(update.handle(), target);
		assert_eq!(update.transform().get_position(), Point::new(1.0, 2.0, 3.0));
		assert_eq!(update.transform().scale(), Scale::new(2.0, 3.0, 4.0));
		assert_eq!(update.transform().get_orientation(), Orientation::identity());
	}

	#[test]
	fn unsupported_message_types_do_not_publish_to_the_transform_route() {
		let (inspector, mut transforms) = test_inspector();
		let factory = Factory::new();
		let _factory_listener = factory.listener();
		let target = factory.create(());

		let error = inspector
			.post_message("DeleteMessage", target, &Value::Null)
			.expect_err("unsupported inspector message type");

		assert!(error.contains("DeleteMessage"));
		assert!(transforms.read().is_none());
	}

	#[test]
	fn incomplete_transform_payloads_do_not_publish() {
		let (inspector, mut transforms) = test_inspector();
		let factory = Factory::new();
		let _factory_listener = factory.listener();
		let target = factory.create(());

		let error = inspector
			.post_message(
				TRANSFORMATION_UPDATE_MESSAGE_TYPE,
				target,
				&serde_json::json!({
					"position": [1.0, 2.0, 3.0]
				}),
			)
			.expect_err("incomplete transform payload");

		assert!(error.contains("payload does not match its reflected shape"));
		assert!(transforms.read().is_none());
	}

	#[test]
	fn invalid_transform_orientations_do_not_publish() {
		let (inspector, mut transforms) = test_inspector();
		let target = Factory::new().create(());

		let error = inspector
			.post_message(
				TRANSFORMATION_UPDATE_MESSAGE_TYPE,
				target,
				&serde_json::json!({
					"position": [1.0, 2.0, 3.0],
					"scale": [1.0, 1.0, 1.0],
					"orientation": [0.0, 0.0, 0.0, 0.0]
				}),
			)
			.expect_err("invalid transform orientation");

		assert!(error.contains("zero-length quaternion"));
		assert!(transforms.read().is_none());
	}

	/// The `ReflectedPayload` struct provides an arbitrary reflected test payload.
	#[derive(Clone, Debug, PartialEq, facet::Facet)]
	#[facet(deny_unknown_fields)]
	struct ReflectedPayload {
		name: String,
		weight: u32,
	}

	/// The `ReflectedTestMessage` struct verifies generic reflected-payload message registration.
	#[derive(Clone, Debug, PartialEq)]
	struct ReflectedTestMessage {
		target: Handle,
		payload: ReflectedPayload,
	}

	impl Message for ReflectedTestMessage {}

	impl TargetedMessage for ReflectedTestMessage {
		type Payload = ReflectedPayload;

		fn from_handle_and_payload(target: Handle, payload: Self::Payload) -> Self {
			Self { target, payload }
		}
	}

	#[test]
	fn any_targeted_message_with_a_reflected_payload_can_be_registered_and_posted() {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("reflected-message-test-world");
		let mut listener = messages.channel::<ReflectedTestMessage>().listener();
		let mut inspector = Inspector::new(DefaultChannel::new(), Configuration::new(), messages);
		inspector
			.register_message::<ReflectedTestMessage>("ReflectedTestMessage")
			.expect("register reflected test message");
		let target = Factory::new().create(());

		inspector
			.post_message(
				"ReflectedTestMessage",
				target,
				&serde_json::json!({ "name": "crate", "weight": 7 }),
			)
			.expect("post reflected test message");

		assert_eq!(
			listener.read(),
			Some(ReflectedTestMessage {
				target,
				payload: ReflectedPayload {
					name: "crate".to_string(),
					weight: 7,
				},
			})
		);
	}
}
