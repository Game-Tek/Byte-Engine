//! Passive diagnostics for publications and factory-created objects.
//!
//! Attach one [`MessageObserver`] through
//! [`crate::core::message_bus::MessageBus::observe`]. Publication ranges come
//! from the bus's existing route counters, so producers do no extra work.
//! Ranges preserve sequence within one route, not ordering between routes.
//! Factory hooks retain handles and Rust type names, never message payloads.
//!
//! The entity catalog is indexed by handle and updated without locks or
//! hashing: each entity is one word that packs the catalog indices of its
//! types, so cataloging a creation costs one compare-and-swap.

#![allow(
	unsafe_code,
	reason = "The entity catalog allocates its handle-indexed chunks on first touch without a lock."
)]

use std::{
	any::{TypeId, type_name},
	fmt,
	sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering},
	sync::{Arc, OnceLock},
};

use utils::{hash::HashMap, sync::Mutex};

use crate::core::{factory::Handle, message_bus::TopicSnapshot};

/// Entities per catalog chunk. Chunks are allocated as handles reach them.
const ENTITIES_PER_CHUNK: usize = 1 << 14;
/// Chunks that cover every possible handle, so no handle needs another path.
const CHUNK_TABLE_LEN: usize = (u32::MAX as usize + 1) / ENTITIES_PER_CHUNK;
/// Bits of the entity word that hold its state: the inline type count or `OVERFLOW_STATE`.
const STATE_BITS: u32 = 8;
/// Bits of one inline type index, which bounds the distinct types a catalog can index.
const INDEX_BITS: u32 = 11;
const INDEX_MASK: u64 = (1 << INDEX_BITS) - 1;
const MAX_TYPES: usize = 1 << INDEX_BITS;
/// Types one entity word holds inline before the entity moves to the side catalog.
const INLINE_TYPES: u8 = ((u64::BITS - STATE_BITS) / INDEX_BITS) as u8;
/// Entity word state meaning the entity's types live in the side catalog.
const OVERFLOW_STATE: u8 = u8::MAX;

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

/// The `ObservedType` struct identifies one factory value type inside an observer's catalog.
///
/// A factory resolves it once and passes it with every creation, so cataloging
/// never hashes a type id.
#[derive(Clone, Copy)]
pub(crate) struct ObservedType(u16);

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
	/// Allocates an empty entity catalog. Route cursors and catalog chunks grow as they are touched.
	pub(crate) fn new() -> Self {
		// SAFETY: Null pointers and empty once-locks are all-zero, and the zeroed
		// pages cost nothing until an entity touches them.
		let chunks = unsafe { Box::<[AtomicPtr<EntityChunk>]>::new_zeroed_slice(CHUNK_TABLE_LEN).assume_init() };
		Self {
			inner: Arc::new(MessageObserverInner {
				message_cursors: Mutex::new(Vec::new()),
				types: (0..MAX_TYPES).map(|_| OnceLock::new()).collect(),
				type_registry: Mutex::new(HashMap::default()),
				chunks,
				chunks_touched: AtomicUsize::new(0),
				side: Mutex::new(HashMap::default()),
			}),
		}
	}

	/// Returns lossless publication ranges since the prior call for these topic snapshots.
	pub fn drain_messages(&self, topics: &[TopicSnapshot]) -> MessageObservationBatch {
		let mut cursors = self.inner.message_cursors.lock();
		let mut messages = Vec::with_capacity(topics.len());
		for topic in topics {
			if cursors.len() <= topic.topic_id {
				cursors.resize(topic.topic_id + 1, 0);
			}
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
		let inner = &*self.inner;
		let side = inner.side.lock();
		let mut snapshot = Vec::new();
		let touched = inner.chunks_touched.load(Ordering::Acquire);
		for (chunk_index, slot) in inner.chunks[..touched].iter().enumerate() {
			let chunk = slot.load(Ordering::Acquire);
			if chunk.is_null() {
				continue;
			}
			// SAFETY: A published chunk pointer stays valid until the observer drops.
			let chunk = unsafe { &*chunk };
			for (entry_index, entry) in chunk.iter().enumerate() {
				let word = entry.load(Ordering::Acquire);
				if word == 0 {
					continue;
				}
				let handle = Handle::from_id((chunk_index * ENTITIES_PER_CHUNK + entry_index) as u32);
				let types = if entity_state(word) == OVERFLOW_STATE {
					side.get(&handle.id())
						.map(|types| types.iter().map(|&index| inner.type_name(index)).collect())
						.unwrap_or_default()
				} else {
					inline_types(word).map(|index| inner.type_name(index)).collect()
				};
				snapshot.push(ObservedEntity { handle, types });
			}
		}
		snapshot
	}

	/// Resolves the catalog identity of a factory value type, registering it on first use.
	///
	/// Next, pass the result to [`Self::observe_entity`] with every creation.
	pub(crate) fn observed_type<T: 'static>(&self) -> ObservedType {
		let inner = &*self.inner;
		let mut registry = inner.type_registry.lock();
		if let Some(&index) = registry.get(&TypeId::of::<T>()) {
			return ObservedType(index);
		}
		assert!(
			registry.len() < MAX_TYPES,
			"Message observer type limit {MAX_TYPES} reached while cataloging '{}'. The most likely cause is a loop that creates factories for generic types without bound.",
			type_name::<T>()
		);
		let index = registry.len() as u16;
		let installed = inner.types[usize::from(index)].set(type_name::<T>()).is_ok();
		debug_assert!(installed, "A catalog type slot was initialized twice");
		registry.insert(TypeId::of::<T>(), index);
		ObservedType(index)
	}

	/// Adds one semantic factory representation to the entity catalog.
	pub(crate) fn observe_entity(&self, handle: Handle, observed: ObservedType) {
		let inner = &*self.inner;
		inner.catalog(inner.entry(handle.id()), handle, observed);
	}

	/// Removes a terminally deleted handle from the current entity catalog.
	pub(crate) fn forget_entity(&self, handle: Handle) {
		let inner = &*self.inner;
		if let Some(entry) = inner.entry_if_present(handle.id())
			&& entity_state(entry.swap(0, Ordering::AcqRel)) == OVERFLOW_STATE
		{
			inner.side.lock().remove(&handle.id());
		}
	}
}

