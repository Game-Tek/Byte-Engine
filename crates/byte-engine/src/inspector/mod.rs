//! Runtime inspection contracts and protocol-facing state access.
//!
//! [`DefaultInspector`] exposes factory-created handles, attached [`Name`]
//! values, application controls, screenshots, and passive message publication
//! headers without choosing a transport. Other application values and
//! published payloads remain opaque.

use std::{collections::HashMap, sync::Arc};

#[cfg(feature = "headed")]
use screenshot::ScreenshotBroker;
use serde::{Serialize, Serializer, ser::SerializeStruct};
use utils::sync::Mutex;

use crate::{
	application::Events,
	configuration::{Configuration, ConfigurationEvent},
	core::{
		channel::{Channel, DefaultChannel},
		factory::Handle,
		message_bus::{MessageBus, MessageScope},
		message_observer::{MessageObserver, ObservedEntity},
	},
	gameplay::Name,
};

#[cfg(feature = "headed")]
#[doc(hidden)]
pub mod http;
mod message;
use message::SerializableMessage;
pub use message::{
	DELETE_MESSAGE_TYPE, DESTROY_MESSAGE_TYPE, RegisteredMessageType, TRANSFORMATION_UPDATE_MESSAGE_TYPE,
	TRIGGER_ACTION_MESSAGE_TYPE,
};
#[cfg(feature = "headed")]
pub(crate) mod screenshot;
#[cfg(feature = "headed")]
pub use screenshot::{
	MAX_SCREENSHOT_CAPTURES, ScreenshotCapture, ScreenshotError, ScreenshotFormat, ScreenshotResponse, ScreenshotSelection,
	ScreenshotSubmitError, Screenshots,
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
	fn set(&mut self, _key: &str, _value: &str) -> Result<(), String> {
		Err(
			"Inspector mutation is not implemented. The most likely cause is that this inspectable type did not override set."
				.to_string(),
		)
	}
}

/// The `InspectedMessage` struct resolves one passive publication to its scope and Rust message type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InspectedMessage {
	/// The stable bus-local route identifier.
	#[serde(rename = "topic")]
	pub topic_id: usize,
	/// The diagnostic name of the route's owning scope.
	pub scope: Box<str>,
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
	/// [`Self::register_message`] with each supported destination channel before sharing
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

	/// Returns the latest configuration event states for protocol adapters.
	pub fn configuration_events(&self) -> Vec<ConfigurationEvent> {
		self.configuration.events()
	}

	/// Returns current factory-created entities filtered by an exact Rust type or attached name.
	pub fn entities(&self, entity_type: Option<&str>, name: Option<&str>) -> Vec<InspectedEntity> {
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

	/// Drains passive publication headers and resolves their route metadata.
	pub fn drain_messages(&self) -> Vec<InspectedMessage> {
		let topic_snapshots = self.message_bus.topics();
		let batch = self.message_observer.drain_messages(&topic_snapshots);
		batch
			.messages()
			.iter()
			.map(|observation| {
				// The bus reports routes in id order, so the id indexes the snapshot list.
				let topic = &topic_snapshots[observation.topic_id()];
				debug_assert_eq!(topic.topic_id, observation.topic_id());
				InspectedMessage {
					topic_id: observation.topic_id(),
					scope: topic.scope.as_ref().into(),
					message_type: topic.message_type,
					first_sequence: observation.first_sequence(),
					count: observation.count(),
				}
			})
			.collect()
	}

	/// Queues captures that must come from the same frame and returns their one-shot response.
	///
	/// Next, receive the [`Screenshots`] from the response and encode each readback with
	/// [`ScreenshotFormat::encode`].
	#[cfg(feature = "headed")]
	pub fn request_screenshots(&self, captures: Vec<ScreenshotSelection>) -> Result<ScreenshotResponse, ScreenshotSubmitError> {
		self.screenshots.request(captures)
	}

	/// Requests application shutdown through the inspector event channel.
	pub fn close_application(&self) {
		self.events.send(Events::Close);
	}
}

#[cfg(all(test, feature = "headed"))]
mod tests {
	use super::DefaultInspector;
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
