//! Chunked storage shared by the engine's typed message routes.
//!
//! Create one [`MessageBus`] during application startup, call
//! [`MessageBus::begin_tick`] once per tick, then create a [`MessageScope`]
//! for each independently owned group of channels. Message types are
//! registered lazily the first time a channel requests them. Every route
//! borrows fixed-size chunks from the bus's one pool as it publishes and
//! returns them once every listener has read them, so no route needs its own
//! capacity and a burst on one type only takes chunks that are free.
//!
//! Publication and reading are lock-free. A publisher reserves a slot with one
//! compare-and-swap on the route's current chunk and commits it with one
//! release store. A listener reads with one acquire load per message. The bus
//! takes its control lock only to attach a chunk to a route, to register a
//! route or a listener, and inside [`MessageBus::begin_tick`].
//!
//! When the pool has no free chunk, a publisher waits instead of overwriting
//! or dropping a message. After one second it logs which routes hold the pool
//! and where their slowest listener was created. A system that stops reading
//! its messages must drop its listener so the chunks behind it return to the
//! pool.

#![allow(
	unsafe_code,
	reason = "The shared chunk pool needs typed access to validated raw payload slots."
)]

use std::{
	alloc::{Layout, alloc_zeroed, dealloc, handle_alloc_error},
	any::{Any, TypeId, type_name},
	fmt::{self, Write as _},
	marker::PhantomData,
	panic::Location,
	ptr::NonNull,
	sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering, fence},
	sync::{Arc, OnceLock},
	time::{Duration, Instant},
};

use utils::{
	hash::HashMap,
	sync::{Mutex, MutexGuard},
};

use crate::core::{
	channel::DefaultChannel,
	factory::Factory,
	message_observer::{MessageObservationError, MessageObserver, ObservedType},
};

/// Bytes in one pool chunk. A chunk holds whole slots of one message type, so
/// one message must fit in a chunk together with its slot header.
pub const CHUNK_BYTES: usize = 64 * 1024;
/// Alignment of every chunk, which bounds the alignment a message type may require.
const CHUNK_ALIGNMENT: usize = 4096;
/// Bytes of the commit stamp in front of every payload.
const STAMP_BYTES: usize = 8;
/// Bytes of reader accounting that follow the stamp for payloads with destructors.
const ACCOUNTING_BYTES: usize = 8;
/// Chunks a bus needs so one route can seal a full chunk by attaching the next one.
const MINIMUM_CHUNKS: usize = 2;
/// Marks an absent chunk index in chain links, the free list, and route cursors.
const NONE: u32 = u32::MAX;
/// Marks a slot whose retained payload has been destroyed.
const GONE: u32 = u32::MAX;
/// Tick start of a route on a bus without ticks: every record is immediately reclaimable.
const TICKLESS: u64 = u64::MAX;
/// Low bits of a slot stamp that carry the sequence; the high bits carry the route.
const STAMP_SEQUENCE_BITS: u32 = 48;
const STAMP_SEQUENCE_MASK: u64 = (1 << STAMP_SEQUENCE_BITS) - 1;
/// Routes a bus can register before stamps could collide between a chunk's owners.
const MAX_TOPICS: usize = (1 << (u64::BITS - STAMP_SEQUENCE_BITS)) - 1;
/// Longest pause, in spin iterations, a publisher takes after losing a slot reservation race.
const RESERVATION_BACKOFF_LIMIT: u32 = 256;
/// Delay before a publisher waiting on an exhausted pool reports it, and between later reports.
const EXHAUSTED_POOL_FIRST_REPORT: Duration = Duration::from_secs(1);
const EXHAUSTED_POOL_REPORT_INTERVAL: Duration = Duration::from_secs(5);

/// The `MessageBusConfig` struct defines the storage and tick behavior a bus is created with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageBusConfig {
	/// Bytes of chunk storage shared by every route. Rounded down to whole chunks of [`CHUNK_BYTES`].
	pub capacity: usize,
	/// Whether the owner calls [`MessageBus::begin_tick`] once per tick.
	///
	/// On a ticking bus a listener replays the current tick's earlier messages
	/// and chunks stay until their tick ends. A bus without ticks keeps
	/// listeners future-only and recycles chunks as soon as every listener has
	/// read them, which suits standalone channels and tests.
	pub ticks: bool,
}

impl Default for MessageBusConfig {
	fn default() -> Self {
		Self {
			capacity: 64 * 1024 * 1024,
			ticks: true,
		}
	}
}

impl MessageBusConfig {
	/// Creates a ticking configuration with the supplied pool size in bytes.
	///
	/// Next, pass the result to [`MessageBus::new`].
	pub fn new(capacity: usize) -> Self {
		Self {
			capacity,
			..Self::default()
		}
	}

	/// Validates the pool size and returns the number of chunks it provides.
	fn validate(self) -> Result<usize, MessageBusConfigError> {
		let chunks = self.capacity / CHUNK_BYTES;
		if chunks < MINIMUM_CHUNKS {
			return Err(MessageBusConfigError::CapacityTooSmall {
				capacity: self.capacity,
				minimum: MINIMUM_CHUNKS * CHUNK_BYTES,
			});
		}
		Ok(chunks)
	}
}

/// The `MessageBusConfigError` enum explains why startup storage cannot be allocated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageBusConfigError {
	/// The pool cannot hold the two chunks one route needs to seal a full chunk.
	CapacityTooSmall { capacity: usize, minimum: usize },
}

impl fmt::Display for MessageBusConfigError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::CapacityTooSmall { capacity, minimum } => write!(
				formatter,
				"Message bus capacity {capacity} is below the {minimum} byte minimum. The most likely cause is a messages.capacity parameter smaller than two chunks."
			),
		}
	}
}

impl std::error::Error for MessageBusConfigError {}

/// The `MessageRouteError` enum explains why a lazy typed route cannot be acquired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageRouteError {
	/// One message and its slot header cannot fit in a chunk.
	MessageTooLarge {
		message_type: &'static str,
		message_bytes: usize,
		chunk_bytes: usize,
	},
	/// The message requires stricter alignment than a chunk provides.
	MessageOveraligned {
		message_type: &'static str,
		required_alignment: usize,
		chunk_alignment: usize,
	},
}

impl fmt::Display for MessageRouteError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::MessageTooLarge {
				message_type,
				message_bytes,
				chunk_bytes,
			} => write!(
				formatter,
				"Message type '{message_type}' needs {message_bytes} bytes, but one pool chunk has {chunk_bytes} bytes. The most likely cause is a message that stores a large value inline instead of behind a pointer."
			),
			Self::MessageOveraligned {
				message_type,
				required_alignment,
				chunk_alignment,
			} => write!(
				formatter,
				"Message type '{message_type}' needs alignment {required_alignment}, but pool chunks are aligned to {chunk_alignment}. The most likely cause is a repr(align) attribute larger than a page."
			),
		}
	}
}

impl std::error::Error for MessageRouteError {}

/// The `ListenerSnapshot` struct reports how far one typed listener lags its route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListenerSnapshot {
	/// Messages published on the route that this listener has not read yet.
	pub behind: u64,
	/// Source location of the call that created the listener.
	pub created_at: &'static Location<'static>,
}

/// The `TopicSnapshot` struct reports one typed route's current state for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopicSnapshot {
	pub topic_id: usize,
	pub scope_id: u64,
	pub scope: Arc<str>,
	pub message_type: &'static str,
	pub active_listeners: usize,
	/// Messages published on this route since the bus was created.
	pub published: u64,
	/// Pool chunks this route currently holds.
	pub chunks: usize,
	/// The listener furthest behind the route's latest message, if any listener exists.
	pub slowest_listener: Option<ListenerSnapshot>,
}

/// The `PoolSnapshot` struct reports the shared chunk pool's state for diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolSnapshot {
	pub chunks: usize,
	/// Times a publisher had to wait for a chunk because the pool was exhausted.
	pub waits: u64,
}

/// The `MessageBus` struct owns one shared chunk pool and the lazy route registry over it.
#[derive(Clone)]
pub struct MessageBus {
	inner: Arc<BusInner>,
}

impl Default for MessageBus {
	fn default() -> Self {
		Self::new(MessageBusConfig::default()).expect("The default message bus configuration must be valid")
	}
}

impl MessageBus {
	/// Allocates the chunk pool and an empty route registry.
	///
	/// Next, call [`Self::new_scope`] for each owner that needs isolated typed
	/// routes. On a ticking configuration, call [`Self::begin_tick`] once per
	/// tick so listeners created during a tick replay its earlier messages.
	pub fn new(config: MessageBusConfig) -> Result<Self, MessageBusConfigError> {
		let chunks = config.validate()?;
		Ok(Self {
			inner: Arc::new(BusInner {
				core: Arc::new(BusCore {
					pool: Pool::new(chunks),
					control: Mutex::new(Control { topics: Vec::new() }),
					ticks: config.ticks,
					observer: OnceLock::new(),
				}),
				registry: Mutex::new(HashMap::default()),
				next_scope: AtomicU64::new(1),
			}),
		})
	}

	/// Attaches the one passive observer for this bus.
	///
	/// The observer sees successful publications from every future route,
	/// including application-defined generic types. Attach it before acquiring
	/// the first channel or factory. Publication observation reads existing route
	/// counters, so it never delays publishers.
	pub fn observe(&self) -> Result<MessageObserver, MessageObservationError> {
		let registry = self.inner.registry.lock();
		if self.inner.core.observer.get().is_some() {
			return Err(MessageObservationError::AlreadyAttached);
		}
		if !registry.is_empty() {
			return Err(MessageObservationError::RoutesAlreadyRegistered);
		}
		let observer = MessageObserver::new();
		self.inner
			.core
			.observer
			.set(observer.clone())
			.map_err(|_| MessageObservationError::AlreadyAttached)?;
		drop(registry);
		Ok(observer)
	}

	/// Returns the diagnostics owner already attached to this bus.
	pub(crate) fn observer(&self) -> Option<MessageObserver> {
		self.inner.core.observer.get().cloned()
	}

