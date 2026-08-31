//! Protocol-to-world message posting for runtime debugging tools.

use facet::{Facet, ScalarType};
use serde_json::Value;

use super::Inspector;
use super::shape::describe_json_shape;
use crate::core::{channel::Channel, factory::Handle, message_bus::MessageRouteError, targeted_message::TargetedMessage};

type SerializableMessagePoster = Box<dyn Fn(Handle, &Value) -> Result<(), String> + Send + Sync + 'static>;

/// The `RegisteredMessageType` struct describes one message type accepted by an inspector protocol adapter.
///
/// Use [`Self::payload_shape`] to build controls for the JSON `payload`, then
/// send the completed value through [`Inspector::post_message`].
#[derive(Clone, Copy, Debug)]
pub struct RegisteredMessageType<'a> {
	message_type: &'static str,
	payload_shape: &'a Value,
}

impl<'a> RegisteredMessageType<'a> {
	/// Returns the stable name used in the message envelope's `type` field.
	pub fn message_type(&self) -> &'static str {
		self.message_type
	}

	/// Returns the cached JSON-oriented shape of the message envelope's `payload` field.
	pub fn payload_shape(&self) -> &'a Value {
		self.payload_shape
	}
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

impl Inspector {
	/// Registers one reflected targeted message for protocol posting.
	///
	/// `message_type` becomes the stable protocol `type`, and the reflected shape
	/// of `M::Payload` defines its JSON payload. Register every supported message
	/// before sharing the inspector with a protocol server. Messages whose
	/// [`TargetedMessage::ENDS_TARGET_LIFECYCLE`] value is `true` also retire the
	/// target from entity diagnostics after publication. Reflected unit payloads
	/// use JSON `null`.
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

	/// Returns the registered protocol message types and their JSON payload shapes.
	///
	/// Results are sorted by protocol name so editor clients receive stable output.
	pub fn message_types(&self) -> Vec<RegisteredMessageType<'_>> {
		let mut types = self
			.serializable_messages
			.iter()
			.map(|(&message_type, message)| RegisteredMessageType {
				message_type,
				payload_shape: &message.payload_shape,
			})
			.collect::<Vec<_>>();
		types.sort_unstable_by_key(RegisteredMessageType::message_type);
		types
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
			})
			.and_then(|message| (message.poster)(target, payload))
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

	/// Creates an inspector with both terminal protocol names on the canonical deletion route.
	fn deletion_inspector() -> (Inspector, DefaultListener<DeleteMessage>, Factory<String>, MessageObserver) {
		let message_bus = MessageBus::default();
		let observer = message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("deletion-inspector-test-world");
		let deletions = messages.channel::<DeleteMessage>();
		let listener = deletions.listener();
		let entities = messages.factory::<String>();
		let mut inspector = Inspector::new(DefaultChannel::new(), Configuration::new(), messages);
		for message_type in [DELETE_MESSAGE_TYPE, DESTROY_MESSAGE_TYPE] {
			inspector
				.register_message::<DeleteMessage>(message_type)
				.expect("register reflected deletion message");
		}

		(inspector, listener, entities, observer)
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
			types.iter().map(RegisteredMessageType::message_type).collect::<Vec<_>>(),
			[DELETE_MESSAGE_TYPE, DESTROY_MESSAGE_TYPE]
		);
		assert!(
			types
				.iter()
				.all(|message| message.payload_shape() == &json!({ "type": "null" }))
		);
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
		let types = inspector.message_types();
		assert_eq!(types.len(), 1);
		assert_eq!(types[0].message_type(), "ReflectedTestMessage");
		assert_eq!(
			types[0].payload_shape(),
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
}
