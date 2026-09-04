//! Protocol-to-world message posting for runtime debugging tools.

use facet::{Facet, ScalarType};
use serde_json::Value;

use super::DefaultInspector;
use super::shape::describe_json_shape;
use crate::core::{
	channel::{Channel, DefaultChannel},
	factory::Handle,
	targeted_message::TargetedMessage,
};

type SerializableMessagePoster = Box<dyn Fn(Handle, &Value) -> Result<(), String> + Send + Sync + 'static>;

/// The `RegisteredMessageType` struct describes one message type accepted by an inspector protocol adapter.
///
/// Use `payload_shape` to build controls for the JSON `payload`, then
/// send the completed value through [`crate::inspector::Inspector::post_message`].
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct RegisteredMessageType<'a> {
	/// The stable name used in the message envelope's `type` field.
	#[serde(rename = "type")]
	pub message_type: &'static str,
	/// The cached JSON-oriented shape of the message envelope's `payload` field.
	#[serde(rename = "payload")]
	pub payload_shape: &'a Value,
}

/// The `SerializableMessage` struct provides one source of truth for posting and describing an accepted protocol message.
pub(super) struct SerializableMessage {
	poster: SerializableMessagePoster,
	payload_shape: Value,
}

/// The stable inspection-protocol name for a transform replacement message.
pub const TRANSFORMATION_UPDATE_MESSAGE_TYPE: &str = "TransformationUpdate";
/// The stable inspection-protocol name for a terminal entity deletion.
pub const DELETE_MESSAGE_TYPE: &str = "Delete";
/// The alternate inspection-protocol name for a terminal entity deletion.
pub const DESTROY_MESSAGE_TYPE: &str = "Destroy";

impl DefaultInspector {
	/// Adds one reflected targeted message and its destination channel to the transport-neutral registry.
	pub(super) fn register_reflected_message<M>(
		&mut self,
		message_type: &'static str,
		channel: DefaultChannel<M>,
	) -> Result<(), String>
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

		let payload_shape = describe_json_shape(M::Payload::SHAPE);
		self.serializable_messages.insert(
			message_type,
			SerializableMessage {
				poster: Box::new(move |target, payload| {
					if matches!(M::Payload::SHAPE.scalar_type(), Some(ScalarType::Unit)) && !payload.is_null() {
						return Err(format!(
							"Inspector message payload is invalid. The most likely cause is that '{message_type}' requires JSON null for its reflected unit shape."
						));
					}
					let json = payload.to_string();
					let payload = facet_json::from_str::<M::Payload>(&json).map_err(|error| {
						format!(
							"Inspector message payload is invalid. The most likely cause is that '{message_type}' payload does not match its reflected shape: {error}"
						)
					})?;
					channel.send(M::from_handle_and_payload(target, payload));
					// Terminal posts use the same diagnostic lifecycle as DefaultWorld::delete.
					if M::ENDS_TARGET_LIFECYCLE {
						channel.forget_entity(target);
					}
					Ok(())
				}),
				payload_shape,
			},
		);
		Ok(())
	}

	/// Borrows registered protocol message types in stable name order.
	pub(super) fn registered_message_types(&self) -> Vec<RegisteredMessageType<'_>> {
		let mut types = self
			.serializable_messages
			.iter()
			.map(|(&message_type, message)| RegisteredMessageType {
				message_type,
				payload_shape: &message.payload_shape,
			})
			.collect::<Vec<_>>();
		types.sort_unstable_by_key(|message| message.message_type);
		types
	}

	/// Publishes one registered message after a transport has parsed its envelope.
	pub(super) fn post_registered_message(&self, message_type: &str, target: Handle, payload: &Value) -> Result<(), String> {
		self.serializable_messages
			.get(message_type)
			.ok_or_else(|| {
				format!(
				"Inspector message type is unsupported. The most likely cause is that '{message_type}' is not registered for inspector posting."
				)
			})
			.and_then(|message| (message.poster)(target, payload))
	}
}

#[cfg(test)]
mod tests {
	use math::{Orientation, Point, Scale};
	use serde_json::json;

	use super::*;
	use crate::{
		configuration::Configuration,
		core::{
			channel::DefaultChannel,
			factory::Factory,
			listener::{DefaultListener, Listener},
			message::{DeleteMessage, Message},
			message_bus::MessageBus,
			message_observer::MessageObserver,
		},
		gameplay::TransformationUpdate,
		input::{ActionEvent, SeatHandle, TRIGGER_ACTION_MESSAGE_TYPE, Value as InputValue},
		inspector::{DefaultInspector, Inspector},
	};

	/// Creates an inspector whose transform route already has a listener.
	fn test_inspector() -> (DefaultInspector, crate::core::listener::DefaultListener<TransformationUpdate>) {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("inspector-test-world");
		let transforms = messages.channel();
		let listener = transforms.listener();

		let mut inspector = DefaultInspector::new(DefaultChannel::new(), Configuration::new(), messages);
		inspector
			.register_message(TRANSFORMATION_UPDATE_MESSAGE_TYPE, transforms)
			.expect("register reflected transformation update");

		(inspector, listener)
	}

	/// Creates an inspector with both terminal protocol names on the canonical deletion route.
	fn deletion_inspector() -> (
		DefaultInspector,
		DefaultListener<DeleteMessage>,
		Factory<String>,
		MessageObserver,
	) {
		let message_bus = MessageBus::default();
		let observer = message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("deletion-inspector-test-world");
		let deletions = messages.channel::<DeleteMessage>();
		let listener = deletions.listener();
		let entities = messages.factory::<String>();
		let mut inspector = DefaultInspector::new(DefaultChannel::new(), Configuration::new(), messages);
		for message_type in [DELETE_MESSAGE_TYPE, DESTROY_MESSAGE_TYPE] {
			inspector
				.register_message(message_type, deletions.clone())
				.expect("register reflected deletion message");
		}

		(inspector, listener, entities, observer)
	}

