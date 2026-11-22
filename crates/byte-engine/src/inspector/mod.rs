//! Runtime inspection contracts and protocol-facing state access.
//!
//! [`DefaultInspector`] exposes factory-created handles, their tracked
//! properties, application controls, screenshots, per-tick performance
//! metrics, passive message publication headers, and events since a client's
//! cursor without choosing a transport. Untracked application values and published payloads remain opaque.

use std::{collections::HashMap, sync::Arc};

#[cfg(feature = "headed")]
use screenshot::ScreenshotBroker;
use serde::{Serialize, Serializer, ser::SerializeMap, ser::SerializeStruct};
use utils::sync::{Mutex, RwLock};

use crate::{
	application::Events,
	configuration::{Configuration, ConfigurationEvent, ConfigurationEventId, ConfigurationValue, Parameter},
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

// Only the graphics application raises the event signal and the HTTP inspector reads events; headless builds leave both
// sides unused.
#[cfg(feature = "headed")]
pub mod console;
#[cfg_attr(not(feature = "headed"), allow(dead_code))]
mod content;
pub use content::Spawned;
mod events;
#[cfg_attr(not(feature = "headed"), allow(unused_imports))]
pub(crate) use events::{EventBatch, EventCursor, EventFilter, EventSignal, EventWaitError, MAX_EVENT_WAIT};
#[cfg(feature = "headed")]
pub mod http;
mod message;
mod properties;
use message::SerializableMessage;
pub use message::{
	CAMERA_UPDATE_MESSAGE_TYPE, CONE_LIGHT_UPDATE_MESSAGE_TYPE, DELETE_MESSAGE_TYPE, DESTROY_MESSAGE_TYPE,
	DIRECTIONAL_LIGHT_UPDATE_MESSAGE_TYPE, POINT_LIGHT_UPDATE_MESSAGE_TYPE, RegisteredMessageType,
	TRANSFORMATION_UPDATE_MESSAGE_TYPE, TRIGGER_ACTION_MESSAGE_TYPE,
};
use properties::Properties;
#[cfg(feature = "headed")]
pub(crate) mod screenshot;
// The search's fuzzy matcher comes with the headed build, as the console does.
#[cfg(feature = "headed")]
mod search;
#[cfg(feature = "headed")]
pub use screenshot::{
	MAX_SCREENSHOT_CAPTURES, ScreenshotCapture, ScreenshotError, ScreenshotFormat, ScreenshotResponse, ScreenshotSelection,
	ScreenshotSubmitError, Screenshots,
};
#[cfg(feature = "headed")]
pub use search::{find_entities, fuzzy_find};
mod shape;
// Only the graphics application opens stream output and adopts requests; headless builds leave that side unused.
#[cfg_attr(not(feature = "headed"), allow(dead_code, unused_imports))]
mod stream;
pub use stream::{DebugDataStream, DebugDataStreamWriter};
#[cfg_attr(not(feature = "headed"), allow(unused_imports))]
pub(crate) use stream::{DebugDataStreams, RetainedStreams, StreamOutput};

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
	/// Each property the entity has, by name, in registration order, serialized.
	properties: Vec<(&'static str, serde_json::Value)>,
}

impl Serialize for InspectedEntity {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut entity = serializer.serialize_struct("InspectedEntity", 3)?;
		entity.serialize_field("target", &self.handle().id())?;
		entity.serialize_field("types", self.types())?;
		entity.serialize_field("properties", &PropertyMap(&self.properties))?;
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

	/// Returns every tracked property the entity has, by protocol name, serialized, in registration order.
	pub fn properties(&self) -> &[(&'static str, serde_json::Value)] {
		&self.properties
	}

	/// Returns the tracked property with this protocol name, such as `name` or `transform`, serialized, when the
	/// entity has it. Read a value of your own type with `serde_json::from_value`.
	pub fn property(&self, name: &str) -> Option<&serde_json::Value> {
		self.properties
			.iter()
			.find_map(|(property, value)| (*property == name).then_some(value))
	}
}

/// The `PropertyMap` struct serializes an entity's properties as one object, keyed by name.
struct PropertyMap<'a>(&'a [(&'static str, serde_json::Value)]);

impl Serialize for PropertyMap<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut properties = serializer.serialize_map(Some(self.0.len()))?;
		for (name, value) in self.0 {
			properties.serialize_entry(name, value)?;
		}
		properties.end()
	}
}

/// The `DefaultInspector` struct owns engine controls and runtime diagnostics shared by protocol adapters.
pub struct DefaultInspector {
	events: DefaultChannel<Events>,
	configuration: Configuration,
	/// The messages clients can post, by protocol name; the application adds its own after it shares the inspector.
	serializable_messages: RwLock<HashMap<&'static str, SerializableMessage>>,
	message_bus: MessageBus,
	message_observer: MessageObserver,
	properties: Arc<Properties>,
	#[cfg(feature = "headed")]
	screenshots: Arc<ScreenshotBroker>,
	#[cfg(feature = "headed")]
	debug_views: Option<Arc<crate::rendering::DebugViewCatalog>>,
	metrics: Arc<Metrics>,
	/// Wakes requests waiting in [`Self::wait_for_events`]; the application loop raises it.
	event_signal: Arc<EventSignal>,
	/// The latest record of each debug data stream, which the console shows.
	streams: Arc<RetainedStreams>,
	/// The world levels spawn into.
	world: MessageScope,
	/// The application's spawner, once it gave it; see [`Self::set_spawner`].
	spawner: Mutex<Option<Arc<crate::gameplay::Spawner>>>,
	/// The application's resource manager, once it gave it; see [`Self::set_resource_manager`].
	resource_manager: std::sync::OnceLock<crate::core::EntityHandle<resource_management::ResourceManager>>,
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
		let properties = Arc::new(Properties::default());
		watch_properties(&messages, &properties, &message_observer);
		Self {
			events,
			configuration,
			serializable_messages: RwLock::default(),
			message_bus,
			message_observer,
			properties,
			#[cfg(feature = "headed")]
			screenshots: Arc::new(ScreenshotBroker::new()),
			#[cfg(feature = "headed")]
			debug_views: None,
			metrics,
			event_signal: Arc::new(EventSignal::new()),
			streams: Arc::default(),
			world: messages,
			spawner: Mutex::new(None),
			resource_manager: std::sync::OnceLock::new(),
		}
	}

	/// Returns the latest record of each debug data stream. Give it to the application's stream output so records
	/// reach it.
	#[cfg_attr(not(feature = "headed"), allow(dead_code))]
	pub(crate) fn streams(&self) -> &Arc<RetainedStreams> {
		&self.streams
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

	/// Returns the signal the application loop raises after each tick and on closing, which wakes requests waiting
	/// in [`Self::wait_for_events`].
	#[cfg_attr(not(feature = "headed"), allow(dead_code))]
	pub(crate) fn event_signal(&self) -> Arc<EventSignal> {
		Arc::clone(&self.event_signal)
	}

	/// Returns the events after `after` that `filter` selects, waiting up to `wait` for one.
	///
	/// Without `after` it returns at once with a cursor at the current tick and every retained configuration
	/// update. Pass each batch's cursor to the next call. See the
	/// [inspector reference](/docs/reference/inspector#events) for the events and their rules.
	#[cfg_attr(not(feature = "headed"), allow(dead_code))]
	pub(crate) fn wait_for_events(
		&self,
		after: Option<EventCursor>,
		filter: EventFilter,
		wait: std::time::Duration,
	) -> Result<EventBatch, EventWaitError> {
		events::wait_for_events(&self.metrics, &self.configuration, &self.event_signal, after, filter, wait)
	}

	/// Returns the latest configuration event states for protocol adapters.
	pub fn configuration_events(&self) -> Vec<ConfigurationEvent> {
		self.configuration.events()
	}

	/// Returns every runtime setting the application's systems declared, by name, with the values each takes and
	/// holds, so a tool can list them and show their values.
	///
	/// Next, set one with [`Self::update_configuration`].
	pub fn configuration_parameters(&self) -> Vec<Parameter> {
		self.configuration.parameters()
	}

	/// Returns the latest state of the configuration event `id`, so a tool can report whether an update it queued with
	/// [`Self::update_configuration`] was set.
	pub fn configuration_event(&self, id: ConfigurationEventId) -> Option<ConfigurationEvent> {
		self.configuration.event(id)
	}

	/// Queues `value` for the system that owns `parameter` and returns the event that reports whether it was set.
	///
	/// The owning system applies it on a later tick. Follow the event's state through
	/// [`Self::configuration_events`] or the `configuration` events of [`Self::wait_for_events`].
	pub fn update_configuration(&self, parameter: &str, value: ConfigurationValue) -> ConfigurationEvent {
		let id = self.configuration.update(parameter, value);
		self.configuration.event(id).expect(
			"Configuration event is missing right after it was queued. The most likely cause is an event history that drops new events. Keep the newest events when trimming the history.",
		)
	}

	/// Connects the catalog the renderer publishes its debug views to, which `GET /debug-views` serves.
	#[cfg(feature = "headed")]
	pub fn set_debug_views(&mut self, debug_views: Arc<crate::rendering::DebugViewCatalog>) {
		self.debug_views = Some(debug_views);
	}

	/// Returns the declared debug views and which are shown, or an empty, disabled catalog without a renderer.
	#[cfg(feature = "headed")]
	pub fn debug_views(&self) -> crate::rendering::debug_views::DebugViewCatalogState {
		self.debug_views.as_ref().map(|catalog| catalog.state()).unwrap_or_default()
	}

	/// Tracks the property `name` of each entity from the creations on `channel`, so queries list its latest value
	/// under that name, serialized. Call it for each value a client should read, such as an NPC's current plan; the
	/// application publishes a new value by creating it again under the entity's handle. An entity's values are
	/// forgotten once it's deleted.
	///
	/// Tracking starts now, so values published before the call aren't listed until they're published again. To let a
	/// client change the value too, register the same channel with [`Self::register_message`].
	pub fn watch_property<T: Serialize + Clone + Send + Sync + 'static>(
		&self,
		name: &'static str,
		channel: DefaultChannel<CreateMessage<T>>,
	) -> Result<(), String> {
		watch(&self.properties, name, &channel)
	}

	/// Returns current factory-created entities that have a transform, filtered by an exact Rust type or attached name.
	pub fn entities(&self, entity_type: Option<&str>, name: Option<&str>) -> Vec<InspectedEntity> {
		// TEMPORARY PATCH: list only entities with a `transform` property instead of the whole factory catalog.
		// The catalog keeps every factory-created handle, including short-lived messages such as physics ray
		// queries and their responses, so scanning it made each query slower the longer the application ran.
		// This hides entities without a transform, such as input actions. Remove it once the catalog retires
		// message handles; see the Inspector catalog item in todo.md.
		let mut handles = self.properties.handles("transform");
		handles.sort_unstable_by_key(|handle| handle.id());
		let mut entities: Vec<_> = handles
			.into_iter()
			.filter_map(|handle| self.message_observer.entity(handle))
			.filter(|entity| entity_type.is_none_or(|entity_type| entity.types().contains(&entity_type)))
			.map(|entity| InspectedEntity {
				entity,
				properties: Vec::new(),
			})
			.collect();
		// The catalog preserves handle order, so property tables can join their sparse values to this snapshot.
		self.properties.read(&mut entities);
		if let Some(name) = name {
			entities.retain(|entity| entity.property("name").and_then(|value| value.as_str()) == Some(name));
		}
		entities
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

/// Tracks the engine's own properties, each published as a creation on its own route, and forgets an entity's
/// properties once it's deleted.
///
/// Watchers see each publication, whatever sent it, so physics and gameplay updates count as well as spawns. Deletions
/// drop the entity's properties and its diagnostics catalog entry, so any sender of a [`DeleteMessage`] retires the
/// handle.
fn watch_properties(messages: &MessageScope, properties: &Arc<Properties>, observer: &MessageObserver) {
	const DISTINCT: &str = "The engine's own properties have distinct names, so registering them never fails.";
	watch(properties, "name", &messages.channel::<CreateMessage<Name>>()).expect(DISTINCT);
	watch(properties, "transform", &messages.channel::<TransformationUpdate>()).expect(DISTINCT);
	// Each light property is named after the level kind whose payload it has.
	#[cfg(feature = "headed")]
	{
		watch(properties, "camera", &messages.channel::<crate::rendering::CameraUpdate>()).expect(DISTINCT);
		watch(
			properties,
			"point_light",
			&messages.channel::<crate::rendering::PointLightUpdate>(),
		)
		.expect(DISTINCT);
		watch(
			properties,
			"cone_light",
			&messages.channel::<crate::rendering::ConeLightUpdate>(),
		)
		.expect(DISTINCT);
		watch(
			properties,
			"directional_light",
			&messages.channel::<crate::rendering::DirectionalLightUpdate>(),
		)
		.expect(DISTINCT);
		watch(
			properties,
			"mesh",
			&messages.channel::<CreateMessage<crate::rendering::RenderableMesh>>(),
		)
		.expect(DISTINCT);
	}
	let deleted = Arc::clone(properties);
	let observer = observer.clone();
	messages.channel::<DeleteMessage>().watch(move |deletion| {
		deleted.forget(*deletion.handle());
		observer.forget_entity(*deletion.handle());
	});
}

/// Tracks the property `name` from the creations on `channel`: each one is the entity's latest value.
fn watch<T: Serialize + Clone + Send + Sync + 'static>(
	properties: &Properties,
	name: &'static str,
	channel: &DefaultChannel<CreateMessage<T>>,
) -> Result<(), String> {
	let record = properties.add::<T>(name)?;
	channel.watch(move |creation| record(creation.handle(), creation.data().clone()));
	Ok(())
}

#[cfg(all(test, feature = "headed"))]
mod tests {
	use std::sync::Arc;

	use math::Point;
	use serde_json::json;

	use super::DefaultInspector;
	use crate::{
		configuration::Configuration,
		core::{
			Creator as _, channel::Channel as _, channel::DefaultChannel, factory::Handle, message::DeleteMessage,
			message_bus::MessageBus,
		},
		gameplay::{Name, Transform, TransformationUpdate},
		metrics::Metrics,
		rendering::{Camera, CameraUpdate},
	};

	#[test]
	fn entity_queries_preserve_sparse_properties_and_registration_order() {
		let message_bus = MessageBus::default();
		message_bus.observe().unwrap();
		let world = message_bus.new_scope("sparse-properties-test");
		let inspector = DefaultInspector::new(
			DefaultChannel::new(),
			Configuration::new(),
			world.clone(),
			Arc::new(Metrics::new()),
		);
		inspector
			.watch_property("count", world.channel::<crate::core::factory::CreateMessage<u32>>())
			.unwrap();
		let first: Handle = world
			.create(10_u32)
			.with(Name::new("first"))
			.with(Transform::identity())
			.into();
		let second: Handle = world.create(Name::new("second")).with(Transform::identity()).into();

		let all = inspector.entities(None, None);
		assert_eq!(all.iter().map(|entity| entity.handle()).collect::<Vec<_>>(), [first, second]);
		assert_eq!(
			all[0].properties().iter().map(|(name, _)| *name).collect::<Vec<_>>(),
			["name", "transform", "count"]
		);
		assert_eq!(all[0].property("count"), Some(&json!(10)));
		assert_eq!(all[1].property("count"), None);

		// A narrow type query still returns every property of each match, in the same order as the full catalog.
		assert_eq!(inspector.entities(Some(std::any::type_name::<u32>()), None), [all[0].clone()]);
		assert_eq!(inspector.entities(None, Some("second")), [all[1].clone()]);
		assert!(
			inspector
				.entities(Some(std::any::type_name::<u32>()), Some("second"))
				.is_empty()
		);
	}

	#[test]
	fn properties_follow_spawn_replacement_and_deletion_through_the_inspector() {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let messages = message_bus.new_scope("inspected-entity-test-world");
		let world = messages.clone();
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
		assert_eq!(entities[0].property("name"), Some(&json!("crate")));
		assert_eq!(
			entities[0].property("transform"),
			Some(&json!({ "position": [1.0, 2.0, 3.0], "scale": [1.0, 1.0, 1.0], "orientation": [0.0, 0.0, 0.0, 1.0] }))
		);
		assert_eq!(
			inspector.entities(Some(std::any::type_name::<String>()), Some("crate")).len(),
			1
		);

		// Updates sent straight to the transform route, as physics does, replace the spawn transform.
		world.channel::<TransformationUpdate>().send(TransformationUpdate::new(
			handle,
			Transform::from_position(Point::new(4.0, 5.0, 6.0)),
		));
		world.factory::<Name>().derive(handle, Name::new("shipping crate"));
		assert!(inspector.entities(None, Some("crate")).is_empty());
		let entities = inspector.entities(None, Some("shipping crate"));
		assert_eq!(entities[0].handle(), handle);
		assert_eq!(
			entities[0].property("transform"),
			Some(&json!({ "position": [4.0, 5.0, 6.0], "scale": [1.0, 1.0, 1.0], "orientation": [0.0, 0.0, 0.0, 1.0] }))
		);

		world.channel::<DeleteMessage>().send(DeleteMessage::new(handle));
		assert!(inspector.entities(None, Some("shipping crate")).is_empty());
	}

	#[test]
	fn an_application_adds_properties_and_messages_to_a_shared_inspector() {
		use crate::core::factory::CreateMessage;

		#[derive(Clone, Debug, PartialEq, serde::Serialize, facet::Facet)]
		struct Plan {
			goal: String,
		}

		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let world = message_bus.new_scope("inspected-application-test-world");
		// Shared as the graphics application shares it, so registration needs no exclusive access.
		let inspector = Arc::new(DefaultInspector::new(
			DefaultChannel::new(),
			Configuration::new(),
			world.clone(),
			Arc::new(Metrics::new()),
		));
		let plans = world.channel::<CreateMessage<Plan>>();
		inspector.watch_property("plan", plans.clone()).unwrap();
		inspector.register_message("Plan", plans).unwrap();
		assert!(
			inspector
				.watch_property("plan", world.channel::<CreateMessage<Plan>>())
				.is_err(),
			"a name is registered once"
		);

		let handle: Handle = world.create(Name::new("npc")).with(Transform::identity()).into();
		inspector.post_message("Plan", handle, r#"{ "goal": "flee" }"#).unwrap();
		let entities = inspector.entities(None, Some("npc"));
		assert_eq!(entities[0].property("plan"), Some(&json!({ "goal": "flee" })));
		assert!(inspector.message_types().iter().any(|message| message.message_type == "Plan"));

		world.channel::<DeleteMessage>().send(DeleteMessage::new(handle));
		world.factory::<Name>().derive(handle, Name::new("npc"));
		world.factory::<Transform>().derive(handle, Transform::identity());
		let entities = inspector.entities(None, Some("npc"));
		assert_eq!(entities[0].property("plan"), None, "a deleted entity's values are forgotten");
	}

	#[test]
	fn a_posted_light_update_replaces_the_light_property_as_authored() {
		use crate::rendering::{ConeLight, ConeLightUpdate, LightColor, PhotometricIntensity};

		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let world = message_bus.new_scope("inspected-light-test-world");
		let inspector = DefaultInspector::new(
			DefaultChannel::new(),
			Configuration::new(),
			world.clone(),
			Arc::new(Metrics::new()),
		);
		inspector
			.register_message(super::CONE_LIGHT_UPDATE_MESSAGE_TYPE, world.channel::<ConeLightUpdate>())
			.unwrap();
		let light = ConeLight::new(
			LightColor::kelvin(4000.0),
			PhotometricIntensity::luminous_flux(800.0),
			math::Radians::new(0.3),
			math::Radians::new(0.5),
		)
		.unwrap();
		let handle: Handle = world.create(light).with(Name::new("spot")).with(Transform::identity()).into();
		let light = |inspector: &DefaultInspector| {
			let entities = inspector.entities(None, Some("spot"));
			serde_json::to_value(entities[0].property("cone_light").expect("a cone light property")).unwrap()
		};
		// The light reports what it was authored with, not what it resolved to.
		assert_eq!(light(&inspector)["color"], serde_json::json!({ "kelvin": 4000.0 }));
		assert_eq!(light(&inspector)["intensity"]["luminous_flux"]["lumens"], 800.0);

		// The property's own shape posts back, with fields changed.
		let mut payload = light(&inspector);
		payload["color"] = serde_json::json!({ "linear_srgb": [1.0, 0.5, 0.25] });
		payload["outer_angle"] = 0.75.into();
		payload["shadow_far"] = 12.0.into();
		inspector
			.post_message(super::CONE_LIGHT_UPDATE_MESSAGE_TYPE, handle, &payload.to_string())
			.unwrap();
		assert_eq!(light(&inspector), payload);

		// A cone whose inner angle isn't below its outer one is refused.
		payload["inner_angle"] = 1.0.into();
		assert!(
			inspector
				.post_message(super::CONE_LIGHT_UPDATE_MESSAGE_TYPE, handle, &payload.to_string())
				.is_err()
		);
	}

	#[test]
	fn a_posted_camera_update_replaces_the_camera_property() {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let world = message_bus.new_scope("inspected-camera-test-world");
		let inspector = DefaultInspector::new(
			DefaultChannel::new(),
			Configuration::new(),
			world.clone(),
			Arc::new(Metrics::new()),
		);
		inspector
			.register_message(super::CAMERA_UPDATE_MESSAGE_TYPE, world.channel::<CameraUpdate>())
			.unwrap();
		let handle: Handle = world
			.create(Camera::new())
			.with(Name::new("camera"))
			.with(Transform::identity())
			.into();
		let camera = |inspector: &DefaultInspector| {
			let entities = inspector.entities(None, Some("camera"));
			serde_json::to_value(entities[0].property("camera").expect("a camera property")).unwrap()
		};
		assert_eq!(camera(&inspector)["fov"], 45.0);

		// The property's own shape posts back, with one field changed.
		let mut payload = camera(&inspector);
		payload["fov"] = 70.0.into();
		payload["f_number"] = 2.8f32.into();
		inspector
			.post_message(super::CAMERA_UPDATE_MESSAGE_TYPE, handle, &payload.to_string())
			.unwrap();
		assert_eq!(camera(&inspector), payload);

		assert!(
			inspector
				.post_message(super::CAMERA_UPDATE_MESSAGE_TYPE, handle, r#"{ "near": 0 }"#)
				.is_err()
		);
	}

	#[test]
	fn an_entity_drawing_a_stored_mesh_names_it() {
		use crate::rendering::RenderableMesh;
		use crate::rendering::mesh::generator::BoxMeshGenerator;

		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let world = message_bus.new_scope("inspected-mesh-test-world");
		let inspector = DefaultInspector::new(
			DefaultChannel::new(),
			Configuration::new(),
			world.clone(),
			Arc::new(Metrics::new()),
		);
		let _crate: Handle = world
			.create(RenderableMesh::resource("props/crate.glb"))
			.with(Name::new("crate"))
			.with(Transform::identity())
			.into();
		let _block: Handle = world
			.create(RenderableMesh::generated(Box::new(BoxMeshGenerator::default())))
			.with(Name::new("block"))
			.with(Transform::identity())
			.into();
		let mesh = |name: &str| {
			let entities = inspector.entities(None, Some(name));
			serde_json::to_value(entities[0].property("mesh").expect("a mesh property")).unwrap()
		};
		assert_eq!(mesh("crate"), serde_json::json!({ "resource": "props/crate.glb" }));
		// Generated geometry has no ID to look up.
		assert_eq!(mesh("block"), serde_json::json!({}));
	}
}
