//! Runtime inspection contracts and protocol-facing state access.
//!
//! The [`Inspector`] exposes factory-created handles, attached [`Name`] values,
//! and passive message publication headers. Other application values and
//! published payloads remain opaque. Registered post types deserialize their
//! payloads through Facet. Protocol adapters should query this object rather
//! than reaching into application subsystems directly.

use std::{collections::HashMap, sync::Arc};

#[cfg(feature = "headed")]
use screenshot::ScreenshotBroker;
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
use message::SerializableMessagePoster;
pub use message::{DELETE_MESSAGE_TYPE, DESTROY_MESSAGE_TYPE, TRANSFORMATION_UPDATE_MESSAGE_TYPE};
#[cfg(feature = "headed")]
pub(crate) mod screenshot;

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

/// The `InspectedMessage` struct resolves one passive publication to its scope and Rust message type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectedMessage {
	topic_id: usize,
	scope: Arc<str>,
	message_type: &'static str,
	first_sequence: u64,
	count: u64,
}

impl InspectedMessage {
	/// Returns the stable bus-local route identifier.
	pub fn topic_id(&self) -> usize {
		self.topic_id
	}

	/// Returns the diagnostic name of the route's owning scope.
	pub fn scope(&self) -> &str {
		&self.scope
	}

	/// Returns the complete Rust type name, including generic arguments.
	pub fn message_type(&self) -> &'static str {
		self.message_type
	}

	/// Returns the first zero-based publication sequence in this range.
	pub fn first_sequence(&self) -> u64 {
		self.first_sequence
	}

	/// Returns the number of consecutive publications in this range.
	pub fn count(&self) -> u64 {
		self.count
	}
}

/// The `InspectedMessageBatch` struct carries resolved publication ranges since the prior drain.
#[derive(Debug, PartialEq, Eq)]
pub struct InspectedMessageBatch {
	messages: Vec<InspectedMessage>,
}

impl InspectedMessageBatch {
	/// Returns one range for each route that published since the prior drain.
	pub fn messages(&self) -> &[InspectedMessage] {
		&self.messages
	}
}

/// The `InspectedEntity` struct describes one current entity and its optional human-readable name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectedEntity {
	entity: ObservedEntity,
	name: Option<Name>,
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

/// The `Inspector` struct owns application controls and passive runtime diagnostics shared by protocol adapters.
pub struct Inspector {
	events: DefaultChannel<Events>,
	configuration: Configuration,
	messages: MessageScope,
	serializable_messages: HashMap<&'static str, SerializableMessagePoster>,
	message_bus: MessageBus,
	message_observer: MessageObserver,
	entity_names: Arc<Mutex<HashMap<Handle, Name>>>,
	#[cfg(feature = "headed")]
	screenshots: Arc<ScreenshotBroker>,
}

impl Inspector {
	/// Creates an inspector that can publish controls and inspect one shared message bus.
	///
	/// Register the application and world listeners before passing their routes so
	/// inspector requests cannot be published without a consumer. Attach message
	/// observation before acquiring any routes in `messages`. Next, call
	/// [`Self::register_message`] for each supported post type before sharing the
	/// inspector with a protocol adapter. Spawn named entities after construction
	/// because name collection is future-only.
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
			messages,
			serializable_messages: HashMap::new(),
			message_bus,
			message_observer,
			entity_names,
			#[cfg(feature = "headed")]
			screenshots: Arc::new(ScreenshotBroker::new()),
		}
	}

	/// Returns the bounded screenshot exchange shared with the graphics application.
	#[cfg(feature = "headed")]
	pub(crate) fn screenshots(&self) -> Arc<ScreenshotBroker> {
		Arc::clone(&self.screenshots)
	}

	/// Returns the latest configuration event states for protocol adapters.
	pub fn configuration_events(&self) -> Vec<ConfigurationEvent> {
		self.configuration.events()
	}

	/// Returns current factory-created entities filtered by an exact Rust type or attached name.
	pub fn entities(&self, entity_type: Option<&str>, name: Option<&str>) -> Vec<InspectedEntity> {
		let entities = self.message_observer.entities();
		let names = self.entity_names.lock();
		entities
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
	///
	/// Payloads remain opaque because general engine messages do not require a
	/// serialization or reflection contract. Each result preserves sequence
	/// within its route but does not claim ordering between routes.
	pub fn drain_messages(&self) -> InspectedMessageBatch {
		let topic_snapshots = self.message_bus.topics();
		let batch = self.message_observer.drain_messages(&topic_snapshots);
		let mut topics = vec![None; self.message_bus.config().max_topics];
		for topic in topic_snapshots {
			let topic_id = topic.topic_id;
			topics[topic_id] = Some(topic);
		}
		let messages = batch
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
			.collect();

		InspectedMessageBatch { messages }
	}

	/// Requests application shutdown through the inspector event channel.
	pub fn close_application(&self) {
		self.events.send(Events::Close);
	}
}

#[cfg(all(test, feature = "headed"))]
mod tests {
	use super::Inspector;
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
		let inspector = Inspector::new(DefaultChannel::new(), Configuration::new(), messages);

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
