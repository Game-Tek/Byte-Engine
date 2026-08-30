//! Runtime inspection contracts and protocol-facing state access.
//!
//! The [`Inspector`] exposes factory-created handles and passive message
//! publication headers without retaining application values or published
//! payloads. Registered post types deserialize their payloads through Facet.
//! Protocol adapters should query this object rather than reaching into
//! application subsystems directly.

use std::{collections::HashMap, sync::Arc};

#[cfg(feature = "headed")]
use screenshot::ScreenshotBroker;

use crate::{
	application::Events,
	configuration::{Configuration, ConfigurationEvent},
	core::{
		channel::{Channel, DefaultChannel},
		message_bus::{MessageBus, MessageScope},
		message_observer::{MessageObserver, ObservedEntity},
	},
};

#[cfg(feature = "headed")]
#[doc(hidden)]
pub mod http;
mod message;
use message::SerializableMessagePoster;
pub use message::TRANSFORMATION_UPDATE_MESSAGE_TYPE;
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

/// The `Inspector` struct owns application controls and passive runtime diagnostics shared by protocol adapters.
pub struct Inspector {
	events: DefaultChannel<Events>,
	configuration: Configuration,
	messages: MessageScope,
	serializable_messages: HashMap<&'static str, SerializableMessagePoster>,
	message_bus: MessageBus,
	message_observer: MessageObserver,
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
	/// inspector with a protocol adapter.
	pub fn new(events: DefaultChannel<Events>, configuration: Configuration, messages: MessageScope) -> Self {
		let message_bus = messages.message_bus().clone();
		let message_observer = message_bus.observer().unwrap_or_else(|| {
			panic!(
				"Inspector message observation is unavailable. The most likely cause is that MessageBus::observe was not called before acquiring application routes."
			)
		});
		Self {
			events,
			configuration,
			messages,
			serializable_messages: HashMap::new(),
			message_bus,
			message_observer,
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

	/// Returns current factory-created entities, optionally filtered by a complete Rust type name.
	pub fn entities(&self, entity_type: Option<&str>) -> Vec<ObservedEntity> {
		let mut entities = self.message_observer.entities();
		if let Some(entity_type) = entity_type {
			entities.retain(|entity| entity.types().contains(&entity_type));
		}
		entities
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
