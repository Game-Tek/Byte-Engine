//! Passive diagnostics for publications and factory-created objects.
//!
//! Attach one [`MessageObserver`] through
//! [`crate::core::message_bus::MessageBus::observe`]. Publication ranges come
//! from the bus's existing route counters, so producers do no extra work.
//! Ranges preserve sequence within one route, not ordering between routes.
//! Factory hooks retain handles and Rust type names, never message payloads or
//! created values.

use std::{
	any::{TypeId, type_name},
	collections::HashMap,
	fmt,
	sync::Arc,
};

use smallvec::SmallVec;
use utils::sync::Mutex;

use crate::core::{factory::Handle, message_bus::TopicSnapshot};

const INLINE_ENTITY_TYPE_CAPACITY: usize = 4;

/// The `MessageObservationError` enum explains why passive observation could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageObservationError {
	/// This bus already has its one passive observer.
	AlreadyAttached,
	/// At least one typed route was registered before observation started.
	RoutesAlreadyRegistered,
}

impl fmt::Display for MessageObservationError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::AlreadyAttached => write!(
				formatter,
				"The message bus already has an observer. The most likely cause is that more than one diagnostics owner tried to drain the same publication trace."
			),
			Self::RoutesAlreadyRegistered => write!(
				formatter,
				"Message observation started after route registration. The most likely cause is that the inspector was attached after a channel or factory was acquired."
			),
		}
	}
}

impl std::error::Error for MessageObservationError {}

/// The `MessageObservation` struct identifies a contiguous range of successful publications without retaining payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageObservation {
	topic_id: usize,
	first_sequence: u64,
	count: u64,
}

impl MessageObservation {
	/// Returns the stable bus-local route identifier used by topic diagnostics.
	pub fn topic_id(self) -> usize {
		self.topic_id
	}

	/// Returns the first zero-based publication sequence in this range.
	pub fn first_sequence(self) -> u64 {
		self.first_sequence
	}

	/// Returns the number of consecutive publications in this range.
	pub fn count(self) -> u64 {
		self.count
	}
}

/// The `MessageObservationBatch` struct carries one lossless snapshot of publication ranges since the prior drain.
#[derive(Debug, PartialEq, Eq)]
pub struct MessageObservationBatch {
	messages: Vec<MessageObservation>,
}

impl MessageObservationBatch {
	/// Returns one range for each route that published since the prior drain.
	pub fn messages(&self) -> &[MessageObservation] {
		&self.messages
	}
}

/// The `ObservedEntity` struct describes one factory handle and every representation created for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedEntity {
	handle: Handle,
	types: Vec<&'static str>,
}

impl ObservedEntity {
	/// Returns the stable identity shared by the entity's representations.
	pub fn handle(&self) -> Handle {
		self.handle
	}

	/// Returns the Rust type names in their first-published order.
	pub fn types(&self) -> &[&'static str] {
		&self.types
	}
}

/// The `MessageObserver` struct provides passive publication ranges and a factory-backed entity catalog.
///
/// Drain publication ranges through [`Self::drain_messages`]. Query current
/// factory handles through [`Self::entities`]. Neither path retains message
/// payloads or factory-created values.
#[derive(Clone)]
pub struct MessageObserver {
	inner: Arc<MessageObserverInner>,
}

impl MessageObserver {
	/// Allocates route cursors and an empty entity catalog.
	pub(crate) fn new(max_topics: usize) -> Self {
		Self {
			inner: Arc::new(MessageObserverInner {
				message_cursors: Mutex::new(vec![0; max_topics].into_boxed_slice()),
				entities: Mutex::new(HashMap::new()),
			}),
		}
	}

	/// Returns lossless publication ranges since the prior call for these topic snapshots.
	pub fn drain_messages(&self, topics: &[TopicSnapshot]) -> MessageObservationBatch {
		let mut cursors = self.inner.message_cursors.lock();
		let mut messages = Vec::with_capacity(topics.len());
		for topic in topics {
			let cursor = &mut cursors[topic.topic_id];
			// Concurrent drains may acquire an older topic snapshot after another
			// request has already advanced this route's shared cursor.
			if topic.published <= *cursor {
				continue;
			}
			messages.push(MessageObservation {
				topic_id: topic.topic_id,
				first_sequence: *cursor,
				count: topic.published - *cursor,
			});
			*cursor = topic.published;
		}

		MessageObservationBatch { messages }
	}

	/// Returns a handle-sorted snapshot of every current factory-created entity.
	pub fn entities(&self) -> Vec<ObservedEntity> {
		let entities = self.inner.entities.lock();
		let mut snapshot = entities
			.iter()
			.map(|(&handle, types)| ObservedEntity {
				handle,
				types: types.iter().map(|entity_type| entity_type.name).collect(),
			})
			.collect::<Vec<_>>();
		snapshot.sort_unstable_by_key(|entity| entity.handle.id());
		snapshot
	}

	/// Adds one semantic factory representation without retaining the created value.
	pub(crate) fn observe_entity<T: 'static>(&self, handle: Handle) {
		let entity_type = ObservedEntityType {
			id: TypeId::of::<T>(),
			name: type_name::<T>(),
		};
		let mut entities = self.inner.entities.lock();
		let types = entities.entry(handle).or_default();
		if types.iter().all(|existing| existing.id != entity_type.id) {
			types.push(entity_type);
		}
	}

	/// Removes a terminally deleted handle from the current entity catalog.
	pub(crate) fn forget_entity(&self, handle: Handle) {
		self.inner.entities.lock().remove(&handle);
	}
}

/// The `MessageObserverInner` struct owns the storage shared by producers and the inspector.
struct MessageObserverInner {
	message_cursors: Mutex<Box<[u64]>>,
	entities: Mutex<HashMap<Handle, SmallVec<[ObservedEntityType; INLINE_ENTITY_TYPE_CAPACITY]>>>,
}

#[derive(Clone, Copy)]
/// The `ObservedEntityType` struct de-duplicates one Rust representation without exposing `TypeId`.
struct ObservedEntityType {
	id: TypeId,
	name: &'static str,
}