	/// Starts a tick: listeners created from now on replay this tick's messages.
	///
	/// Call this once per application tick before any system publishes. It also
	/// destroys the previous tick's payloads that every listener has read and
	/// returns the chunks nobody still reads to the pool.
	pub fn begin_tick(&self) {
		debug_assert!(
			self.inner.core.ticks,
			"begin_tick was called on a bus configured without ticks"
		);
		let control = self.inner.core.control.lock();
		for state in &control.topics {
			self.inner.core.end_tick(state);
		}
	}

	/// Creates an isolated namespace over the same chunk pool.
	///
	/// Routes remain lazy: creating a scope does not register a route until code
	/// requests a concrete message type from it.
	pub fn new_scope(&self, name: impl Into<Arc<str>>) -> MessageScope {
		// Keep `u64::MAX` as an exhausted sentinel so catching the panic cannot
		// wrap the counter and alias an existing namespace.
		let id = self
			.inner
			.next_scope
			.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
			.unwrap_or_else(|_| {
				panic!("Message scope identifiers exhausted. The most likely cause is an unbounded scope creation loop.")
			});
		MessageScope {
			bus: self.clone(),
			id,
			name: name.into(),
		}
	}

	/// Returns a diagnostic snapshot of every route registered so far, in route id order.
	pub fn topics(&self) -> Vec<TopicSnapshot> {
		let control = self.inner.core.control.lock();
		control.topics.iter().map(|state| self.inner.core.snapshot(state)).collect()
	}

	/// Returns the shared pool's current occupancy.
	pub fn pool(&self) -> PoolSnapshot {
		PoolSnapshot {
			chunks: self.inner.core.pool.chunks.len(),
			waits: self.inner.core.pool.waits.load(Ordering::Relaxed),
		}
	}

	/// Creates the private root namespace used by standalone typed facades.
	pub(crate) fn root_scope(&self, name: impl Into<Arc<str>>) -> MessageScope {
		MessageScope {
			bus: self.clone(),
			id: 0,
			name: name.into(),
		}
	}
}

/// The `MessageScope` struct isolates typed routes owned by one subsystem while sharing bus storage.
#[derive(Clone)]
pub struct MessageScope {
	bus: MessageBus,
	id: u64,
	name: Arc<str>,
}

impl MessageScope {
	/// Returns the shared bus that owns this namespace.
	pub(crate) fn message_bus(&self) -> &MessageBus {
		&self.bus
	}

	/// Acquires the canonical typed channel in this scope, registering it on first use.
	///
	/// Next, create listeners before publishing messages that they must observe.
	pub fn channel<M>(&self) -> DefaultChannel<M>
	where
		M: Clone + Send + Sync + 'static,
	{
		self.try_channel().unwrap_or_else(|error| panic!("{error}"))
	}

	/// Tries to acquire the canonical typed channel in this scope.
	pub fn try_channel<M>(&self) -> Result<DefaultChannel<M>, MessageRouteError>
	where
		M: Clone + Send + Sync + 'static,
	{
		self.topic().map(DefaultChannel::from_topic)
	}

	/// Acquires the canonical creation factory for `T` in this scope.
	///
	/// The factory registers `CreateMessage<T>` only when this method is first
	/// called, so application-defined types require no startup declaration.
	pub fn factory<T>(&self) -> Factory<T>
	where
		T: Clone + Send + Sync + 'static,
	{
		Factory::from_channel(self.channel())
	}

	/// Returns this scope's diagnostic name.
	pub fn name(&self) -> &str {
		&self.name
	}

	/// Looks up or registers the typed route for `M` in this scope.
	pub(crate) fn topic<M>(&self) -> Result<Arc<Topic<M>>, MessageRouteError>
	where
		M: Clone + Send + Sync + 'static,
	{
		let key = TopicKey {
			scope: self.id,
			message: TypeId::of::<M>(),
		};
		let mut registry = self.bus.inner.registry.lock();
		if let Some(route) = registry.get(&key) {
			// The key carries the message type, so the cached route is a `Topic<M>`.
			return Ok(Arc::downcast::<Topic<M>>(Arc::clone(route))
				.unwrap_or_else(|_| panic!("A cached route matches the message type it was registered under")));
		}

		let layout = SlotLayout::for_message::<M>()?;
		let core = &self.bus.inner.core;
		let mut control = core.control.lock();
		assert!(
			control.topics.len() < MAX_TOPICS,
			"Message route limit {MAX_TOPICS} reached while registering '{}'. The most likely cause is a loop that creates scopes or generic message types without bound.",
			type_name::<M>()
		);
		let state = Arc::new(TopicState {
			index: control.topics.len() as u32,
			scope_id: self.id,
			scope: Arc::clone(&self.name),
			message_type: type_name::<M>(),
			layout,
			drop_payload: std::mem::needs_drop::<M>().then_some(drop_payload::<M> as unsafe fn(*mut u8)),
			current: AtomicU64::new(u64::from(NONE)),
			oldest: AtomicU32::new(NONE),
			chunk_count: AtomicU32::new(0),
			release_flag: AtomicBool::new(false),
			membership: AtomicU64::new(0),
			// A route created inside a tick replays everything it publishes during that tick.
			tick_start: AtomicU64::new(if core.ticks { 0 } else { TICKLESS }),
			sweep: Mutex::new(SlotCursor {
				chunk: NONE,
				slot: 0,
				sequence: 0,
			}),
			listeners: Mutex::new(Vec::new()),
		});
		control.topics.push(Arc::clone(&state));
		drop(control);

		let topic = Arc::new(Topic::<M> {
			core: Arc::clone(core),
			state,
			observed_type: OnceLock::new(),
			watcher: AtomicPtr::new(std::ptr::null_mut()),
			_marker: PhantomData,
		});
		registry.insert(key, topic.clone());
		Ok(topic)
	}
}

impl fmt::Debug for MessageScope {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("MessageScope")
			.field("id", &self.id)
			.field("name", &self.name)
			.finish()
	}
}

/// The `Watcher` type is the boxed diagnostics hook one route calls for every publication.
type Watcher<M> = Box<dyn Fn(&M) + Send + Sync>;

/// Passes one publication to a route watcher loaded with relaxed ordering.
///
/// Kept out of line so the unwatched send path stays as small as it was without watchers.
#[cold]
#[inline(never)]
fn notify_watcher<M>(watcher: *mut Watcher<M>, message: &M) {
	// Pairs with the release in `Topic::watch`, so the boxed watcher is fully visible.
	fence(Ordering::Acquire);
	// SAFETY: A published watcher stays valid until its topic drops, and the topic outlives this send.
	unsafe { (*watcher)(message) };
}

impl<M> Drop for Topic<M> {
	fn drop(&mut self) {
		let watcher = *self.watcher.get_mut();
		if !watcher.is_null() {
			// SAFETY: The watcher came from `Box::into_raw` in `Topic::watch` and is freed exactly once here.
			drop(unsafe { Box::from_raw(watcher) });
		}
	}
}

/// Destroys the retained payload of one slot through its erased route type.
unsafe fn drop_payload<M>(payload: *mut u8) {
	// SAFETY: Callers pass the payload address of a committed slot whose route
	// was registered for `M`, and they hold exclusive ownership of that payload.
	unsafe { payload.cast::<M>().drop_in_place() }
}

/// Packs a tag into the high half of a word and a value into the low half.
///
/// The free list, chunk reservations, route cursors, and listener membership
/// all use this shape so one compare-and-swap covers both halves.
fn tagged(tag: u32, value: u32) -> u64 {
	(u64::from(tag) << 32) | u64::from(value)
}

fn tag(word: u64) -> u32 {
	(word >> 32) as u32
}

fn value(word: u64) -> u32 {
	word as u32
}

/// The `BusInner` struct keeps the route registry beside the core that routes share.
///
/// Cached routes hold the core, never this struct, so dropping the last bus
/// handle releases the registry and with it every route.
struct BusInner {
	core: Arc<BusCore>,
	/// Cached typed routes for each scoped message type.
	registry: Mutex<HashMap<TopicKey, Arc<dyn Any + Send + Sync>>>,
	next_scope: AtomicU64,
}

/// The `BusCore` struct keeps the pool, the route list, and tick state behind one shared handle.
struct BusCore {
	pool: Pool,
	control: Mutex<Control>,
	/// Whether the owner calls `begin_tick`, which decides where new routes start their tick.
	ticks: bool,
	observer: OnceLock<MessageObserver>,
}

/// The `Control` struct holds the route list that the bus's control lock protects.
struct Control {
	topics: Vec<Arc<TopicState>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// The `TopicKey` struct identifies one message type inside one isolated scope.
struct TopicKey {
	scope: u64,
	message: TypeId,
}

/// The `Pool` struct owns the chunk memory, the per-chunk metadata, and the lock-free free list.
struct Pool {
	allocation: NonNull<u8>,
	layout: Layout,
	/// First chunk, aligned to `CHUNK_ALIGNMENT` inside the allocation.
	base: NonNull<u8>,
	chunks: Box<[ChunkMeta]>,
	/// Head of the free list: the free chunk index in the low bits and an ABA tag in the high bits.
	free_head: AtomicU64,
	waits: AtomicU64,
}

impl Pool {
	/// Reserves zeroed chunk memory and threads every chunk onto the free list.
	fn new(chunk_count: usize) -> Self {
		// Requesting the allocator's small alignment keeps the zeroed pages lazy;
		// a stricter request would make the allocator write every page at startup.
		let layout = Layout::from_size_align(chunk_count * CHUNK_BYTES + CHUNK_ALIGNMENT, 16)
			.expect("A validated chunk count must produce an addressable pool");
		// SAFETY: The layout is nonzero and has a power-of-two alignment.
		let pointer = unsafe { alloc_zeroed(layout) };
		let allocation = NonNull::new(pointer).unwrap_or_else(|| handle_alloc_error(layout));
		// SAFETY: The extra `CHUNK_ALIGNMENT` bytes keep the aligned base inside the allocation.
		let base = unsafe { allocation.add(allocation.as_ptr().align_offset(CHUNK_ALIGNMENT)) };
		let chunks = (0..chunk_count)
			.map(|_| ChunkMeta::default())
			.collect::<Vec<_>>()
			.into_boxed_slice();
		let pool = Self {
			allocation,
			layout,
			base,
			chunks,
			free_head: AtomicU64::new(u64::from(NONE)),
			waits: AtomicU64::new(0),
		};
		for index in (0..chunk_count as u32).rev() {
			pool.push_free(index);
		}
		pool
	}