/// The `MessageObserverInner` struct owns the storage shared by producers and the inspector.
struct MessageObserverInner {
	message_cursors: Mutex<Vec<u64>>,
	/// Rust names of the indexed catalog types, registered once through `type_registry`.
	types: Box<[OnceLock<&'static str>]>,
	/// Catalog index for each type id. Only type registration takes this lock.
	type_registry: Mutex<HashMap<TypeId, u16>>,
	/// Handle-indexed entity words, allocated one chunk at a time on first touch.
	chunks: Box<[AtomicPtr<EntityChunk>]>,
	/// Chunk slots that may hold a chunk, so snapshots skip the untouched rest of the table.
	chunks_touched: AtomicUsize,
	/// Types of entities with more representations than one word holds.
	side: Mutex<HashMap<u32, Vec<u16>>>,
}

/// The `EntityChunk` type holds the entity words for one contiguous handle range.
type EntityChunk = [AtomicU64; ENTITIES_PER_CHUNK];

impl MessageObserverInner {
	fn type_name(&self, index: u16) -> &'static str {
		self.types[usize::from(index)]
			.get()
			.expect("An indexed catalog type is registered before any entity refers to it")
	}

	/// Returns the entity word of a handle, allocating its chunk on first touch.
	fn entry(&self, id: u32) -> &AtomicU64 {
		let (chunk_index, entry_index) = (id as usize / ENTITIES_PER_CHUNK, id as usize % ENTITIES_PER_CHUNK);
		let slot = &self.chunks[chunk_index];
		let mut chunk = slot.load(Ordering::Acquire);
		if chunk.is_null() {
			// SAFETY: An all-zero bit pattern is a valid `AtomicU64`, and zeroed heap
			// pages cost nothing until an entity touches them.
			let fresh = Box::into_raw(unsafe { Box::<EntityChunk>::new_zeroed().assume_init() });
			match slot.compare_exchange(std::ptr::null_mut(), fresh, Ordering::AcqRel, Ordering::Acquire) {
				Ok(_) => {
					self.chunks_touched.fetch_max(chunk_index + 1, Ordering::AcqRel);
					chunk = fresh;
				}
				Err(existing) => {
					// SAFETY: `fresh` was never published, so this is its only owner.
					drop(unsafe { Box::from_raw(fresh) });
					chunk = existing;
				}
			}
		}
		// SAFETY: A published chunk pointer stays valid until the observer drops.
		let chunk = unsafe { &*chunk };
		&chunk[entry_index]
	}

	/// Returns the entity word of a handle whose chunk already exists.
	fn entry_if_present(&self, id: u32) -> Option<&AtomicU64> {
		let chunk = self.chunks[id as usize / ENTITIES_PER_CHUNK].load(Ordering::Acquire);
		if chunk.is_null() {
			return None;
		}
		// SAFETY: A published chunk pointer stays valid until the observer drops.
		let chunk = unsafe { &*chunk };
		Some(&chunk[id as usize % ENTITIES_PER_CHUNK])
	}

