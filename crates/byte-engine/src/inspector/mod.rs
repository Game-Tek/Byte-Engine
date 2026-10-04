//! Runtime inspection contracts and protocol-facing state access.
//!
//! [`DefaultInspector`] exposes factory-created handles, their tracked
//! properties, application controls, screenshots, per-tick performance
//! metrics, and passive message publication headers without choosing a
//! transport. Untracked application values and published payloads remain opaque.

use std::{collections::HashMap, sync::Arc};

use math::Point;
#[cfg(feature = "headed")]
use screenshot::ScreenshotBroker;
use serde::{Serialize, Serializer, ser::SerializeMap, ser::SerializeStruct};
use utils::sync::Mutex;

use crate::{
	application::Events,
	configuration::{Configuration, ConfigurationEvent},
	core::{
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Handle},
		message::DeleteMessage,
		message_bus::{MessageBus, MessageScope},
		message_observer::{MessageObserver, ObservedEntity},
	},
	gameplay::{Name, TransformationUpdate},
	metrics::Metrics,
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

/// The `InspectedEntity` struct describes one current entity and the properties the inspector tracks for it.
///
/// Read a property by its protocol name with [`Self::property`].
#[derive(Clone, Debug, PartialEq)]
pub struct InspectedEntity {
	entity: ObservedEntity,
	properties: InspectedProperties,
}

/// The `InspectedValue` enum carries one tracked property value in a transport-neutral form.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InspectedValue<'a> {
	/// Text, such as an attached [`Name`].
	Text(&'a str),
	/// A world-space location, serialized as `[x, y, z]`.
	Point(Point),
}

impl Serialize for InspectedValue<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self {
			Self::Text(text) => serializer.serialize_str(text),
			Self::Point(point) => [point.x(), point.y(), point.z()].serialize(serializer),
		}
	}
}

/// The `InspectedProperties` struct holds the latest published value of each tracked property of one entity.
#[derive(Clone, Debug, Default, PartialEq)]
struct InspectedProperties {
	/// The latest [`Name`] derived for the entity.
	name: Option<Name>,
	/// The position of the latest [`Transform`](crate::gameplay::Transform) published for the entity.
	position: Option<Point>,
}

impl InspectedProperties {
	/// Returns each present property with its protocol name, in a stable order.
	fn iter(&self) -> impl Iterator<Item = (&'static str, InspectedValue<'_>)> {
		[
			self.name.as_ref().map(|name| ("name", InspectedValue::Text(name.as_str()))),
			self.position.map(|position| ("position", InspectedValue::Point(position))),
		]
		.into_iter()
		.flatten()
	}
}

impl Serialize for InspectedEntity {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut entity = serializer.serialize_struct("InspectedEntity", 3)?;
		entity.serialize_field("target", &self.handle().id())?;
		entity.serialize_field("types", self.types())?;
		entity.serialize_field("properties", &self.properties)?;
		entity.end()
	}
}

impl Serialize for InspectedProperties {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut properties = serializer.serialize_map(None)?;
		for (name, value) in self.iter() {
			properties.serialize_entry(name, &value)?;
		}
		properties.end()
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

	/// Returns the tracked property with this protocol name, such as `name` or `position`, when the entity has it.
	pub fn property(&self, name: &str) -> Option<InspectedValue<'_>> {
		self.properties
			.iter()
			.find_map(|(property, value)| (property == name).then_some(value))
	}
}

/// The `DefaultInspector` struct owns engine controls and runtime diagnostics shared by protocol adapters.
pub struct DefaultInspector {
	events: DefaultChannel<Events>,
	configuration: Configuration,
	serializable_messages: HashMap<&'static str, SerializableMessage>,
	message_bus: MessageBus,
	message_observer: MessageObserver,
	properties: Arc<Mutex<HashMap<Handle, InspectedProperties>>>,
	#[cfg(feature = "headed")]
	screenshots: Arc<ScreenshotBroker>,
	metrics: Arc<Metrics>,
}