	/// Returns a chunk to the free list. Safe to call concurrently with other pushes and one pop.
	fn push_free(&self, index: u32) {
		let meta = &self.chunks[index as usize];
		let mut head = self.free_head.load(Ordering::Acquire);
		loop {
			meta.next_free.store(value(head), Ordering::Relaxed);
			let next = tagged(tag(head) + 1, index);
			match self
				.free_head
				.compare_exchange_weak(head, next, Ordering::AcqRel, Ordering::Acquire)
			{
				Ok(_) => return,
				Err(actual) => head = actual,
			}
		}
	}

	/// Takes a chunk from the free list. Callers hold the control lock, so pops never race each other.
	fn pop_free(&self) -> Option<u32> {
		let mut head = self.free_head.load(Ordering::Acquire);
		loop {
			let index = value(head);
			if index == NONE {
				return None;
			}
			let next = tagged(tag(head) + 1, self.chunks[index as usize].next_free.load(Ordering::Relaxed));
			match self
				.free_head
				.compare_exchange_weak(head, next, Ordering::AcqRel, Ordering::Acquire)
			{
				Ok(_) => return Some(index),
				Err(actual) => head = actual,
			}
		}
	}

	fn has_free_chunk(&self) -> bool {
		value(self.free_head.load(Ordering::Acquire)) != NONE
	}

	/// Returns the address of one slot inside a chunk.
	fn slot(&self, chunk: u32, slot: u32, layout: SlotLayout) -> *mut u8 {
		let offset = chunk as usize * CHUNK_BYTES + slot as usize * layout.stride as usize;
		debug_assert!(offset + layout.stride as usize <= self.chunks.len() * CHUNK_BYTES);
		// SAFETY: Layout validation keeps every slot of every chunk inside the pool allocation.
		unsafe { self.base.as_ptr().add(offset) }
	}

	/// Returns the commit stamp in front of one slot's payload.
	fn stamp(&self, chunk: u32, slot: u32, layout: SlotLayout) -> &AtomicU64 {
		// SAFETY: Slots are 8-byte aligned inside zeroed pool memory, and an atomic
		// tolerates concurrent access from every thread.
		unsafe { &*self.slot(chunk, slot, layout).cast::<AtomicU64>() }
	}

	/// Returns the reader accounting of one slot. Only routes with destructors have it.
	fn accounting(&self, chunk: u32, slot: u32, layout: SlotLayout) -> &SlotAccounting {
		debug_assert!(layout.accounting, "Only payloads with destructors track owed reads");
		// SAFETY: The layout reserved these bytes after the stamp, and the struct
		// holds only atomics.
		unsafe { &*self.slot(chunk, slot, layout).add(STAMP_BYTES).cast::<SlotAccounting>() }
	}

	/// Returns the payload address of one slot for the route's message type.
	fn payload<M>(&self, chunk: u32, slot: u32, layout: SlotLayout) -> *mut M {
		self.payload_bytes(chunk, slot, layout).cast::<M>()
	}

	/// Returns the payload address of one slot without its type.
	fn payload_bytes(&self, chunk: u32, slot: u32, layout: SlotLayout) -> *mut u8 {
		// SAFETY: The payload offset was rounded up to the message alignment and the
		// stride keeps the payload inside its slot.
		unsafe { self.slot(chunk, slot, layout).add(layout.payload_offset as usize) }
	}
}

impl Drop for Pool {
	fn drop(&mut self) {
		// SAFETY: `allocation` came from `alloc_zeroed` with this exact layout, and the
		// core destroys every retained payload before it drops the pool.
		unsafe { dealloc(self.allocation.as_ptr(), self.layout) };
	}
}

// SAFETY: Chunk memory is partitioned into slots whose publication and reuse are
// synchronized through the atomic headers and chunk metadata described in the
// module documentation.
unsafe impl Send for Pool {}
// SAFETY: See the `Send` implementation. Shared payload references exist only for
// message types that implement `Sync`.
unsafe impl Sync for Pool {}

#[repr(align(64))]
/// The `ChunkMeta` struct keeps one chunk's chain links and reader accounting on its own cache line.
struct ChunkMeta {
	/// Reserved slot count in the low bits and the chunk's generation in the high bits. The generation changes
	/// on every reuse so a publisher holding a stale route cursor cannot reserve into the chunk's next owner.
	reservation: AtomicU64,
	/// Next chunk of the same route, or `NONE` while this chunk still accepts reservations.
	next: AtomicU32,
	/// Previous chunk of the same route. Followed only under the control lock to locate a tick's first record.
	prev: AtomicU32,
	/// Listeners that have not left this chunk yet. The last one to leave recycles it.
	pending: AtomicU32,
	/// Free-list link.
	next_free: AtomicU32,
	/// Sequence of the first slot.
	first_sequence: AtomicU64,
}

impl Default for ChunkMeta {
	fn default() -> Self {
		Self {
			reservation: AtomicU64::new(0),
			next: AtomicU32::new(NONE),
			prev: AtomicU32::new(NONE),
			pending: AtomicU32::new(0),
			next_free: AtomicU32::new(NONE),
			first_sequence: AtomicU64::new(0),
		}
	}
}

impl ChunkMeta {
	fn generation(&self) -> u32 {
		tag(self.reservation.load(Ordering::Relaxed))
	}

	/// Returns how many slots are reserved, capped at the route's slot count.
	fn reserved(&self, layout: SlotLayout, ordering: Ordering) -> u32 {
		value(self.reservation.load(ordering)).min(layout.slots_per_chunk)
	}

	/// Returns the sequence after this chunk's last slot.
	fn end_sequence(&self, layout: SlotLayout) -> u64 {
		self.first_sequence.load(Ordering::Relaxed) + u64::from(layout.slots_per_chunk)
	}
}

#[repr(C)]
/// The `SlotAccounting` struct follows the stamp of payloads with destructors.
struct SlotAccounting {
	/// Listeners that still owe a read of this payload, or `GONE` once the payload is destroyed.
	remaining: AtomicU32,
	/// Route listener epoch at publication, used to decide whether a listener joining or leaving
	/// was counted in `remaining`.
	epoch: AtomicU32,
}

const _: () = assert!(std::mem::size_of::<SlotAccounting>() == ACCOUNTING_BYTES);

/// Destroys a payload nobody still owes a read to. Returns whether this call destroyed it.
///
/// The exchange to `GONE` claims exclusive destruction, so concurrent callers
/// never destroy a payload twice.
unsafe fn destroy_consumed(accounting: &SlotAccounting, payload: *mut u8, drop: unsafe fn(*mut u8)) -> bool {
	let claimed = accounting
		.remaining
		.compare_exchange(0, GONE, Ordering::AcqRel, Ordering::Relaxed)
		.is_ok();
	if claimed {
		// SAFETY: A zero count proves every counted listener finished reading, the
		// exchange claimed destruction, and the caller vouches for the slot's route.
		unsafe { drop(payload) };
	}
	claimed
}

/// Gives up one owed read, destroying the payload when this was the last read of a record no
/// listener can replay anymore.
fn release_read(accounting: &SlotAccounting, payload: *mut u8, drop: unsafe fn(*mut u8), movable: bool) {
	let previous = accounting.remaining.fetch_sub(1, Ordering::AcqRel);
	assert_ne!(previous, 0, "A listener released a read of a payload it was not counted for");
	if previous == 1 && movable {
		// SAFETY: The caller releases a read it was counted for on a committed slot of this route.
		unsafe { destroy_consumed(accounting, payload, drop) };
	}
}

#[derive(Clone, Copy)]
/// The `SlotLayout` struct maps one message type onto fixed slots inside a chunk.
struct SlotLayout {
	stride: u32,
	payload_offset: u32,
	slots_per_chunk: u32,
	/// Whether slots carry reader accounting, which only payloads with destructors need.
	accounting: bool,
}

impl SlotLayout {
	/// Computes slot placement for a message type or rejects one that cannot share a chunk.
	fn for_message<M>() -> Result<Self, MessageRouteError> {
		let required_alignment = std::mem::align_of::<M>();
		if required_alignment > CHUNK_ALIGNMENT {
			return Err(MessageRouteError::MessageOveraligned {
				message_type: type_name::<M>(),
				required_alignment,
				chunk_alignment: CHUNK_ALIGNMENT,
			});
		}
		let accounting = std::mem::needs_drop::<M>();
		let header = STAMP_BYTES + if accounting { ACCOUNTING_BYTES } else { 0 };
		let payload_offset = header.next_multiple_of(required_alignment);
		let stride = (payload_offset + std::mem::size_of::<M>()).next_multiple_of(required_alignment.max(8));
		if stride > CHUNK_BYTES {
			return Err(MessageRouteError::MessageTooLarge {
				message_type: type_name::<M>(),
				message_bytes: std::mem::size_of::<M>(),
				chunk_bytes: CHUNK_BYTES,
			});
		}
		Ok(Self {
			stride: stride as u32,
			payload_offset: payload_offset as u32,
			slots_per_chunk: (CHUNK_BYTES / stride) as u32,
			accounting,
		})
	}
}

/// The `TopicState` struct keeps one route's chunk chain, listener membership, and tick state.
struct TopicState {
	index: u32,
	scope_id: u64,
	scope: Arc<str>,
	message_type: &'static str,
	layout: SlotLayout,
	/// Destroys one retained payload. `None` for message types without destructors.
	drop_payload: Option<unsafe fn(*mut u8)>,
	/// Chunk accepting reservations in the low bits and its generation in the high bits, or `NONE`
	/// before the first chunk is attached.
	current: AtomicU64,
	/// Head of the chunk chain. Advanced only while `release_flag` is held.
	oldest: AtomicU32,
	/// Chunks currently in the chain, kept for diagnostics.
	chunk_count: AtomicU32,
	/// Try-lock held while recycling head chunks so releases stay in chain order.
	release_flag: AtomicBool,
	/// Active listener count in the low bits and the membership epoch in the high bits. The epoch
	/// changes whenever a listener joins or leaves.
	membership: AtomicU64,
	/// First sequence of the current tick, or `TICKLESS` on a bus without ticks. Records below it
	/// are from finished ticks: they can be moved out by their last reader, and chunks entirely
	/// below it can return to the pool.
	tick_start: AtomicU64,
	/// Position from which the next tick sweep resumes destroying consumed payloads. Touched only
	/// while `release_flag` is held.
	sweep: Mutex<SlotCursor>,
	/// Registered listeners, kept for lag diagnostics.
	listeners: Mutex<Vec<Arc<ListenerShared>>>,
}

impl TopicState {
	fn active_listeners(&self) -> u32 {
		value(self.membership.load(Ordering::Relaxed))
	}