	/// Appends a type to an entity word, or moves the entity to the side catalog when the word is full.
	fn catalog(&self, entry: &AtomicU64, handle: Handle, observed: ObservedType) {
		let mut word = entry.load(Ordering::Acquire);
		loop {
			let state = entity_state(word);
			if state == OVERFLOW_STATE {
				return self.catalog_in_side(handle, observed);
			}
			if inline_types(word).any(|index| index == observed.0) {
				return;
			}
			if state == INLINE_TYPES {
				return self.promote_to_side(entry, handle, observed);
			}
			// Replace the count byte and append the index at the next inline position.
			let next = (word & !0xFF) | u64::from(state + 1) | (u64::from(observed.0) << type_shift(state));
			match entry.compare_exchange_weak(word, next, Ordering::AcqRel, Ordering::Acquire) {
				Ok(_) => return,
				Err(actual) => word = actual,
			}
		}
	}

	/// Moves a full entity word's types to the side catalog together with one more type.
	#[cold]
	#[inline(never)]
	fn promote_to_side(&self, entry: &AtomicU64, handle: Handle, observed: ObservedType) {
		let mut side = self.side.lock();
		// Another thread may have promoted the entity first; the swap reads the final inline set.
		let word = entry.swap(u64::from(OVERFLOW_STATE), Ordering::AcqRel);
		let types = side.entry(handle.id()).or_default();
		if entity_state(word) != OVERFLOW_STATE {
			types.extend(inline_types(word));
		}
		push_unique(types, observed);
	}

	/// Appends a type to an entity that already lives in the side catalog.
	#[cold]
	#[inline(never)]
	fn catalog_in_side(&self, handle: Handle, observed: ObservedType) {
		push_unique(self.side.lock().entry(handle.id()).or_default(), observed);
	}
}

impl Drop for MessageObserverInner {
	fn drop(&mut self) {
		for slot in self.chunks.iter_mut() {
			let chunk = *slot.get_mut();
			if !chunk.is_null() {
				// SAFETY: Each published chunk came from `Box::into_raw` and is freed exactly once here.
				drop(unsafe { Box::from_raw(chunk) });
			}
		}
	}
}

/// Returns the state byte of an entity word: its inline type count or `OVERFLOW_STATE`.
fn entity_state(word: u64) -> u8 {
	word as u8
}

/// Returns the bit position of the inline type index at `position`.
fn type_shift(position: u8) -> u32 {
	STATE_BITS + INDEX_BITS * u32::from(position)
}

/// Returns the catalog indices packed in an entity word, in first-published order.
fn inline_types(word: u64) -> impl Iterator<Item = u16> {
	(0..entity_state(word).min(INLINE_TYPES)).map(move |position| ((word >> type_shift(position)) & INDEX_MASK) as u16)
}

fn push_unique(types: &mut Vec<u16>, observed: ObservedType) {
	if !types.contains(&observed.0) {
		types.push(observed.0);
	}
}

#[cfg(test)]
mod tests {
	use super::{INLINE_TYPES, MessageObserver};
	use crate::core::factory::Handle;

	/// Catalogs `count` distinct marker types under one handle through the crate-private hooks.
	fn catalog_distinct_types(observer: &MessageObserver, handle: Handle, count: usize) {
		macro_rules! marker_types {
			($($marker:ident),*) => {{
				$(struct $marker;)*
				let catalog: &[fn(&MessageObserver, Handle)] = &[$(|observer, handle| {
					observer.observe_entity(handle, observer.observed_type::<$marker>());
				}),*];
				for record in catalog.iter().take(count) {
					record(observer, handle);
				}
			}};
		}
		marker_types!(A, B, C, D, E, F, G, H, I, J);
	}

	#[test]
	fn an_entity_keeps_every_type_in_publication_order_beyond_the_inline_word() {
		let observer = MessageObserver::new();
		let handle = Handle::from_id(5);
		let count = usize::from(INLINE_TYPES) + 3;

		catalog_distinct_types(&observer, handle, count);
		// Cataloging the same types again must not duplicate them.
		catalog_distinct_types(&observer, handle, count);

		let entities = observer.entities();
		assert_eq!(entities.len(), 1);
		assert_eq!(entities[0].handle(), handle);
		let types = entities[0].types();
		assert_eq!(types.len(), count);
		assert!(types[0].ends_with("::A") && types[count - 1].ends_with("::H"), "{types:?}");
	}

	#[test]
	fn forgotten_entities_leave_the_catalog_whatever_their_size() {
		let observer = MessageObserver::new();
		let small = Handle::from_id(1);
		let large = Handle::from_id(2);
		let far = Handle::from_id(u32::MAX - 1);
		catalog_distinct_types(&observer, small, 1);
		catalog_distinct_types(&observer, large, usize::from(INLINE_TYPES) + 1);
		catalog_distinct_types(&observer, far, 2);
		assert_eq!(
			observer.entities().iter().map(|entity| entity.handle()).collect::<Vec<_>>(),
			[small, large, far]
		);

		observer.forget_entity(large);
		observer.forget_entity(far);
		observer.forget_entity(small);
		observer.forget_entity(Handle::from_id(7_000_000));

		assert!(observer.entities().is_empty());
	}
}