impl DefaultInspector {
	/// Creates an inspector backend that can publish controls and inspect one shared message bus.
	///
	/// Register the application and world listeners before passing their routes so
	/// inspector requests cannot be published without a consumer. Attach message
	/// observation before acquiring any routes in `messages`. Next, call
	/// [`Self::register_message`] with each supported destination channel before sharing
	/// the inspector with a protocol adapter. Spawn entities after construction
	/// because property tracking is future-only. `metrics` is the application's
	/// collector, which `GET /metrics` and `GET /metrics/frames` read.
	pub fn new(
		events: DefaultChannel<Events>,
		configuration: Configuration,
		messages: MessageScope,
		metrics: Arc<Metrics>,
	) -> Self {
		let message_bus = messages.message_bus().clone();
		let message_observer = message_bus.observer().unwrap_or_else(|| {
			panic!(
				"Inspector message observation is unavailable. The most likely cause is that MessageBus::observe was not called before acquiring application routes."
			)
		});
		let properties = Arc::new(Mutex::new(HashMap::new()));
		watch_properties(&messages, &properties);
		Self {
			events,
			configuration,
			serializable_messages: HashMap::new(),
			message_bus,
			message_observer,
			properties,
			#[cfg(feature = "headed")]
			screenshots: Arc::new(ScreenshotBroker::new()),
			metrics,
		}
	}

	/// Returns the per-tick CPU and GPU timings of the application.
	pub fn metrics(&self) -> &Metrics {
		&self.metrics
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
		let properties = self.properties.lock();
		self.message_observer
			.entities()
			.into_iter()
			.filter_map(|entity| {
				if entity_type.is_some_and(|entity_type| !entity.types().contains(&entity_type)) {
					return None;
				}
				let entity_properties = properties.get(&entity.handle()).cloned().unwrap_or_default();
				if name.is_some_and(|name| {
					entity_properties
						.name
						.as_ref()
						.is_none_or(|entity_name| entity_name.as_str() != name)
				}) {
					return None;
				}
				Some(InspectedEntity {
					entity,
					properties: entity_properties,
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

/// Keeps `properties` current by watching the scope's routes for every tracked value.
///
/// Watchers see each publication, whatever sent it, so physics and gameplay
/// updates count as well as spawns. Deletions drop the entity's properties.
fn watch_properties(messages: &MessageScope, properties: &Arc<Mutex<HashMap<Handle, InspectedProperties>>>) {
	let names = Arc::clone(properties);
	messages.channel::<CreateMessage<Name>>().watch(move |creation| {
		names.lock().entry(creation.handle()).or_default().name = Some(creation.data().clone());
	});
	let positions = Arc::clone(properties);
	messages.channel::<TransformationUpdate>().watch(move |update| {
		positions.lock().entry(update.handle()).or_default().position = Some(update.transform().get_position());
	});
	let deleted = Arc::clone(properties);
	messages.channel::<DeleteMessage>().watch(move |deletion| {
		deleted.lock().remove(deletion.handle());
	});
}

#[cfg(all(test, feature = "headed"))]
mod tests {
	use std::sync::Arc;

	use math::Point;

	use super::{DefaultInspector, InspectedValue};
	use crate::{
		configuration::Configuration,
		core::{Creator as _, channel::Channel as _, channel::DefaultChannel, factory::Handle, message_bus::MessageBus},
		gameplay::{DefaultWorld, Name, Transform, TransformationUpdate},
		metrics::Metrics,
	};

	#[test]
	fn properties_follow_spawn_replacement_and_deletion_through_the_inspector() {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("inspected-entity-test-world");
		let world = DefaultWorld::with_messages(messages.clone());
		let inspector = DefaultInspector::new(
			DefaultChannel::new(),
			Configuration::new(),
			messages,
			Arc::new(Metrics::new()),
		);

		let handle: Handle = world
			.create(String::from("crate-model"))
			.with(Name::new("crate"))
			.with(Transform::from_position(Point::new(1.0, 2.0, 3.0)))
			.into();

		let entities = inspector.entities(None, Some("crate"));
		assert_eq!(entities.len(), 1);
		assert_eq!(entities[0].handle(), handle);
		assert_eq!(entities[0].property("name"), Some(InspectedValue::Text("crate")));
		assert_eq!(
			entities[0].property("position"),
			Some(InspectedValue::Point(Point::new(1.0, 2.0, 3.0)))
		);
		assert_eq!(
			inspector.entities(Some(std::any::type_name::<String>()), Some("crate")).len(),
			1
		);

		// Updates sent straight to the transform route, as physics does, replace the spawn position.
		world.transforms_channel().send(TransformationUpdate::new(
			handle,
			Transform::from_position(Point::new(4.0, 5.0, 6.0)),
		));
		world.factory::<Name>().derive(handle, Name::new("shipping crate"));
		assert!(inspector.entities(None, Some("crate")).is_empty());
		let entities = inspector.entities(None, Some("shipping crate"));
		assert_eq!(entities[0].handle(), handle);
		assert_eq!(
			entities[0].property("position"),
			Some(InspectedValue::Point(Point::new(4.0, 5.0, 6.0)))
		);

		world.delete(handle);
		assert!(inspector.entities(None, Some("shipping crate")).is_empty());
	}
}