	/// Returns the current chunk and the sequence after its last reservation.
	///
	/// Membership changes read the reservation with `SeqCst` so they order
	/// against publishers that reserve first and read membership second.
	fn frontier(&self, pool: &Pool, ordering: Ordering) -> (u32, u64) {
		let current = value(self.current.load(Ordering::Acquire));
		if current == NONE {
			return (NONE, 0);
		}
		let meta = &pool.chunks[current as usize];
		(
			current,
			meta.first_sequence.load(Ordering::Relaxed) + u64::from(meta.reserved(self.layout, ordering)),
		)
	}

	/// Returns the next sequence this route will assign.
	fn published(&self, pool: &Pool) -> u64 {
		self.frontier(pool, Ordering::Acquire).1
	}

	fn stamp(&self, sequence: u64) -> u64 {
		((u64::from(self.index) + 1) << STAMP_SEQUENCE_BITS) | ((sequence + 1) & STAMP_SEQUENCE_MASK)
	}
}

#[repr(align(64))]
/// The `ListenerShared` struct exposes one listener's progress and origin to diagnostics.
///
/// It takes a cache line of its own so listeners drained on different threads
/// do not invalidate each other's progress stores.
struct ListenerShared {
	next_sequence: AtomicU64,
	created_at: &'static Location<'static>,
}

impl BusCore {
	/// Locks the control plane with the route's first chunk attached, waiting for a free chunk if needed.
	fn lock_with_first_chunk(&self, state: &TopicState) -> MutexGuard<'_, Control> {
		loop {
			let control = self.control.lock();
			if value(state.current.load(Ordering::Acquire)) != NONE || self.attach_first_chunk(state) {
				return control;
			}
			drop(control);
			self.wait_for_chunk(state);
		}
	}

	/// Attaches a route's first chunk while holding the control lock. Returns `false` when the pool is empty.
	fn attach_first_chunk(&self, state: &TopicState) -> bool {
		let Some(chunk) = self.pool.pop_free() else {
			return false;
		};
		let generation = self.init_chunk(state, chunk, NONE, 0);
		*state.sweep.lock() = SlotCursor {
			chunk,
			slot: 0,
			sequence: 0,
		};
		state.oldest.store(chunk, Ordering::Release);
		state.current.store(tagged(generation, chunk), Ordering::Release);
		true
	}

	/// Prepares a popped chunk for a route and returns its new generation. Callers hold the control lock.
	fn init_chunk(&self, state: &TopicState, chunk: u32, prev: u32, first_sequence: u64) -> u32 {
		let meta = &self.pool.chunks[chunk as usize];
		let generation = meta.generation().wrapping_add(1);
		meta.reservation.store(tagged(generation, 0), Ordering::Relaxed);
		meta.next.store(NONE, Ordering::Relaxed);
		meta.prev.store(prev, Ordering::Relaxed);
		meta.first_sequence.store(first_sequence, Ordering::Relaxed);
		// Every active listener will pass through this chunk. Listeners that join
		// later and replay into it add themselves under the control lock.
		meta.pending.store(state.active_listeners(), Ordering::Relaxed);
		state.chunk_count.fetch_add(1, Ordering::Relaxed);
		generation
	}

	/// Seals a full chunk by attaching its successor, waiting for a free chunk if the pool is empty.
	fn attach_next_chunk(&self, state: &TopicState, full: u32, generation: u32) {
		loop {
			let control = self.control.lock();
			let meta = &self.pool.chunks[full as usize];
			// Another publisher may have sealed this chunk, or it may have been
			// recycled under a cursor this publisher loaded before falling behind.
			if meta.generation() != generation || meta.next.load(Ordering::Acquire) != NONE {
				return;
			}
			if let Some(chunk) = self.pool.pop_free() {
				let next_generation = self.init_chunk(state, chunk, full, meta.end_sequence(state.layout));
				meta.next.store(chunk, Ordering::Release);
				state.current.store(tagged(next_generation, chunk), Ordering::Release);
				return;
			}
			drop(control);
			self.wait_for_chunk(state);
		}
	}

	/// Waits without the control lock until the pool has a free chunk, reporting long waits.
	fn wait_for_chunk(&self, state: &TopicState) {
		self.pool.waits.fetch_add(1, Ordering::Relaxed);
		let started = Instant::now();
		let mut next_report = started + EXHAUSTED_POOL_FIRST_REPORT;
		let mut spins = 0u32;
		while !self.pool.has_free_chunk() {
			if Instant::now() >= next_report {
				self.report_exhausted_pool(state, started.elapsed());
				next_report += EXHAUSTED_POOL_REPORT_INTERVAL;
			}
			backoff(&mut spins);
		}
	}

	/// Logs which routes hold the pool while a publisher is stalled.
	#[cold]
	#[inline(never)]
	fn report_exhausted_pool(&self, waiting: &TopicState, elapsed: Duration) {
		let control = self.control.lock();
		let mut usage = control
			.topics
			.iter()
			.map(|state| (state.chunk_count.load(Ordering::Relaxed), state))
			.filter(|(chunks, _)| *chunks > 0)
			.collect::<Vec<_>>();
		usage.sort_unstable_by_key(|(chunks, _)| std::cmp::Reverse(*chunks));
		let mut report = format!(
			"Message pool exhausted: a publisher of '{}' has waited {:.1} s for a free chunk. The most likely cause is a listener that no longer reads its messages; a system that stops consuming must drop its listener, or raise messages.capacity. Chunks held:",
			waiting.message_type,
			elapsed.as_secs_f64()
		);
		for (chunks, state) in usage.iter().take(3) {
			let _ = write!(report, " '{}' holds {chunks}", state.message_type);
			if let Some(slowest) = self.slowest_listener(state, state.published(&self.pool)) {
				let _ = write!(
					report,
					" (slowest listener created at {} is {} messages behind)",
					slowest.created_at, slowest.behind
				);
			}
			report.push(';');
		}
		drop(control);
		log::error!("{report}");
	}

	/// Finds the listener furthest behind the route's latest message. Callers hold the control lock.
	fn slowest_listener(&self, state: &TopicState, published: u64) -> Option<ListenerSnapshot> {
		state
			.listeners
			.lock()
			.iter()
			.map(|listener| ListenerSnapshot {
				behind: published.saturating_sub(listener.next_sequence.load(Ordering::Relaxed)),
				created_at: listener.created_at,
			})
			.max_by_key(|listener| listener.behind)
	}

	/// Builds one route's diagnostic snapshot. Callers hold the control lock.
	fn snapshot(&self, state: &TopicState) -> TopicSnapshot {
		let published = state.published(&self.pool);
		TopicSnapshot {
			topic_id: state.index as usize,
			scope_id: state.scope_id,
			scope: Arc::clone(&state.scope),
			message_type: state.message_type,
			active_listeners: state.active_listeners() as usize,
			published,
			chunks: state.chunk_count.load(Ordering::Relaxed) as usize,
			slowest_listener: self.slowest_listener(state, published),
		}
	}

	/// Finishes the current tick for one route. Callers hold the control lock.
	///
	/// Consumed payloads of the finished tick are destroyed first, while its
	/// chunks are still pinned. Publishing the new tick start then lets the last
	/// reader of each older record move it out and lets chunks below the tick
	/// return to the pool.
	fn end_tick(&self, state: &TopicState) {
		let next = state.published(&self.pool);
		let flag = ReleaseFlag::acquire(state);
		let mut sweep = state.sweep.lock();
		// An idle route has nothing to destroy and nothing new to make releasable.
		if next == state.tick_start.load(Ordering::Relaxed) && sweep.sequence == next {
			return;
		}
		self.sweep_consumed_payloads(state, &mut sweep, next);
		state.tick_start.store(next, Ordering::Release);
		drop(sweep);
		drop(flag);
		self.release_chunks(state);
	}

	/// Destroys payloads below `end` that every listener has read. Callers hold the release flag.
	fn sweep_consumed_payloads(&self, state: &TopicState, cursor: &mut SlotCursor, end: u64) {
		let Some(drop) = state.drop_payload else {
			return;
		};
		let layout = state.layout;
		while cursor.chunk != NONE && cursor.sequence < end {
			cursor.enter_next_chunk(&self.pool, layout);
			debug_assert!(
				cursor.slot < layout.slots_per_chunk,
				"A sequence below the frontier has a slot"
			);
			// An uncommitted slot belongs to a publisher still writing; resume here next tick.
			if !cursor.is_committed(&self.pool, state) {
				break;
			}
			// SAFETY: The slot is committed on this route and nobody owes it a read once the count is zero.
			unsafe {
				destroy_consumed(
					self.pool.accounting(cursor.chunk, cursor.slot, layout),
					self.pool.payload_bytes(cursor.chunk, cursor.slot, layout),
					drop,
				)
			};
			cursor.step();
		}
	}

