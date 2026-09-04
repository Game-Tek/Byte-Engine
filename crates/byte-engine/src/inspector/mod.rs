//! Runtime inspection contracts and protocol-facing state access.
//!
//! The [`Inspector`] trait exposes factory-created handles, attached [`Name`]
//! values, application controls, screenshots, and passive message publication
//! headers without choosing a transport. Other application values and
//! published payloads remain opaque.

use std::{collections::HashMap, sync::Arc};

use facet::Facet;
#[cfg(feature = "headed")]
use screenshot::ScreenshotBroker;
use serde::{Serialize, Serializer, ser::SerializeStruct};
use serde_json::Value;
use utils::sync::Mutex;

use crate::{
	application::Events,
	configuration::{Configuration, ConfigurationEvent},
	core::{
		channel::{Channel, DefaultChannel},
		factory::Handle,
		message_bus::{MessageBus, MessageScope},
		message_observer::{MessageObserver, ObservedEntity},
		targeted_message::TargetedMessage,
	},
	gameplay::Name,
};

#[cfg(feature = "headed")]
#[doc(hidden)]
pub mod http;
mod message;
use message::SerializableMessage;
pub use message::{DELETE_MESSAGE_TYPE, DESTROY_MESSAGE_TYPE, RegisteredMessageType, TRANSFORMATION_UPDATE_MESSAGE_TYPE};
#[cfg(feature = "headed")]
pub(crate) mod screenshot;
#[cfg(feature = "headed")]
pub use screenshot::{
	Screenshot, ScreenshotCapture, ScreenshotError, ScreenshotResponse, ScreenshotResult, ScreenshotSubmitError,
};
mod shape;

/// The [`Inspectable`] trait defines the read and mutation surface exposed to
/// external engine tooling.
pub trait Inspectable: Send + Sync {
	/// Returns a display string for inspection responses.
	fn as_string(&self) -> String;

	/// Returns the class name used by inspection filters.
	fn class_name(&self) -> &'static str {
		std::any::type_name::<Self>()
	}

	/// Applies an inspector-provided string value to a named property.
	fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
		Err(
			"Inspector mutation is not implemented. The most likely cause is that this inspectable type did not override set."
				.to_string(),
		)
	}
}

/// The `Inspector` trait defines the transport-neutral controls and diagnostics exposed to external engine tooling.
///
/// Register supported messages on a concrete implementation before sharing it
/// as `dyn Inspector`, then pass the same handle to each protocol transport.
pub trait Inspector: Send + Sync {
	/// Registers one reflected targeted message and its destination channel for protocol posting.
	///
	/// Create the channel's listeners before registration. The channel may belong
	/// to any scope on the shared message bus.
	fn register_message<M>(&mut self, message_type: &'static str, channel: DefaultChannel<M>) -> Result<(), String>
	where
		Self: Sized,
		M: TargetedMessage + Clone + Send + Sync + 'static,
		M::Payload: Facet<'static>;

	/// Returns the latest configuration event states for protocol adapters.
	fn configuration_events(&self) -> Vec<ConfigurationEvent>;

	/// Returns current factory-created entities filtered by an exact Rust type or attached name.
	fn entities(&self, entity_type: Option<&str>, name: Option<&str>) -> Vec<InspectedEntity>;

	/// Drains passive publication headers and resolves their route metadata.
	fn drain_messages(&self) -> Vec<InspectedMessage>;

	/// Returns registered protocol message types in stable name order.
	fn message_types(&self) -> Vec<RegisteredMessageType<'_>>;

	/// Publishes one registered targeted world message from its reflected JSON payload.
	fn post_message(&self, message_type: &str, target: Handle, payload: &Value) -> Result<(), String>;

	/// Queues one screenshot request and returns its one-shot response.
	#[cfg(feature = "headed")]
	fn request_screenshot(&self, sink: usize, capture: ScreenshotCapture) -> Result<ScreenshotResponse, ScreenshotSubmitError>;

	/// Requests application shutdown through the inspector event channel.
	fn close_application(&self);
}

/// The `InspectedMessage` struct resolves one passive publication to its scope and Rust message type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InspectedMessage {
	/// The stable bus-local route identifier.
	#[serde(rename = "topic")]
	pub topic_id: usize,
	/// The diagnostic name of the route's owning scope.
	pub scope: Arc<str>,
	/// The complete Rust type name, including generic arguments.
	#[serde(rename = "type")]
	pub message_type: &'static str,
	/// The first zero-based publication sequence in this range.
	pub first_sequence: u64,
	/// The number of consecutive publications in this range.
	pub count: u64,
}