	#[test]
	fn transformation_update_posts_the_selected_target_and_complete_payload() {
		let (inspector, mut transforms) = test_inspector();
		let target = Factory::new().create(());

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
	fn delete_and_destroy_posts_targeted_deletions_and_retire_entities() {
		let (inspector, mut deletions, entities, observer) = deletion_inspector();
		let delete_target = entities.create("delete-target".to_string());
		let destroy_target = entities.create("destroy-target".to_string());
		assert_eq!(observer.entities().len(), 2);

		for (message_type, target) in [(DELETE_MESSAGE_TYPE, delete_target), (DESTROY_MESSAGE_TYPE, destroy_target)] {
			inspector
				.post_message(message_type, target, &Value::Null)
				.expect("post reflected deletion");
			assert_eq!(deletions.read().expect("posted deletion").into_handle(), target);
			assert!(observer.entities().iter().all(|entity| entity.handle() != target));
		}
	}

	#[test]
	fn invalid_deletion_payloads_neither_publish_nor_retire_the_target() {
		let (inspector, mut deletions, entities, observer) = deletion_inspector();
		let target = entities.create("preserved-target".to_string());

		let error = inspector
			.post_message(DELETE_MESSAGE_TYPE, target, &serde_json::json!({}))
			.expect_err("reject non-unit deletion payload");

		assert!(error.contains("requires JSON null for its reflected unit shape"));
		assert!(deletions.read().is_none());
		assert_eq!(observer.entities()[0].handle(), target);
	}

	#[test]
	fn registered_unit_messages_report_sorted_null_payload_shapes() {
		let (inspector, _deletions, _entities, _observer) = deletion_inspector();
		let types = inspector.message_types();

		assert_eq!(
			types.iter().map(|message| message.message_type).collect::<Vec<_>>(),
			[DELETE_MESSAGE_TYPE, DESTROY_MESSAGE_TYPE]
		);
		assert!(
			types
				.iter()
				.all(|message| message.payload_shape == &json!({ "type": "null" }))
		);
	}

	#[test]
	fn unsupported_message_types_do_not_publish_to_the_transform_route() {
		let (inspector, mut transforms) = test_inspector();
		let target = Factory::new().create(());

		let error = inspector
			.post_message("DeleteMessage", target, &Value::Null)
			.expect_err("unsupported inspector message type");

		assert!(error.contains("DeleteMessage"));
		assert!(transforms.read().is_none());
	}

	#[test]
	fn malformed_transform_payloads_do_not_publish() {
		let (inspector, mut transforms) = test_inspector();
		let target = Factory::new().create(());
		for (payload, expected_error) in [
			(
				json!({ "position": [1.0, 2.0, 3.0] }),
				"payload does not match its reflected shape",
			),
			(
				json!({
					"position": [1.0, 2.0, 3.0],
					"scale": [1.0, 1.0, 1.0],
					"orientation": [0.0, 0.0, 0.0, 0.0]
				}),
				"zero-length quaternion",
			),
		] {
			let error = inspector
				.post_message(TRANSFORMATION_UPDATE_MESSAGE_TYPE, target, &payload)
				.expect_err("reject malformed transform payload");
			assert!(error.contains(expected_error));
			assert!(transforms.read().is_none());
		}
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
		let inspector_messages = message_bus.new_scope("reflected-message-test-inspector");
		let destination_messages = message_bus.new_scope("reflected-message-test-destination");
		let reflected_messages = destination_messages.channel::<ReflectedTestMessage>();
		let mut listener = reflected_messages.listener();
		let mut inspector = DefaultInspector::new(DefaultChannel::new(), Configuration::new(), inspector_messages);
		inspector
			.register_message("ReflectedTestMessage", reflected_messages)
			.expect("register reflected test message");
		let types = inspector.message_types();
		assert_eq!(types.len(), 1);
		assert_eq!(types[0].message_type, "ReflectedTestMessage");
		assert_eq!(
			types[0].payload_shape,
			&json!({
				"type": "object",
				"fields": [
					{
						"name": "name",
						"required": true,
						"flattened": false,
						"shape": { "type": "string" }
					},
					{
						"name": "weight",
						"required": true,
						"flattened": false,
						"shape": { "type": "integer", "format": "u32" }
					}
				],
				"additional_fields": false
			})
		);
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

	#[test]
	fn reflected_action_values_publish_canonical_action_events() {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let inspector_messages = message_bus.new_scope("action-inspector-test");
		let action_events = message_bus.new_scope("action-event-test").channel::<ActionEvent>();
		let mut listener = action_events.listener();
		let mut inspector = DefaultInspector::new(DefaultChannel::new(), Configuration::new(), inspector_messages);
		inspector
			.register_message(TRIGGER_ACTION_MESSAGE_TYPE, action_events)
			.expect("register reflected action event");
		let target = Factory::new().create(());

		inspector
			.post_message(
				TRIGGER_ACTION_MESSAGE_TYPE,
				target,
				&serde_json::json!({ "type": "Float", "value": 1.0 }),
			)
			.expect("post reflected action event");

		let event = listener.read().expect("posted action event");
		assert_eq!(event.seat_handle(), SeatHandle::stub());
		assert_eq!(event.handle(), target);
		assert_eq!(event.value(), InputValue::Float(1.0));
	}
}