	/// Recycles head chunks nobody reads anymore, retrying if another thread is already doing so.
	fn release_chunks(&self, state: &TopicState) {
		loop {
			let Some(flag) = ReleaseFlag::try_acquire(state) else {
				// The holder rechecks the head after it clears the flag.
				return;
			};
			self.release_head_chunks(state);
			drop(flag);
			if !self.head_releasable(state) {
				return;
			}
		}
	}

	/// Returns whether the oldest chunk is sealed, read by nobody, and older than the current tick.
	fn head_releasable(&self, state: &TopicState) -> bool {
		let head = state.oldest.load(Ordering::Acquire);
		if head == NONE {
			return false;
		}
		let meta = &self.pool.chunks[head as usize];
		meta.next.load(Ordering::Acquire) != NONE
			&& meta.pending.load(Ordering::SeqCst) == 0
			&& meta.end_sequence(state.layout) <= state.tick_start.load(Ordering::Acquire)
	}

	/// Returns releasable head chunks to the pool in chain order. Callers hold the release flag.
	fn release_head_chunks(&self, state: &TopicState) {
		while self.head_releasable(state) {
			let head = state.oldest.load(Ordering::Acquire);
			if !self.retire_slots(state, head) {
				return;
			}
			let meta = &self.pool.chunks[head as usize];
			let next = meta.next.load(Ordering::Acquire);
			// The sweep may have stopped in this chunk at a slot that was still uncommitted.
			let mut sweep = state.sweep.lock();
			if sweep.chunk == head {
				*sweep = SlotCursor {
					chunk: next,
					slot: 0,
					sequence: meta.end_sequence(state.layout),
				};
			}
			drop(sweep);
			state.oldest.store(next, Ordering::Release);
			state.chunk_count.fetch_sub(1, Ordering::Relaxed);
			self.pool.push_free(head);
		}
	}

	/// Destroys the retained payloads of a chunk nobody reads. Returns `false` if a slot is still uncommitted.
	fn retire_slots(&self, state: &TopicState, chunk: u32) -> bool {
		let layout = state.layout;
		let first_sequence = self.pool.chunks[chunk as usize].first_sequence.load(Ordering::Relaxed);
		for slot in 0..layout.slots_per_chunk {
			// A route without listeners may still have a publisher writing here.
			if self.pool.stamp(chunk, slot, layout).load(Ordering::Acquire) != state.stamp(first_sequence + u64::from(slot)) {
				return false;
			}
			let Some(drop) = state.drop_payload else {
				continue;
			};
			let accounting = self.pool.accounting(chunk, slot, layout);
			// SAFETY: No listener is pending on this chunk, so every read of this committed slot finished.
			if !unsafe { destroy_consumed(accounting, self.pool.payload_bytes(chunk, slot, layout), drop) } {
				debug_assert_eq!(
					accounting.remaining.load(Ordering::Relaxed),
					GONE,
					"A recycled chunk cannot retain a payload a listener still owes a read to"
				);
			}
		}
		true
	}
}

impl Drop for BusCore {
	fn drop(&mut self) {
		let control = self.control.get_mut();
		for state in &control.topics {
			let Some(drop) = state.drop_payload else {
				continue;
			};
			let layout = state.layout;
			let mut chunk = state.oldest.load(Ordering::Relaxed);
			while chunk != NONE {
				let meta = &self.pool.chunks[chunk as usize];
				let first_sequence = meta.first_sequence.load(Ordering::Relaxed);
				for slot in 0..meta.reserved(layout, Ordering::Relaxed) {
					if self.pool.stamp(chunk, slot, layout).load(Ordering::Relaxed)
						== state.stamp(first_sequence + u64::from(slot))
					{
						// SAFETY: The last core owner is being destroyed, so no listener can still owe a read.
						unsafe {
							destroy_consumed(
								self.pool.accounting(chunk, slot, layout),
								self.pool.payload_bytes(chunk, slot, layout),
								drop,
							)
						};
					}
				}
				chunk = meta.next.load(Ordering::Relaxed);
			}
		}
	}
}

/// Pauses a thread that is waiting on another thread's progress, escalating from spinning to sleeping.
fn backoff(spins: &mut u32) {
	if *spins < 32 {
		std::hint::spin_loop();
	} else if *spins < 64 {
		std::thread::yield_now();
	} else {
		std::thread::sleep(Duration::from_micros(50));
	}
	*spins = spins.saturating_add(1);
}

/// The `ReleaseFlag` struct holds a route's recycling right and returns it even if recycling unwinds.
struct ReleaseFlag<'topic> {
	state: &'topic TopicState,
}

impl<'topic> ReleaseFlag<'topic> {
	fn try_acquire(state: &'topic TopicState) -> Option<Self> {
		(!state.release_flag.swap(true, Ordering::SeqCst)).then_some(Self { state })
	}

	/// Waits for the holder, which only recycles chunks and never blocks, to finish.
	fn acquire(state: &'topic TopicState) -> Self {
		let mut spins = 0;
		loop {
			if let Some(flag) = Self::try_acquire(state) {
				return flag;
			}
			backoff(&mut spins);
		}
	}
}

impl Drop for ReleaseFlag<'_> {
	fn drop(&mut self) {
		self.state.release_flag.store(false, Ordering::SeqCst);
	}
}

/// The `Topic` struct provides one cached typed route over the shared pool.
pub(crate) struct Topic<M> {
	core: Arc<BusCore>,
	state: Arc<TopicState>,
	/// The observer's catalog identity of the factory value type carried by `M`, resolved once.
	observed_type: OnceLock<ObservedType>,
	/// The optional diagnostics hook that sees every publication before listeners can read it.
	///
	/// Sends load it relaxed, and only watched sends pay for the acquire fence.
	watcher: AtomicPtr<Watcher<M>>,
	_marker: PhantomData<fn() -> M>,
}