/// The `InspectedEntity` struct describes one current entity and its optional human-readable name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectedEntity {
	entity: ObservedEntity,
	name: Option<Name>,
}

impl Serialize for InspectedEntity {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut entity = serializer.serialize_struct("InspectedEntity", 3)?;
		entity.serialize_field("target", &self.handle().id())?;
		entity.serialize_field("name", &self.name())?;
		entity.serialize_field("types", self.types())?;
		entity.end()
	}
}

impl InspectedEntity {
	/// Returns the stable identity shared by the entity's representations.
	pub fn handle(&self) -> Handle {
		self.entity.handle()
	}

	/// Returns the Rust type names in their first-published order.
	pub fn types(&self) -> &[&'static str] {
		self.entity.types()
	}

	/// Returns the name attached through [`Name`] when the entity has one.
	pub fn name(&self) -> Option<&str> {
		self.name.as_ref().map(Name::as_str)
	}
}

/// The `DefaultInspector` struct owns engine controls and runtime diagnostics shared by protocol adapters.
pub struct DefaultInspector {
	events: DefaultChannel<Events>,
	configuration: Configuration,
	serializable_messages: HashMap<&'static str, SerializableMessage>,
	message_bus: MessageBus,
	message_observer: MessageObserver,
	entity_names: Arc<Mutex<HashMap<Handle, Name>>>,
	#[cfg(feature = "headed")]
	screenshots: Arc<ScreenshotBroker>,
}

impl DefaultInspector {
	/// Creates an inspector backend that can publish controls and inspect one shared message bus.
	///
	/// Register the application and world listeners before passing their routes so
	/// inspector requests cannot be published without a consumer. Attach message
	/// observation before acquiring any routes in `messages`. Next, call
	/// [`Inspector::register_message`] with each supported destination channel before sharing
	/// the inspector with a protocol adapter. Spawn named entities after
	/// construction because name collection is future-only.
	pub fn new(events: DefaultChannel<Events>, configuration: Configuration, messages: MessageScope) -> Self {
		let message_bus = messages.message_bus().clone();
		let message_observer = message_bus.observer().unwrap_or_else(|| {
			panic!(
				"Inspector message observation is unavailable. The most likely cause is that MessageBus::observe was not called before acquiring application routes."
			)
		});
		let entity_names = Arc::new(Mutex::new(HashMap::new()));
		let collected_names = Arc::clone(&entity_names);
		let forgotten_names = Arc::clone(&entity_names);
		// Names are the one factory value retained by inspection. The collector
		// runs at publication time, so scene spawning cannot fill a dormant queue.
		message_observer.collect_entity_values::<Name, _, _>(
			move |handle, name| {
				collected_names.lock().insert(handle, name.clone());
			},
			move |handle| {
				forgotten_names.lock().remove(&handle);
			},
		);
		Self {
			events,
			configuration,
			serializable_messages: HashMap::new(),
			message_bus,
			message_observer,
			entity_names,
			#[cfg(feature = "headed")]
			screenshots: Arc::new(ScreenshotBroker::new()),
		}
	}

	/// Returns the bounded screenshot exchange consumed by the graphics application.
	#[cfg(feature = "headed")]
	pub(crate) fn screenshot_broker(&self) -> Arc<ScreenshotBroker> {
		Arc::clone(&self.screenshots)
	}
}