impl<M> Topic<M>
where
	M: Clone + Send + Sync + 'static,
{
	/// Publishes one message, waiting for a free chunk if the pool is exhausted.
	pub(crate) fn send(&self, message: M) {
		let layout = self.state.layout;
		let pool = &self.core.pool;
		let (chunk, slot) = self.reserve();
		let sequence = pool.chunks[chunk as usize].first_sequence.load(Ordering::Relaxed) + u64::from(slot);
		if std::mem::needs_drop::<M>() {
			// Read membership only after the reservation is visible. A listener that
			// joins or leaves reads the reserved count after changing membership, so
			// it either sees this slot and corrects its count or this load sees it.
			let membership = self.state.membership.load(Ordering::SeqCst);
			let accounting = pool.accounting(chunk, slot, layout);
			accounting.remaining.store(value(membership), Ordering::Relaxed);
			accounting.epoch.store(tag(membership), Ordering::Relaxed);
		}
		let payload = pool.payload::<M>(chunk, slot, layout);
		// SAFETY: The reservation gives this publisher exclusive ownership of the
		// slot, whose previous payload was destroyed before its chunk was recycled.
		unsafe { payload.write(message) };
		// The watcher borrows the payload in place, before the stamp lets listeners take it.
		// Borrowing it here keeps the message out of memory on the unwatched path.
		let watcher = self.watcher.load(Ordering::Relaxed);
		if !watcher.is_null() {
			// SAFETY: The payload was just written and stays owned by this publisher until it is stamped.
			notify_watcher(watcher, unsafe { &*payload });
		}
		pool.stamp(chunk, slot, layout)
			.store(self.state.stamp(sequence), Ordering::Release);
	}

	/// Reserves one slot in the route's current chunk, attaching chunks as they fill.
	fn reserve(&self) -> (u32, u32) {
		let layout = self.state.layout;
		loop {
			let current = self.state.current.load(Ordering::Acquire);
			let chunk = value(current);
			if chunk == NONE {
				drop(self.core.lock_with_first_chunk(&self.state));
				continue;
			}
			let generation = tag(current);
			let meta = &self.core.pool.chunks[chunk as usize];
			let mut reservation = meta.reservation.load(Ordering::Relaxed);
			let mut pause = 1u32;
			loop {
				// The chunk was recycled after this publisher loaded the route cursor.
				if tag(reservation) != generation {
					break;
				}
				let reserved = value(reservation);
				if reserved >= layout.slots_per_chunk {
					self.core.attach_next_chunk(&self.state, chunk, generation);
					break;
				}
				match meta
					.reservation
					.compare_exchange_weak(reservation, reservation + 1, Ordering::SeqCst, Ordering::Relaxed)
				{
					Ok(_) => return (chunk, reserved),
					Err(actual) => {
						reservation = actual;
						// Losing publishers pause for growing intervals so the winner
						// keeps publishing into cache lines it already owns instead of
						// every publisher fighting over the same slots.
						for _ in 0..pause {
							std::hint::spin_loop();
						}
						pause = (pause << 1).min(RESERVATION_BACKOFF_LIMIT);
					}
				}
			}
		}
	}

	/// Registers a listener that starts at the current tick's first message.
	///
	/// On a bus without ticks the listener is future-only.
	pub(crate) fn subscribe(self: &Arc<Self>, created_at: &'static Location<'static>) -> ListenerToken<M> {
		let layout = self.state.layout;
		let pool = &self.core.pool;
		let state = &self.state;
		let shared = Arc::new(ListenerShared {
			next_sequence: AtomicU64::new(0),
			created_at,
		});
		let control = self.core.lock_with_first_chunk(state);

		// Joining changes the epoch first so publishers reserving from now on count this listener.
		let epoch = tag(join(&state.membership)) + 1;
		let (current, end_sequence) = state.frontier(pool, Ordering::SeqCst);
		let start_sequence = state.tick_start.load(Ordering::Acquire).min(end_sequence);

		// Walk back to the chunk holding the tick's first record, counting this
		// listener into every chunk it will pass through. Chunks that hold this
		// tick's records cannot be recycled, so every link on the way is live.
		let mut chunk = current;
		loop {
			let meta = &pool.chunks[chunk as usize];
			meta.pending.fetch_add(1, Ordering::SeqCst);
			if meta.first_sequence.load(Ordering::Relaxed) <= start_sequence {
				break;
			}
			chunk = meta.prev.load(Ordering::Relaxed);
		}
		let cursor = SlotCursor {
			chunk,
			slot: (start_sequence - pool.chunks[chunk as usize].first_sequence.load(Ordering::Relaxed)) as u32,
			sequence: start_sequence,
		};

		// Replayed payloads with destructors must count one more owed read, unless
		// their publisher already saw this listener in the membership.
		if std::mem::needs_drop::<M>() {
			let mut replay = cursor;
			while replay.sequence < end_sequence {
				replay.enter_next_chunk(pool, layout);
				replay.wait_committed(pool, state);
				let accounting = pool.accounting(replay.chunk, replay.slot, layout);
				if published_before(accounting.epoch.load(Ordering::Relaxed), epoch) {
					accounting.remaining.fetch_add(1, Ordering::Relaxed);
				}
				replay.step();
			}
		}

		shared.next_sequence.store(start_sequence, Ordering::Relaxed);
		state.listeners.lock().push(Arc::clone(&shared));
		drop(control);

		ListenerToken {
			topic: Arc::clone(self),
			cursor,
			shared,
		}
	}

	/// Resolves the observer and catalog identity for factory values of type `T` carried by `M`.
	pub(crate) fn observation<T: 'static>(&self) -> Option<(MessageObserver, ObservedType)> {
		let observer = self.core.observer.get()?;
		let observed = *self.observed_type.get_or_init(|| observer.observed_type::<T>());
		Some((observer.clone(), observed))
	}

	/// Installs the route's one watcher, which sees every later publication from any sender.
	pub(crate) fn watch(&self, watcher: impl Fn(&M) + Send + Sync + 'static) {
		let fresh = Box::into_raw(Box::new(Box::new(watcher) as Watcher<M>));
		let installed = self
			.watcher
			.compare_exchange(std::ptr::null_mut(), fresh, Ordering::Release, Ordering::Relaxed)
			.is_ok();
		if !installed {
			// SAFETY: `fresh` was never published, so this is its only owner.
			drop(unsafe { Box::from_raw(fresh) });
		}
		assert!(
			installed,
			"Message route '{}' already has a watcher. The most likely cause is that more than one inspector watches the same scope.",
			self.state.message_type
		);
	}

	/// Removes one terminally deleted handle from the optional entity catalog.
	#[inline(always)]
	pub(crate) fn forget_entity(&self, handle: crate::core::factory::Handle) {
		if let Some(observer) = self.core.observer.get() {
			observer.forget_entity(handle);
		}
	}

	/// Reads the next committed message at the cursor, moving into the next chunk when this one is read.
	fn read(&self, cursor: &mut SlotCursor, shared: &ListenerShared) -> Option<M> {
		let layout = self.state.layout;
		let pool = &self.core.pool;
		if let Some(left) = cursor.enter_next_chunk(pool, layout) {
			self.leave_chunk(left);
		}
		if cursor.slot == layout.slots_per_chunk || !cursor.is_committed(pool, &self.state) {
			return None;
		}
		Some(self.take(cursor, shared))
	}

	/// Clones or moves out the committed payload at the cursor and advances past it.
	fn take(&self, cursor: &mut SlotCursor, shared: &ListenerShared) -> M {
		let layout = self.state.layout;
		let pool = &self.core.pool;
		let (chunk, slot, sequence) = (cursor.chunk, cursor.slot, cursor.sequence);
		let payload = pool.payload::<M>(chunk, slot, layout);
		// Advance before running user code so a panicking clone still releases the slot.
		let _advance = CursorAdvance { cursor, shared };
		if !std::mem::needs_drop::<M>() {
			// SAFETY: The matching acquire stamp proves initialization, and the chunk
			// cannot be recycled while this listener is pending on it.
			return unsafe { (*payload).clone() };
		}
		let accounting = pool.accounting(chunk, slot, layout);
		// A record from a finished tick can be moved out by its last reader because
		// no later listener can replay it.
		let movable = sequence < self.state.tick_start.load(Ordering::Acquire);
		if movable
			&& accounting.remaining.load(Ordering::Acquire) == 1
			&& accounting
				.remaining
				.compare_exchange(1, GONE, Ordering::AcqRel, Ordering::Relaxed)
				.is_ok()
		{
			// SAFETY: This exchange claimed the only owed read, so no clone can
			// still borrow the payload and nobody else will destroy it.
			return unsafe { payload.read() };
		}
		let _release = ReadRelease {
			accounting,
			payload: payload.cast::<u8>(),
			drop: drop_payload::<M>,
			movable,
		};
		// SAFETY: This listener's owed read keeps the payload alive until the
		// release guard gives it up after the clone finishes or unwinds.
		unsafe { (*payload).clone() }
	}

	/// Records that a listener stopped reading a chunk, recycling it if it was the last one.
	fn leave_chunk(&self, chunk: u32) {
		if self.core.pool.chunks[chunk as usize].pending.fetch_sub(1, Ordering::SeqCst) == 1 {
			self.core.release_chunks(&self.state);
		}
	}

	/// Releases a listener's remaining reads and chunk holds.
	fn unsubscribe(&self, cursor: SlotCursor, shared: &Arc<ListenerShared>) {
		let layout = self.state.layout;
		let pool = &self.core.pool;
		let state = &self.state;
		let control = self.core.control.lock();
		// Leaving changes the epoch first so publishers reserving from now on stop counting this listener.
		let epoch = tag(leave(&state.membership)) + 1;
		let (current, end_sequence) = state.frontier(pool, Ordering::SeqCst);

		if std::mem::needs_drop::<M>() {
			let tick_start = state.tick_start.load(Ordering::Acquire);
			let mut unread = cursor;
			while unread.sequence < end_sequence {
				unread.enter_next_chunk(pool, layout);
				unread.wait_committed(pool, state);
				let accounting = pool.accounting(unread.chunk, unread.slot, layout);
				if published_before(accounting.epoch.load(Ordering::Relaxed), epoch) {
					release_read(
						accounting,
						pool.payload_bytes(unread.chunk, unread.slot, layout),
						drop_payload::<M>,
						unread.sequence < tick_start,
					);
				}
				unread.step();
			}
		}

		// Leave every chunk from the cursor's to the current one. Read each link
		// before leaving because the chunk may be recycled right after.
		let mut chunk = cursor.chunk;
		let mut released = false;
		loop {
			let meta = &pool.chunks[chunk as usize];
			let next = meta.next.load(Ordering::Acquire);
			released |= meta.pending.fetch_sub(1, Ordering::SeqCst) == 1;
			if chunk == current {
				break;
			}
			chunk = next;
		}
		state.listeners.lock().retain(|listener| !Arc::ptr_eq(listener, shared));
		drop(control);
		if released {
			self.core.release_chunks(state);
		}
	}
}

/// Adds one listener to a route's membership and returns the previous word.
fn join(membership: &AtomicU64) -> u64 {
	membership
		.try_update(Ordering::SeqCst, Ordering::SeqCst, |word| {
			Some(tagged(tag(word).wrapping_add(1), value(word) + 1))
		})
		.expect("The membership update always produces a value")
}

/// Removes one listener from a route's membership and returns the previous word.
fn leave(membership: &AtomicU64) -> u64 {
	membership
		.try_update(Ordering::SeqCst, Ordering::SeqCst, |word| {
			debug_assert!(value(word) > 0, "A listener left a route that counts no listeners");
			Some(tagged(tag(word).wrapping_add(1), value(word) - 1))
		})
		.expect("The membership update always produces a value")
}

#[derive(Clone, Copy)]
/// The `SlotCursor` struct addresses one slot of a route by chunk, slot, and sequence.
///
/// A cursor whose slot equals the route's slots per chunk is parked at the end
/// of a full chunk: it moves to the successor's first slot on its next use once
/// that chunk is attached.
struct SlotCursor {
	chunk: u32,
	slot: u32,
	sequence: u64,
}

impl SlotCursor {
	/// Moves to the following slot without leaving the chunk.
	fn step(&mut self) {
		self.slot += 1;
		self.sequence += 1;
	}

	/// Moves a parked cursor to the start of the attached successor and returns the chunk it left.
	fn enter_next_chunk(&mut self, pool: &Pool, layout: SlotLayout) -> Option<u32> {
		if self.slot != layout.slots_per_chunk {
			return None;
		}
		let next = pool.chunks[self.chunk as usize].next.load(Ordering::Acquire);
		if next == NONE {
			return None;
		}
		let left = self.chunk;
		self.chunk = next;
		self.slot = 0;
		Some(left)
	}

	/// Returns whether the publisher of the slot at this cursor has committed it.
	fn is_committed(&self, pool: &Pool, state: &TopicState) -> bool {
		pool.stamp(self.chunk, self.slot, state.layout).load(Ordering::Acquire) == state.stamp(self.sequence)
	}

	/// Waits for the slot at this cursor to be committed. Callers hold the control lock.
	fn wait_committed(&self, pool: &Pool, state: &TopicState) {
		let mut spins = 0;
		// A reserved slot is committed by a publisher that holds no lock, so waiting here cannot deadlock.
		while !self.is_committed(pool, state) {
			backoff(&mut spins);
		}
	}
}

/// The `ListenerToken` struct owns one route cursor for a typed listener.
pub(crate) struct ListenerToken<M>
where
	M: Clone + Send + Sync + 'static,
{
	topic: Arc<Topic<M>>,
	cursor: SlotCursor,
	shared: Arc<ListenerShared>,
}

impl<M> ListenerToken<M>
where
	M: Clone + Send + Sync + 'static,
{
	pub(crate) fn read(&mut self) -> Option<M> {
		self.topic.read(&mut self.cursor, &self.shared)
	}

	pub(crate) fn new_listener(&self, created_at: &'static Location<'static>) -> Self {
		self.topic.subscribe(created_at)
	}
}

impl<M> Drop for ListenerToken<M>
where
	M: Clone + Send + Sync + 'static,
{
	fn drop(&mut self) {
		self.topic.unsubscribe(self.cursor, &self.shared);
	}
}

/// The `CursorAdvance` struct moves a listener past a slot even when cloning its payload unwinds.
struct CursorAdvance<'listener> {
	cursor: &'listener mut SlotCursor,
	shared: &'listener ListenerShared,
}

impl Drop for CursorAdvance<'_> {
	fn drop(&mut self) {
		self.cursor.step();
		self.shared.next_sequence.store(self.cursor.sequence, Ordering::Relaxed);
	}
}

/// The `ReadRelease` struct gives up one owed read after cloning finishes or unwinds.
struct ReadRelease<'slot> {
	accounting: &'slot SlotAccounting,
	payload: *mut u8,
	drop: unsafe fn(*mut u8),
	movable: bool,
}

impl Drop for ReadRelease<'_> {
	fn drop(&mut self) {
		release_read(self.accounting, self.payload, self.drop, self.movable);
	}
}

/// Returns whether a record stamped with `record_epoch` was published before membership epoch `epoch`.
///
/// Epochs wrap, so the comparison is a signed distance.
fn published_before(record_epoch: u32, epoch: u32) -> bool {
	(record_epoch.wrapping_sub(epoch) as i32) < 0
}

#[cfg(test)]
mod tests {
	use std::{
		panic::{AssertUnwindSafe, catch_unwind},
		sync::Barrier,
		sync::atomic::{AtomicUsize, Ordering},
		time::{Duration, Instant},
	};

	use super::{CHUNK_BYTES, MessageBus, MessageBusConfig, MessageRouteError};
	use crate::core::{
		channel::{Channel as _, DefaultChannel},
		listener::Listener as _,
	};

	/// Creates a bus without ticks whose pool holds exactly `chunks` chunks.
	fn test_bus(chunks: usize) -> MessageBus {
		MessageBus::new(MessageBusConfig {
			capacity: chunks * CHUNK_BYTES,
			ticks: false,
		})
		.expect("valid test bus")
	}

	/// Creates a ticking bus whose pool holds exactly `chunks` chunks.
	fn ticking_bus(chunks: usize) -> MessageBus {
		MessageBus::new(MessageBusConfig::new(chunks * CHUNK_BYTES)).expect("valid test bus")
	}

	/// Returns how many `u64` messages one chunk holds: each slot is an 8-byte stamp plus the value.
	fn u64_slots_per_chunk() -> usize {
		CHUNK_BYTES / 16
	}

	/// Publishes one producer's tagged sequence after every producer is ready.
	fn publish_tagged_sequence(
		channel: DefaultChannel<(usize, usize)>,
		start: &Barrier,
		producer: usize,
		message_count: usize,
	) {
		start.wait();
		for sequence in 0..message_count {
			channel.send((producer, sequence));
		}
	}

	#[test]
	fn a_burst_larger_than_any_chunk_is_delivered_in_order_without_draining() {
		let bus = test_bus(16);
		let channel = bus.new_scope("application").channel::<u64>();
		let mut listener = channel.listener();
		let burst = 5 * u64_slots_per_chunk() as u64 + 3;

		for value in 0..burst {
			channel.send(value);
		}

		assert_eq!(bus.topics()[0].chunks, 6);
		assert_eq!(listener.to_vec(), (0..burst).collect::<Vec<_>>());
		assert_eq!(listener.read(), None);
	}

	#[test]
	fn chunks_return_to_the_pool_once_every_listener_passes_them() {
		let bus = test_bus(2);
		let channel = bus.new_scope("application").channel::<u64>();
		let mut listener = channel.listener();
		let slots = u64_slots_per_chunk() as u64;

		for round in 0..10 {
			for value in 0..slots {
				channel.send(round * slots + value);
			}
			assert_eq!(listener.to_vec(), (round * slots..(round + 1) * slots).collect::<Vec<_>>());
		}

		assert_eq!(bus.pool().waits, 0);
		assert!(bus.topics()[0].chunks <= 2);
	}

	#[test]
	fn a_route_nobody_reads_pins_only_its_own_chunks() {
		let bus = test_bus(4);
		let messages = bus.new_scope("application");
		let pinned = messages.channel::<u64>();
		let flowing = messages.channel::<i64>();
		let _unread = pinned.listener();
		let mut reader = flowing.listener();
		let slots = u64_slots_per_chunk() as u64;

		// The unread route fills two chunks and keeps both.
		for value in 0..2 * slots - 1 {
			pinned.send(value);
		}
		// The flowing route cycles far more than the two remaining chunks.
		for value in 0..6 * slots as i64 {
			flowing.send(value);
			assert_eq!(reader.read(), Some(value));
		}

		assert_eq!(bus.pool().waits, 0);
	}

	#[test]
	fn a_publisher_waits_for_the_slowest_listener_instead_of_dropping() {
		let bus = test_bus(2);
		let channel = bus.new_scope("application").channel::<u64>();
		let mut listener = channel.listener();
		let slots = u64_slots_per_chunk() as u64;
		// Two chunks hold two chunks of messages; the next one needs a third chunk.
		for value in 0..2 * slots {
			channel.send(value);
		}

		let blocked = std::thread::spawn(move || channel.send(2 * slots));
		let deadline = Instant::now() + Duration::from_secs(5);
		while bus.pool().waits == 0 {
			assert!(Instant::now() < deadline, "the publisher did not wait for a chunk");
			std::thread::yield_now();
		}
		assert!(!blocked.is_finished());

		// Reading the first chunk and stepping into the second returns the first one.
		for value in 0..2 * slots {
			assert_eq!(listener.read(), Some(value));
		}
		blocked.join().expect("the publisher completes once a chunk is free");
		assert_eq!(listener.read(), Some(2 * slots));
		assert_eq!(listener.read(), None);
	}

	#[test]
	fn equal_message_types_in_independent_scopes_do_not_cross_routes() {
		let bus = test_bus(4);
		let left = bus.new_scope("left").channel::<u64>();
		let right = bus.new_scope("right").channel::<u64>();
		let mut left_listener = left.listener();
		let mut right_listener = right.listener();

		left.send(11);
		assert_eq!(left_listener.read(), Some(11));
		assert_eq!(right_listener.read(), None);

		right.send(29);
		assert_eq!(right_listener.read(), Some(29));
		assert_eq!(left_listener.read(), None);
		assert_eq!(bus.topics().len(), 2);
	}

	#[test]
	fn listeners_created_later_in_a_tick_receive_its_earlier_messages() {
		let bus = ticking_bus(4);
		let channel = bus.new_scope("application").channel::<u64>();
		channel.send(1);
		channel.send(2);

		let mut same_tick = channel.listener();
		assert_eq!(same_tick.to_vec(), [1, 2]);

		bus.begin_tick();
		let mut next_tick = channel.listener();
		assert_eq!(next_tick.read(), None);

		channel.send(3);
		assert_eq!(same_tick.to_vec(), [3]);
		assert_eq!(next_tick.to_vec(), [3]);
	}

	#[test]
	fn tick_replay_spans_every_chunk_the_tick_filled() {
		let bus = ticking_bus(8);
		let channel = bus.new_scope("application").channel::<u64>();
		let count = 3 * u64_slots_per_chunk() as u64 + 1;
		for value in 0..count {
			channel.send(value);
		}

		let mut late = channel.listener();

		assert_eq!(late.to_vec(), (0..count).collect::<Vec<_>>());
	}

	#[test]
	fn a_bus_without_ticks_keeps_listeners_future_only() {
		let bus = test_bus(2);
		let channel = bus.new_scope("application").channel::<u64>();
		channel.send(1);

		let mut late = channel.listener();
		channel.send(2);

		assert_eq!(late.to_vec(), [2]);
	}

	#[test]
	fn passive_observation_collapses_publications_without_applying_backpressure() {
		let bus = test_bus(2);
		let observer = bus.observe().expect("attach observer");
		let channel = bus.new_scope("application").channel::<u64>();
		let mut listener = channel.listener();

		channel.send(3);
		channel.send(5);

		assert_eq!(listener.to_vec(), [3, 5]);
		let batch = observer.drain_messages(&bus.topics());
		assert_eq!(batch.messages().len(), 1);
		assert_eq!(batch.messages()[0].first_sequence(), 0);
		assert_eq!(batch.messages()[0].count(), 2);
		assert!(observer.drain_messages(&bus.topics()).messages().is_empty());
	}

	#[repr(align(8192))]
	#[derive(Clone, Copy)]
	/// The `Overaligned` struct exercises route alignment validation.
	struct Overaligned;

	#[test]
	fn lazy_routes_report_payload_limits() {
		let bus = test_bus(2);
		let messages = bus.new_scope("application");

		match messages.try_channel::<[u8; CHUNK_BYTES]>() {
			Err(MessageRouteError::MessageTooLarge {
				message_bytes,
				chunk_bytes,
				..
			}) => assert_eq!((message_bytes, chunk_bytes), (CHUNK_BYTES, CHUNK_BYTES)),
			Err(error) => panic!("expected oversized payload error, got {error}"),
			Ok(_) => panic!("an oversized payload acquired a route"),
		}

		match messages.try_channel::<Overaligned>() {
			Err(MessageRouteError::MessageOveraligned {
				required_alignment,
				chunk_alignment,
				..
			}) => assert_eq!((required_alignment, chunk_alignment), (8192, 4096)),
			Err(error) => panic!("expected over-aligned payload error, got {error}"),
			Ok(_) => panic!("an over-aligned payload acquired a route"),
		}
	}