impl Inspector for DefaultInspector {
	fn register_message<M>(&mut self, message_type: &'static str, channel: DefaultChannel<M>) -> Result<(), String>
	where
		M: TargetedMessage + Clone + Send + Sync + 'static,
		M::Payload: Facet<'static>,
	{
		self.register_reflected_message(message_type, channel)
	}

	fn configuration_events(&self) -> Vec<ConfigurationEvent> {
		self.configuration.events()
	}

	fn entities(&self, entity_type: Option<&str>, name: Option<&str>) -> Vec<InspectedEntity> {
		let names = self.entity_names.lock();
		self.message_observer
			.entities()
			.into_iter()
			.filter_map(|entity| {
				if entity_type.is_some_and(|entity_type| !entity.types().contains(&entity_type)) {
					return None;
				}
				let entity_name = names.get(&entity.handle()).cloned();
				if name.is_some_and(|name| entity_name.as_ref().is_none_or(|entity_name| entity_name.as_str() != name)) {
					return None;
				}
				Some(InspectedEntity {
					entity,
					name: entity_name,
				})
			})
			.collect()
	}

	fn drain_messages(&self) -> Vec<InspectedMessage> {
		let topic_snapshots = self.message_bus.topics();
		let batch = self.message_observer.drain_messages(&topic_snapshots);
		let mut topics = vec![None; self.message_bus.config().max_topics];
		for topic in topic_snapshots {
			let topic_id = topic.topic_id;
			topics[topic_id] = Some(topic);
		}
		batch
			.messages()
			.iter()
			.map(|observation| {
				let topic = topics[observation.topic_id()]
					.as_ref()
					.expect("An observed publication must retain its registered message topic");
				InspectedMessage {
					topic_id: observation.topic_id(),
					scope: Arc::clone(&topic.scope),
					message_type: topic.message_type,
					first_sequence: observation.first_sequence(),
					count: observation.count(),
				}
			})
			.collect()
	}

	fn message_types(&self) -> Vec<RegisteredMessageType<'_>> {
		self.registered_message_types()
	}

	fn post_message(&self, message_type: &str, target: Handle, payload: &Value) -> Result<(), String> {
		self.post_registered_message(message_type, target, payload)
	}

	#[cfg(feature = "headed")]
	fn request_screenshot(&self, sink: usize, capture: ScreenshotCapture) -> Result<ScreenshotResponse, ScreenshotSubmitError> {
		self.screenshots.request(sink, capture)
	}

	fn close_application(&self) {
		self.events.send(Events::Close);
	}
}

#[cfg(all(test, feature = "headed"))]
mod tests {
	use super::{DefaultInspector, Inspector};
	use crate::{
		configuration::Configuration,
		core::{Creator as _, channel::DefaultChannel, factory::Handle, message_bus::MessageBus},
		gameplay::{DefaultWorld, Name},
	};

	#[test]
	fn names_follow_spawn_replacement_and_deletion_through_the_inspector() {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("named-entity-test-world");
		let world = DefaultWorld::with_messages(messages.clone());
		let inspector = DefaultInspector::new(DefaultChannel::new(), Configuration::new(), messages);

		let handle: Handle = world.create(String::from("crate-model")).with(Name::new("crate")).into();

		let entities = inspector.entities(None, Some("crate"));
		assert_eq!(entities.len(), 1);
		assert_eq!(entities[0].handle(), handle);
		assert_eq!(entities[0].name(), Some("crate"));
		assert_eq!(
			inspector.entities(Some(std::any::type_name::<String>()), Some("crate")).len(),
			1
		);

		world.factory::<Name>().derive(handle, Name::new("shipping crate"));
		assert!(inspector.entities(None, Some("crate")).is_empty());
		assert_eq!(inspector.entities(None, Some("shipping crate"))[0].handle(), handle);

		world.delete(handle);
		assert!(inspector.entities(None, Some("shipping crate")).is_empty());
	}
}