	#[test]
	fn snapshots_report_the_slowest_listener_and_where_it_was_created() {
		let bus = test_bus(2);
		let channel = bus.new_scope("application").channel::<u64>();
		let mut reading = channel.listener();
		let _stalled = channel.listener();
		let created_line = line!() - 1;

		for value in 0..5 {
			channel.send(value);
		}
		assert_eq!(reading.to_vec(), [0, 1, 2, 3, 4]);

		let snapshot = &bus.topics()[0];
		assert_eq!(snapshot.active_listeners, 2);
		assert_eq!(snapshot.published, 5);
		let slowest = snapshot.slowest_listener.expect("two listeners are registered");
		assert_eq!(slowest.behind, 5);
		assert_eq!(slowest.created_at.line(), created_line);
		assert!(slowest.created_at.file().ends_with("message_bus.rs"));
	}

	/// Leaks one counter so `'static` messages can report back to the test without shared ownership.
	fn counter() -> &'static AtomicUsize {
		Box::leak(Box::new(AtomicUsize::new(0)))
	}

	#[derive(Clone)]
	/// The `DropTracked` struct counts destruction of retained values.
	struct DropTracked {
		drops: &'static AtomicUsize,
	}

	impl Drop for DropTracked {
		fn drop(&mut self) {
			self.drops.fetch_add(1, Ordering::Relaxed);
		}
	}

	#[test]
	fn unread_values_drop_when_the_last_listener_leaves() {
		let drops = counter();
		let bus = test_bus(2);
		let messages = bus.new_scope("application");
		let channel = messages.channel::<DropTracked>();
		let first = channel.listener();
		let second = channel.listener();

		for _ in 0..2 {
			channel.send(DropTracked { drops });
		}

		drop(first);
		assert_eq!(drops.load(Ordering::Relaxed), 0, "the second listener still owes both reads");
		drop(second);
		assert_eq!(drops.load(Ordering::Relaxed), 2);

		drop(channel);
		drop(messages);
		drop(bus);
		assert_eq!(drops.load(Ordering::Relaxed), 2);
	}

	#[test]
	fn dropping_a_listener_parked_at_the_end_of_a_full_chunk_releases_the_chunk() {
		let drops = counter();
		let bus = test_bus(4);
		let channel = bus.new_scope("application").channel::<DropTracked>();
		let mut parked = channel.listener();
		let mut published = 0;
		while bus.topics()[0].chunks < 2 {
			channel.send(DropTracked { drops });
			published += 1;
		}
		// Read the whole first chunk, which leaves the one message in the second chunk unread.
		for _ in 0..published - 1 {
			parked.read().expect("delivery");
		}
		assert_eq!(drops.load(Ordering::Relaxed), published - 1);

		drop(parked);

		assert_eq!(drops.load(Ordering::Relaxed), published, "the unread message is released");
		assert_eq!(bus.topics()[0].chunks, 1, "the first chunk returned to the pool");
	}

	#[test]
	fn retained_values_drop_with_the_bus() {
		let drops = counter();
		let bus = ticking_bus(2);
		let channel = bus.new_scope("application").channel::<DropTracked>();
		let _listener = channel.listener();
		channel.send(DropTracked { drops });
		channel.send(DropTracked { drops });

		drop(_listener);
		drop(channel);
		drop(bus);

		assert_eq!(drops.load(Ordering::Relaxed), 2);
	}

	#[test]
	fn same_tick_replay_keeps_exact_ownership_of_retained_values() {
		let drops = counter();
		let bus = ticking_bus(2);
		let channel = bus.new_scope("application").channel::<DropTracked>();
		let mut early = channel.listener();
		channel.send(DropTracked { drops });
		channel.send(DropTracked { drops });
		let mut late = channel.listener();

		let early_values = early.to_vec();
		let late_values = late.to_vec();
		assert_eq!((early_values.len(), late_values.len()), (2, 2));
		drop((early_values, late_values));
		assert_eq!(drops.load(Ordering::Relaxed), 4, "each listener dropped its own clones");

		bus.begin_tick();
		assert_eq!(
			drops.load(Ordering::Relaxed),
			6,
			"the sweep destroys both originals exactly once"
		);
		drop(late);
		drop(early);
		drop(channel);
		drop(bus);
		assert_eq!(drops.load(Ordering::Relaxed), 6);
	}

	/// The `CloneTracked` struct reports whether delivery cloned or moved its retained value.
	struct CloneTracked {
		value: u32,
		clones: &'static AtomicUsize,
		drops: &'static AtomicUsize,
	}

	impl Clone for CloneTracked {
		fn clone(&self) -> Self {
			self.clones.fetch_add(1, Ordering::Relaxed);
			Self {
				value: self.value,
				clones: self.clones,
				drops: self.drops,
			}
		}
	}

	impl Drop for CloneTracked {
		fn drop(&mut self) {
			self.drops.fetch_add(1, Ordering::Relaxed);
		}
	}

	#[test]
	fn multiple_listeners_clone_then_move_one_retained_message() {
		let clones = counter();
		let drops = counter();
		let bus = test_bus(2);
		let channel = bus.new_scope("application").channel::<CloneTracked>();
		let mut first = channel.listener();
		let mut second = channel.listener();
		channel.send(CloneTracked {
			value: 73,
			clones,
			drops,
		});

		let first_value = first.read().expect("first delivery");
		let second_value = second.read().expect("second delivery");
		assert_eq!((first_value.value, second_value.value), (73, 73));
		assert_eq!(clones.load(Ordering::Relaxed), 1);
		drop((first_value, second_value));
		assert_eq!(drops.load(Ordering::Relaxed), 2);
	}

	/// The `PanicClone` struct exercises unwind cleanup for a reserved clone reader.
	struct PanicClone {
		value: u32,
		clone_attempts: &'static AtomicUsize,
		drops: &'static AtomicUsize,
	}

	impl Clone for PanicClone {
		fn clone(&self) -> Self {
			self.clone_attempts.fetch_add(1, Ordering::Relaxed);
			panic!("intentional clone failure")
		}
	}

	impl Drop for PanicClone {
		fn drop(&mut self) {
			self.drops.fetch_add(1, Ordering::Relaxed);
		}
	}

	#[test]
	fn a_panicking_clone_advances_its_cursor_and_releases_its_read() {
		let clone_attempts = counter();
		let drops = counter();
		let bus = test_bus(2);
		let channel = bus.new_scope("application").channel::<PanicClone>();
		let mut panicking = channel.listener();
		let mut last = channel.listener();
		let make_message = |value| PanicClone {
			value,
			clone_attempts,
			drops,
		};
		channel.send(make_message(1));

		let failure = catch_unwind(AssertUnwindSafe(|| panicking.read()));
		assert!(failure.is_err());
		assert_eq!(clone_attempts.load(Ordering::Relaxed), 1);
		let retained = last.read().expect("last listener moves the retained original");
		assert_eq!(retained.value, 1);
		drop(retained);
		assert_eq!(drops.load(Ordering::Relaxed), 1);

		channel.send(make_message(2));
		assert!(catch_unwind(AssertUnwindSafe(|| panicking.read())).is_err());
		assert_eq!(
			clone_attempts.load(Ordering::Relaxed),
			2,
			"the panicking listener skipped message 1"
		);
		drop(panicking);
		drop(last);
		assert_eq!(drops.load(Ordering::Relaxed), 2);
	}

	#[test]
	fn concurrent_producers_preserve_the_complete_broadcast_order() {
		const PRODUCERS: usize = 4;
		const MESSAGES_PER_PRODUCER: usize = 10_000;
		const TOTAL_MESSAGES: usize = PRODUCERS * MESSAGES_PER_PRODUCER;

		let bus = test_bus(64);
		let channel = bus.new_scope("application").channel::<(usize, usize)>();
		let mut first_listener = channel.listener();
		let mut second_listener = channel.listener();
		let start = Barrier::new(PRODUCERS);

		std::thread::scope(|threads| {
			for producer in 0..PRODUCERS {
				let channel = channel.clone();
				let start = &start;
				threads.spawn(move || publish_tagged_sequence(channel, start, producer, MESSAGES_PER_PRODUCER));
			}
		});

		let first = first_listener.to_vec();
		let second = second_listener.to_vec();
		assert_eq!(first.len(), TOTAL_MESSAGES);
		assert_eq!(first, second, "each listener must observe the same global order");

		let mut complete_set = first.clone();
		complete_set.sort_unstable();
		let expected = (0..PRODUCERS)
			.flat_map(|producer| (0..MESSAGES_PER_PRODUCER).map(move |sequence| (producer, sequence)))
			.collect::<Vec<_>>();
		assert_eq!(complete_set, expected);

		for producer in 0..PRODUCERS {
			let observed = first
				.iter()
				.filter_map(|&(source, sequence)| (source == producer).then_some(sequence))
				.collect::<Vec<_>>();
			assert_eq!(observed, (0..MESSAGES_PER_PRODUCER).collect::<Vec<_>>());
		}
	}

	#[test]
	fn concurrent_producers_and_a_draining_consumer_recycle_a_small_pool() {
		const PRODUCERS: usize = 4;
		const MESSAGES_PER_PRODUCER: usize = 20_000;

		let bus = test_bus(3);
		let channel = bus.new_scope("application").channel::<(usize, usize)>();
		let mut listener = channel.listener();
		let start = Barrier::new(PRODUCERS + 1);

		std::thread::scope(|threads| {
			for producer in 0..PRODUCERS {
				let channel = channel.clone();
				let start = &start;
				threads.spawn(move || publish_tagged_sequence(channel, start, producer, MESSAGES_PER_PRODUCER));
			}
			start.wait();
			let mut next_expected = [0; PRODUCERS];
			let mut received = 0;
			while received < PRODUCERS * MESSAGES_PER_PRODUCER {
				let Some((producer, sequence)) = listener.read() else {
					std::thread::yield_now();
					continue;
				};
				assert_eq!(sequence, next_expected[producer]);
				next_expected[producer] += 1;
				received += 1;
			}
		});

		assert_eq!(listener.read(), None);
	}
}
