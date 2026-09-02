//! Startup-sized storage shared by the engine's typed message routes.
//!
//! Create one [`MessageBus`] during application startup, then create a
//! [`MessageScope`] for each independently owned group of channels. Message
//! types are registered lazily the first time a channel requests them.

#![allow(
	unsafe_code,
	reason = "The fixed heterogeneous arena needs typed access to validated raw payload cells."
)]

use std::{
	alloc::{Layout, alloc, dealloc, handle_alloc_error},
	any::{Any, TypeId, type_name},
	collections::HashMap,
	fmt,
	marker::PhantomData,
	ptr::NonNull,
	sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
	sync::{Arc, OnceLock},
};

use utils::sync::Mutex;

use crate::core::{
	channel::{DefaultChannel, TrySendError},
	factory::Factory,
	message_observer::{MessageObservationError, MessageObserver},
};

/// The `MessageBusConfig` struct defines the fixed storage limits reserved at startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageBusConfig {
	/// Maximum number of typed routes across every scope.
	pub max_topics: usize,
	/// Number of fixed cells reserved for each typed route.
	pub cells_per_topic: usize,
	/// Number of payload bytes in one cell.
	pub cell_bytes: usize,
	/// Alignment of every payload cell.
	pub cell_alignment: usize,
	/// Maximum number of simultaneous listeners on one typed route.
	pub max_listeners_per_topic: usize,
}

impl Default for MessageBusConfig {
	fn default() -> Self {
		Self {
			max_topics: 64,
			cells_per_topic: 512,
			cell_bytes: 256,
			cell_alignment: 64,
			max_listeners_per_topic: 64,
		}
	}
}

impl MessageBusConfig {
	/// Creates fixed message storage with the supplied route and cell limits.
	///
	/// Next, adjust alignment or listener limits if the application needs values
	/// outside the defaults, then pass the result to [`MessageBus::new`].
	pub fn new(max_topics: usize, cells_per_topic: usize, cell_bytes: usize) -> Self {
		Self {
			max_topics,
			cells_per_topic,
			cell_bytes,
			..Self::default()
		}
	}

	/// Returns this configuration with a replacement payload alignment.
	pub fn with_cell_alignment(mut self, cell_alignment: usize) -> Self {
		self.cell_alignment = cell_alignment;
		self
	}

	/// Returns this configuration with a replacement per-topic listener limit.
	pub fn with_max_listeners_per_topic(mut self, max_listeners_per_topic: usize) -> Self {
		self.max_listeners_per_topic = max_listeners_per_topic;
		self
	}

	/// Builds the smallest one-route arena that preserves standalone channel capacity.
	pub(crate) fn standalone<M>(capacity: usize) -> Self {
		let alignment = std::mem::align_of::<M>().max(1);
		let bytes =
			align_up(std::mem::size_of::<M>().max(1), alignment).expect("A Rust message layout must fit in addressable memory");

		Self {
			max_topics: 1,
			cells_per_topic: capacity,
			cell_bytes: bytes,
			cell_alignment: alignment,
			max_listeners_per_topic: 128,
		}
	}

	/// Validates limits and computes the fixed arena layout.
	fn validate(self) -> Result<ValidatedConfig, MessageBusConfigError> {
		if self.max_topics == 0 {
			return Err(MessageBusConfigError::ZeroLimit("max_topics"));
		}
		if self.cells_per_topic == 0 {
			return Err(MessageBusConfigError::ZeroLimit("cells_per_topic"));
		}
		if self.cell_bytes == 0 {
			return Err(MessageBusConfigError::ZeroLimit("cell_bytes"));
		}
		if self.cell_alignment == 0 || !self.cell_alignment.is_power_of_two() {
			return Err(MessageBusConfigError::InvalidAlignment(self.cell_alignment));
		}
		if !self.cell_bytes.is_multiple_of(self.cell_alignment) {
			return Err(MessageBusConfigError::MisalignedCellSize {
				cell_bytes: self.cell_bytes,
				cell_alignment: self.cell_alignment,
			});
		}
		if self.max_listeners_per_topic == 0 {
			return Err(MessageBusConfigError::ZeroLimit("max_listeners_per_topic"));
		}

		let total_cells = self
			.max_topics
			.checked_mul(self.cells_per_topic)
			.ok_or(MessageBusConfigError::StorageSizeOverflow)?;
		let payload_bytes = total_cells
			.checked_mul(self.cell_bytes)
			.ok_or(MessageBusConfigError::StorageSizeOverflow)?;
		let total_listeners = self
			.max_topics
			.checked_mul(self.max_listeners_per_topic)
			.ok_or(MessageBusConfigError::StorageSizeOverflow)?;
		let payload_layout = Layout::from_size_align(payload_bytes, self.cell_alignment)
			.map_err(|_| MessageBusConfigError::StorageSizeOverflow)?;

		Ok(ValidatedConfig {
			config: self,
			total_cells,
			total_listeners,
			payload_layout,
		})
	}
}

/// The `MessageBusConfigError` enum explains why startup storage cannot be allocated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageBusConfigError {
	/// A required limit was zero.
	ZeroLimit(&'static str),
	/// The configured cell alignment was not a nonzero power of two.
	InvalidAlignment(usize),
	/// The cell size was not a multiple of its alignment.
	MisalignedCellSize { cell_bytes: usize, cell_alignment: usize },
	/// The requested arena size exceeded addressable memory.
	StorageSizeOverflow,
}

impl fmt::Display for MessageBusConfigError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::ZeroLimit(name) => write!(
				formatter,
				"Message bus limit '{name}' must be greater than zero. The most likely cause is a zero-valued startup parameter."
			),
			Self::InvalidAlignment(alignment) => write!(
				formatter,
				"Message bus cell alignment {alignment} is invalid. The most likely cause is that the configured alignment is not a power of two."
			),
			Self::MisalignedCellSize {
				cell_bytes,
				cell_alignment,
			} => write!(
				formatter,
				"Message bus cell size {cell_bytes} is not aligned to {cell_alignment}. The most likely cause is that cell_bytes is not a multiple of cell_alignment."
			),
			Self::StorageSizeOverflow => write!(
				formatter,
				"Message bus storage size is too large. The most likely cause is that the startup topic, cell, or byte limits multiply beyond addressable memory."
			),
		}
	}
}

impl std::error::Error for MessageBusConfigError {}

/// The `MessageRouteError` enum explains why a lazy typed route cannot be acquired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageRouteError {
	/// The bus has no unclaimed topic slab.
	TopicLimit { message_type: &'static str, max_topics: usize },
	/// One message cannot fit in its topic slab.
	MessageTooLarge {
		message_type: &'static str,
		message_bytes: usize,
		available_bytes: usize,
	},
	/// The message requires stricter alignment than the arena provides.
	MessageOveraligned {
		message_type: &'static str,
		required_alignment: usize,
		cell_alignment: usize,
	},
	/// The route has no free listener descriptor.
	ListenerLimit {
		message_type: &'static str,
		max_listeners: usize,
	},
}

impl fmt::Display for MessageRouteError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::TopicLimit {
				message_type,
				max_topics,
			} => write!(
				formatter,
				"Message bus topic limit {max_topics} was reached while registering '{message_type}'. The most likely cause is that more message types or independent scopes were created than the startup configuration allows."
			),
			Self::MessageTooLarge {
				message_type,
				message_bytes,
				available_bytes,
			} => write!(
				formatter,
				"Message type '{message_type}' needs {message_bytes} bytes, but one topic has {available_bytes} bytes. The most likely cause is that messages.cell-bytes or messages.cells-per-topic is too small."
			),
			Self::MessageOveraligned {
				message_type,
				required_alignment,
				cell_alignment,
			} => write!(
				formatter,
				"Message type '{message_type}' needs alignment {required_alignment}, but the bus provides {cell_alignment}. The most likely cause is that messages.cell-alignment is too small."
			),
			Self::ListenerLimit {
				message_type,
				max_listeners,
			} => write!(
				formatter,
				"Message route '{message_type}' already has {max_listeners} listeners. The most likely cause is that listeners were retained beyond their owning system or messages.listeners-per-topic is too small."
			),
		}
	}
}

impl std::error::Error for MessageRouteError {}

/// The `TopicSnapshot` struct reports one typed route's current state for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopicSnapshot {
	pub topic_id: usize,
	pub scope_id: u64,
	pub scope: Arc<str>,
	pub message_type: &'static str,
	pub capacity: usize,
	pub active_listeners: usize,
	pub queued_for_slowest_listener: usize,
	pub published: u64,
	pub full: u64,
	pub disconnected: u64,
}

/// The `MessageBus` struct owns one fixed heterogeneous payload arena and its lazy route registry.
#[derive(Clone)]
pub struct MessageBus {
	inner: Arc<MessageBusInner>,
}

impl Default for MessageBus {
	fn default() -> Self {
		Self::new(MessageBusConfig::default()).expect("The default message bus configuration must be valid")
	}
}

impl MessageBus {
	/// Allocates the complete payload arena and route metadata.
	///
	/// Next, call [`Self::new_scope`] for each owner that needs isolated typed
	/// routes, then acquire channels or factories from that scope.
	pub fn new(config: MessageBusConfig) -> Result<Self, MessageBusConfigError> {
		let validated = config.validate()?;
		let arena = Arc::new(Arena::new(validated));
		let registry = Registry {
			topics: HashMap::with_capacity(config.max_topics),
			next_topic: 0,
		};

		Ok(Self {
			inner: Arc::new(MessageBusInner {
				arena,
				registry: Mutex::new(registry),
				next_scope: AtomicU64::new(1),
			}),
		})
	}

	/// Returns the immutable startup limits for this bus.
	pub fn config(&self) -> MessageBusConfig {
		self.inner.arena.config
	}

	/// Attaches the one passive observer for this bus.
	///
	/// The observer sees successful publications from every future route,
	/// including application-defined generic types. Attach it before acquiring
	/// the first channel or factory. Publication observation reads existing route
	/// counters, so it never delays publishers.
	pub fn observe(&self) -> Result<MessageObserver, MessageObservationError> {
		let observer = MessageObserver::new(self.inner.arena.config.max_topics);
		let registry = self.inner.registry.lock();
		if self.inner.arena.observer.get().is_some() {
			return Err(MessageObservationError::AlreadyAttached);
		}
		if registry.next_topic != 0 {
			return Err(MessageObservationError::RoutesAlreadyRegistered);
		}
		self.inner
			.arena
			.observer
			.set(observer.clone())
			.map_err(|_| MessageObservationError::AlreadyAttached)?;
		drop(registry);
		Ok(observer)
	}

	/// Returns the diagnostics owner already attached to this bus.
	pub(crate) fn observer(&self) -> Option<MessageObserver> {
		self.inner.arena.observer.get().cloned()
	}

	/// Creates an isolated namespace over the same fixed arena.
	///
	/// Routes remain lazy: creating a scope does not claim a topic until code
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

	/// Returns diagnostic snapshots for every route registered so far.
	pub fn topics(&self) -> Vec<TopicSnapshot> {
		let registry = self.inner.registry.lock();
		let mut snapshots = registry
			.topics
			.values()
			.map(|record| record.diagnostics.snapshot())
			.collect::<Vec<_>>();
		snapshots.sort_unstable_by(|left, right| {
			left.scope_id
				.cmp(&right.scope_id)
				.then_with(|| left.message_type.cmp(right.message_type))
		});
		snapshots
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

	/// Returns diagnostic snapshots for routes registered in this scope.
	pub fn topics(&self) -> Vec<TopicSnapshot> {
		self.bus
			.topics()
			.into_iter()
			.filter(|snapshot| snapshot.scope_id == self.id)
			.collect()
	}

	/// Looks up or installs a typed topic while holding the control-plane registry lock.
	pub(crate) fn topic<M>(&self) -> Result<Arc<Topic<M>>, MessageRouteError>
	where
		M: Clone + Send + Sync + 'static,
	{
		let key = TopicKey {
			scope: self.id,
			message: TypeId::of::<M>(),
		};
		let mut registry = self.bus.inner.registry.lock();

		if let Some(record) = registry.topics.get(&key) {
			return Arc::downcast::<Topic<M>>(Arc::clone(&record.typed)).map_err(|_| unreachable_type_collision::<M>());
		}

		let layout = TopicLayout::for_message::<M>(&self.bus.inner.arena.config)?;
		if registry.next_topic == self.bus.inner.arena.config.max_topics {
			return Err(MessageRouteError::TopicLimit {
				message_type: type_name::<M>(),
				max_topics: self.bus.inner.arena.config.max_topics,
			});
		}

		let topic_index = registry.next_topic;
		registry.next_topic += 1;
		let topic = Arc::new(Topic::<M>::new(
			Arc::clone(&self.bus.inner.arena),
			topic_index,
			layout,
			self.id,
			Arc::clone(&self.name),
		));
		let typed: Arc<dyn Any + Send + Sync> = topic.clone();
		let diagnostics: Arc<dyn TopicDiagnostics> = topic.clone();
		registry.topics.insert(key, TopicRecord { typed, diagnostics });

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

/// The `MessageBusInner` struct keeps shared arena and registry ownership behind one handle.
struct MessageBusInner {
	arena: Arc<Arena>,
	registry: Mutex<Registry>,
	next_scope: AtomicU64,
}

/// The `Registry` struct maps lazy scoped types to their permanent topic slabs.
struct Registry {
	topics: HashMap<TopicKey, TopicRecord>,
	next_topic: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// The `TopicKey` struct identifies one message type inside one isolated scope.
struct TopicKey {
	scope: u64,
	message: TypeId,
}

/// The `TopicRecord` struct keeps typed lookup and type-erased diagnostics for one route.
struct TopicRecord {
	typed: Arc<dyn Any + Send + Sync>,
	diagnostics: Arc<dyn TopicDiagnostics>,
}

/// The `TopicDiagnostics` trait lets the bus inspect routes without knowing their message types.
trait TopicDiagnostics: Send + Sync {
	fn snapshot(&self) -> TopicSnapshot;
}

/// The `ValidatedConfig` struct carries overflow-checked startup allocation sizes.
struct ValidatedConfig {
	config: MessageBusConfig,
	total_cells: usize,
	total_listeners: usize,
	payload_layout: Layout,
}

/// The `Arena` struct owns every payload slab, route cursor, slot state, and listener descriptor.
struct Arena {
	config: MessageBusConfig,
	payload: PayloadArena,
	routes: Box<[RouteState]>,
	stamps: Box<[AtomicU64]>,
	remaining_readers: Box<[AtomicUsize]>,
	listeners: Box<[ListenerState]>,
	observer: OnceLock<MessageObserver>,
}

impl Arena {
	/// Allocates payload bytes and zeroed atomic metadata for every possible route.
	fn new(validated: ValidatedConfig) -> Self {
		let payload = PayloadArena::new(validated.payload_layout);
		let routes = (0..validated.config.max_topics)
			.map(|_| RouteState::default())
			.collect::<Vec<_>>()
			.into_boxed_slice();
		let stamps = (0..validated.total_cells)
			.map(|_| AtomicU64::new(0))
			.collect::<Vec<_>>()
			.into_boxed_slice();
		let remaining_readers = (0..validated.total_cells)
			.map(|_| AtomicUsize::new(0))
			.collect::<Vec<_>>()
			.into_boxed_slice();
		let listeners = (0..validated.total_listeners)
			.map(|_| ListenerState::default())
			.collect::<Vec<_>>()
			.into_boxed_slice();

		Self {
			config: validated.config,
			payload,
			routes,
			stamps,
			remaining_readers,
			listeners,
			observer: OnceLock::new(),
		}
	}
}

/// The `PayloadArena` struct owns the aligned raw allocation shared by typed topic slabs.
struct PayloadArena {
	pointer: NonNull<u8>,
	layout: Layout,
}

impl PayloadArena {
	/// Reserves the aligned payload allocation without constructing typed values.
	fn new(layout: Layout) -> Self {
		// SAFETY: The validated layout is nonzero and has a power-of-two alignment.
		let pointer = unsafe { alloc(layout) };
		let pointer = NonNull::new(pointer).unwrap_or_else(|| handle_alloc_error(layout));
		Self { pointer, layout }
	}
}

impl Drop for PayloadArena {
	fn drop(&mut self) {
		// SAFETY: `pointer` came from `alloc` with this exact layout, and typed
		// topics drop all initialized values before the last arena owner is gone.
		unsafe { dealloc(self.pointer.as_ptr(), self.layout) };
	}
}

// SAFETY: Payload access is partitioned by permanent topic slabs. Typed topics
// publish with release stamps and prevent reuse until every active listener has
// advanced past the prior value.
unsafe impl Send for PayloadArena {}
// SAFETY: See the `Send` implementation. Shared payload references exist only
// for message types that implement `Sync`.
unsafe impl Sync for PayloadArena {}

#[derive(Default)]
#[repr(align(64))]
/// The `RouteState` struct keeps synchronization and diagnostics local to one typed route.
struct RouteState {
	writer: Mutex<WriterState>,
}

#[derive(Default)]
/// The `WriterState` struct keeps data already protected by the per-route writer gate.
struct WriterState {
	next_ticket: u64,
	next_slot: usize,
	cached_minimum: u64,
	active_listeners: usize,
	full: u64,
	disconnected: u64,
}

#[derive(Default)]
/// The `ListenerState` struct exposes one preallocated consumer cursor to publishers.
struct ListenerState {
	active: AtomicBool,
	cursor: AtomicU64,
}

#[derive(Clone, Copy)]
/// The `TopicLayout` struct maps one message layout onto contiguous fixed cells.
struct TopicLayout {
	message_stride: usize,
	capacity: usize,
}

impl TopicLayout {
	/// Converts a Rust layout into a contiguous cell group inside one route slab.
	fn for_message<M>(config: &MessageBusConfig) -> Result<Self, MessageRouteError> {
		let required_alignment = std::mem::align_of::<M>();
		if required_alignment > config.cell_alignment {
			return Err(MessageRouteError::MessageOveraligned {
				message_type: type_name::<M>(),
				required_alignment,
				cell_alignment: config.cell_alignment,
			});
		}

		let message_bytes = std::mem::size_of::<M>().max(1);
		let cells_per_message = message_bytes.div_ceil(config.cell_bytes);
		if cells_per_message > config.cells_per_topic {
			return Err(MessageRouteError::MessageTooLarge {
				message_type: type_name::<M>(),
				message_bytes,
				available_bytes: config.cells_per_topic * config.cell_bytes,
			});
		}
		let message_stride = align_up(message_bytes, required_alignment)
			.expect("A validated Rust message layout must fit in addressable memory");
		debug_assert!(message_stride <= cells_per_message * config.cell_bytes);

		Ok(Self {
			message_stride,
			capacity: config.cells_per_topic / cells_per_message,
		})
	}
}

/// The `Topic` struct provides one cached typed view into a permanent bus slab.
pub(crate) struct Topic<M> {
	arena: Arc<Arena>,
	index: usize,
	layout: TopicLayout,
	payload_byte_offset: usize,
	stamp_offset: usize,
	listener_offset: usize,
	scope_id: u64,
	scope: Arc<str>,
	_marker: PhantomData<M>,
}

impl<M> Topic<M>
where
	M: Clone + Send + Sync + 'static,
{
	fn new(arena: Arc<Arena>, index: usize, layout: TopicLayout, scope_id: u64, scope: Arc<str>) -> Self {
		let payload_byte_offset = index * arena.config.cells_per_topic * arena.config.cell_bytes;
		let stamp_offset = index * arena.config.cells_per_topic;
		let listener_offset = index * arena.config.max_listeners_per_topic;
		Self {
			arena,
			index,
			layout,
			payload_byte_offset,
			stamp_offset,
			listener_offset,
			scope_id,
			scope,
			_marker: PhantomData,
		}
	}

	/// Registers a future-only listener at the latest committed cursor.
	pub(crate) fn subscribe(self: &Arc<Self>) -> Result<ListenerToken<M>, MessageRouteError> {
		let route = &self.arena.routes[self.index];
		let mut writer = route.writer.lock();
		// Holding the writer gate makes registration future-only at one exact
		// committed cursor without another lifecycle lock or atomic tail.
		let cursor = writer.next_ticket;

		for listener_index in self.listener_range() {
			let listener = &self.arena.listeners[listener_index];
			if listener.active.load(Ordering::Relaxed) {
				continue;
			}
			listener.cursor.store(cursor, Ordering::Relaxed);
			listener.active.store(true, Ordering::Relaxed);
			if writer.active_listeners == 0 {
				writer.cached_minimum = cursor;
			}
			writer.active_listeners += 1;
			return Ok(ListenerToken {
				topic: Arc::clone(self),
				listener_index,
				cursor,
				slot: writer.next_slot,
			});
		}

		Err(MessageRouteError::ListenerLimit {
			message_type: type_name::<M>(),
			max_listeners: self.arena.config.max_listeners_per_topic,
		})
	}

	/// Attempts one allocation-free publication into this route.
	pub(crate) fn try_send(&self, message: M) -> Result<(), TrySendError<M>> {
		let route = &self.arena.routes[self.index];
		let mut writer = route.writer.lock();

		if writer.active_listeners == 0 {
			writer.disconnected = writer.disconnected.wrapping_add(1);
			return Err(TrySendError::Disconnected(message));
		}

		let ticket = writer.next_ticket;
		if ticket == u64::MAX {
			return Err(TrySendError::SequenceExhausted(message));
		}

		let mut minimum = writer.cached_minimum;
		if ticket - minimum >= self.layout.capacity as u64 {
			minimum = self.minimum_active_cursor(ticket, writer.active_listeners);
			writer.cached_minimum = minimum;
			if ticket - minimum >= self.layout.capacity as u64 {
				return Err(TrySendError::Full(message));
			}
		}

		let slot = writer.next_slot;
		let stamp = self.stamp(slot);
		// SAFETY: This publisher owns `ticket`, and the capacity check prevents a
		// second live ticket from using the same slot.
		let value_pointer = unsafe { self.value_ptr(slot) };
		if std::mem::needs_drop::<M>() {
			debug_assert_eq!(
				stamp.load(Ordering::Relaxed),
				0,
				"Every owner must release a drop-bearing slot before reuse"
			);
			debug_assert_eq!(
				self.remaining_readers(slot).load(Ordering::Relaxed),
				0,
				"A released message slot cannot retain reader reservations"
			);
		}
		// SAFETY: Drop-bearing slots are empty after their last reader releases the
		// retained value. Overwriting a no-drop value needs no destructor call.
		unsafe { value_pointer.write(message) };
		if std::mem::needs_drop::<M>() {
			self.remaining_readers(slot).store(writer.active_listeners, Ordering::Relaxed);
		}
		stamp.store(ticket + 1, Ordering::Release);
		writer.next_ticket = ticket + 1;
		writer.next_slot = if slot + 1 == self.layout.capacity { 0 } else { slot + 1 };
		Ok(())
	}

	/// Returns the diagnostics owner attached to this route's bus.
	pub(crate) fn observer(&self) -> Option<&MessageObserver> {
		self.arena.observer.get()
	}

	/// Removes one terminally deleted handle from the optional entity catalog.
	#[inline(always)]
	pub(crate) fn forget_entity(&self, handle: crate::core::factory::Handle) {
		if let Some(observer) = self.arena.observer.get() {
			forget_observed_entity(observer, handle);
		}
	}

	/// Records one logical publication that encountered a full route.
	pub(crate) fn record_full(&self) {
		let mut writer = self.arena.routes[self.index].writer.lock();
		writer.full = writer.full.wrapping_add(1);
	}

	/// Finds the slowest active listener while the writer gate stabilizes membership.
	fn minimum_active_cursor(&self, tail: u64, active_listeners: usize) -> u64 {
		let mut minimum = tail;
		let mut visited = 0;
		for listener_index in self.listener_range() {
			let listener = &self.arena.listeners[listener_index];
			if !listener.active.load(Ordering::Relaxed) {
				continue;
			}
			minimum = minimum.min(listener.cursor.load(Ordering::Acquire));
			visited += 1;
			if visited == active_listeners {
				break;
			}
		}
		debug_assert_eq!(
			visited, active_listeners,
			"The route's listener count must match its active descriptors"
		);
		minimum
	}

	/// Clones the message at one listener cursor and advances it after normal return or unwind.
	fn read(&self, listener_index: usize, cursor: &mut u64, slot: &mut usize) -> Option<M> {
		let listener = &self.arena.listeners[listener_index];
		let ticket = *cursor;
		if ticket == u64::MAX {
			return None;
		}
		let current_slot = *slot;
		let stamp = self.stamp(current_slot).load(Ordering::Acquire);
		if stamp != ticket + 1 {
			debug_assert!(
				stamp == 0 || stamp < ticket + 1,
				"An active message listener was overtaken by its publisher"
			);
			return None;
		}
		let next_slot = if current_slot + 1 == self.layout.capacity {
			0
		} else {
			current_slot + 1
		};

		let _advance = CursorAdvanceGuard {
			local_cursor: cursor,
			local_slot: slot,
			shared: &listener.cursor,
			next_cursor: ticket + 1,
			next_slot,
		};
		// SAFETY: The validated topic layout places this slot inside its permanent
		// slab with the alignment required by M.
		let value = unsafe { self.value_ptr(current_slot) };
		if !std::mem::needs_drop::<M>() {
			// SAFETY: The matching acquire stamp proves initialization. Publishers
			// cannot reuse this slot until this active listener advances its cursor.
			return Some(unsafe { (&*value).clone() });
		}

		let remaining = self.remaining_readers(current_slot);
		let readers = remaining.load(Ordering::Relaxed);
		assert_ne!(readers, 0, "A stamped drop-bearing message must retain its current reader");
		if readers == 1 {
			remaining
				.compare_exchange(1, 0, Ordering::AcqRel, Ordering::Acquire)
				.expect("The sole retained reader cannot race another reservation");
			// Clear publication state before moving the retained value. The cursor
			// guard prevents a publisher from reusing this slot until the move ends.
			self.stamp(current_slot).store(0, Ordering::Release);
			// SAFETY: This CAS claimed the only remaining reader, so no clone can
			// still borrow the retained value.
			return Some(unsafe { value.read() });
		}

		let release = CloneReleaseGuard {
			remaining,
			stamp: self.stamp(current_slot),
			value,
		};
		// SAFETY: A count above one reserves this reader while it clones. Other
		// readers cannot move the retained value until this guard decrements it.
		let cloned = unsafe { (&*value).clone() };
		drop(release);
		Some(cloned)
	}

	/// Releases a listener descriptor so it no longer contributes backpressure.
	fn unsubscribe(&self, listener_index: usize, cursor: u64, slot: usize) {
		let route = &self.arena.routes[self.index];
		let mut writer = route.writer.lock();
		let listener = &self.arena.listeners[listener_index];
		let tail = writer.next_ticket;
		let mut release = ListenerReleaseGuard {
			topic: self,
			listener,
			writer: &mut writer,
			cursor,
			slot,
			tail,
			active: true,
		};
		release.run();
	}

	fn snapshot_inner(&self) -> TopicSnapshot {
		let route = &self.arena.routes[self.index];
		let writer = route.writer.lock();
		let tail = writer.next_ticket;
		let minimum = self.minimum_active_cursor(tail, writer.active_listeners);
		TopicSnapshot {
			topic_id: self.index,
			scope_id: self.scope_id,
			scope: Arc::clone(&self.scope),
			message_type: type_name::<M>(),
			capacity: self.layout.capacity,
			active_listeners: writer.active_listeners,
			queued_for_slowest_listener: tail.saturating_sub(minimum) as usize,
			published: tail,
			full: writer.full,
			disconnected: writer.disconnected,
		}
	}
}

/// Keeps the enabled diagnostics path out of ordinary deletion publication code.
#[cold]
#[inline(never)]
fn forget_observed_entity(observer: &MessageObserver, handle: crate::core::factory::Handle) {
	observer.forget_entity(handle);
}

impl<M> Topic<M> {
	/// Returns one message address inside this topic's densely packed payload slab.
	unsafe fn value_ptr(&self, slot: usize) -> *mut M {
		let byte_offset = self.payload_byte_offset + slot * self.layout.message_stride;
		// SAFETY: Layout validation proves that every aligned stride and the final
		// message remain inside this topic's permanent payload slab.
		unsafe { self.arena.payload.pointer.as_ptr().add(byte_offset).cast::<M>() }
	}

	fn stamp(&self, slot: usize) -> &AtomicU64 {
		&self.arena.stamps[self.stamp_offset + slot]
	}

	fn remaining_readers(&self, slot: usize) -> &AtomicUsize {
		&self.arena.remaining_readers[self.stamp_offset + slot]
	}

	fn listener_range(&self) -> std::ops::Range<usize> {
		self.listener_offset..self.listener_offset + self.arena.config.max_listeners_per_topic
	}
}

impl<M> TopicDiagnostics for Topic<M>
where
	M: Clone + Send + Sync + 'static,
{
	fn snapshot(&self) -> TopicSnapshot {
		self.snapshot_inner()
	}
}

impl<M> Drop for Topic<M> {
	fn drop(&mut self) {
		if !std::mem::needs_drop::<M>() {
			return;
		}
		let route = &self.arena.routes[self.index];
		let initialized = route.writer.lock().next_ticket.min(self.layout.capacity as u64) as usize;
		for slot in 0..initialized {
			if self.stamp(slot).swap(0, Ordering::AcqRel) != 0 {
				self.remaining_readers(slot).store(0, Ordering::Relaxed);
				// SAFETY: The validated topic layout places this slot inside its permanent
				// slab with the alignment required by M.
				let value = unsafe { self.value_ptr(slot) };
				// SAFETY: Each nonzero stamp identifies one retained initialized value,
				// and the final Topic drop means no publisher or listener can access it.
				unsafe { value.drop_in_place() };
			}
		}
	}
}

/// The `ListenerToken` struct owns one fixed cursor descriptor for a typed listener.
pub(crate) struct ListenerToken<M>
where
	M: Clone + Send + Sync + 'static,
{
	topic: Arc<Topic<M>>,
	listener_index: usize,
	cursor: u64,
	slot: usize,
}

impl<M> ListenerToken<M>
where
	M: Clone + Send + Sync + 'static,
{
	pub(crate) fn read(&mut self) -> Option<M> {
		self.topic.read(self.listener_index, &mut self.cursor, &mut self.slot)
	}

	pub(crate) fn new_listener(&self) -> Result<Self, MessageRouteError> {
		self.topic.subscribe()
	}
}

impl<M> Drop for ListenerToken<M>
where
	M: Clone + Send + Sync + 'static,
{
	fn drop(&mut self) {
		self.topic.unsubscribe(self.listener_index, self.cursor, self.slot);
	}
}

/// The `CloneReleaseGuard` struct releases one reader reservation after cloning finishes or unwinds.
struct CloneReleaseGuard<'slot, M> {
	remaining: &'slot AtomicUsize,
	stamp: &'slot AtomicU64,
	value: *mut M,
}

impl<M> Drop for CloneReleaseGuard<'_, M> {
	fn drop(&mut self) {
		release_retained_reader(self.remaining, self.stamp, self.value);
	}
}

/// The `ListenerReleaseGuard` struct releases unread reservations before returning a listener descriptor.
///
/// If one retained value's destructor unwinds, this guard continues from the
/// next ticket before it returns the descriptor to the route.
struct ListenerReleaseGuard<'listener, M> {
	topic: &'listener Topic<M>,
	listener: &'listener ListenerState,
	writer: &'listener mut WriterState,
	cursor: u64,
	slot: usize,
	tail: u64,
	active: bool,
}

impl<M> ListenerReleaseGuard<'_, M> {
	fn run(&mut self) {
		self.release_remaining();
		self.deactivate();
	}

	/// Releases every unread ticket while publication remains excluded by the writer gate.
	fn release_remaining(&mut self) {
		if !std::mem::needs_drop::<M>() {
			self.cursor = self.tail;
			return;
		}

		while self.cursor < self.tail {
			let ticket = self.cursor;
			let slot = self.slot;
			let stamp = self.topic.stamp(slot);
			debug_assert_eq!(
				stamp.load(Ordering::Acquire),
				ticket + 1,
				"An unread listener reservation must reference its published ticket"
			);

			// Advance cleanup state before running user Drop code so this guard can
			// continue at the following ticket if destruction unwinds.
			self.cursor = ticket + 1;
			self.slot = if slot + 1 == self.topic.layout.capacity { 0 } else { slot + 1 };
			// SAFETY: The matching stamp identifies one initialized retained M.
			let value = unsafe { self.topic.value_ptr(slot) };
			release_retained_reader(self.topic.remaining_readers(slot), stamp, value);
		}
	}

	fn deactivate(&mut self) {
		self.listener.cursor.store(self.tail, Ordering::Release);
		let was_active = self.listener.active.swap(false, Ordering::Relaxed);
		debug_assert!(was_active, "A message listener descriptor was released twice");
		if was_active {
			debug_assert!(self.writer.active_listeners > 0);
			self.writer.active_listeners -= 1;
			if self.writer.active_listeners == 0 {
				self.writer.cached_minimum = self.writer.next_ticket;
			}
		}
		self.active = false;
	}
}

impl<M> Drop for ListenerReleaseGuard<'_, M> {
	fn drop(&mut self) {
		if self.active {
			self.release_remaining();
			self.deactivate();
		}
	}
}

/// Releases one retained-reader reservation and destroys the original after the last clone.
fn release_retained_reader<M>(remaining: &AtomicUsize, stamp: &AtomicU64, value: *mut M) {
	let previous = remaining.fetch_sub(1, Ordering::AcqRel);
	assert_ne!(previous, 0, "A retained message reader reservation was released twice");
	if previous == 1 {
		// Clear publication state before user Drop code runs. The last reader's
		// cursor or the writer gate still prevents reuse during destruction.
		stamp.store(0, Ordering::Release);
		// SAFETY: The transition to zero proves that every clone is complete and
		// this caller exclusively owns destruction of the retained value.
		unsafe { value.drop_in_place() };
	}
}

/// The `CursorAdvanceGuard` struct releases a consumed slot even when message cloning unwinds.
struct CursorAdvanceGuard<'cursor> {
	local_cursor: &'cursor mut u64,
	local_slot: &'cursor mut usize,
	shared: &'cursor AtomicU64,
	next_cursor: u64,
	next_slot: usize,
}

impl Drop for CursorAdvanceGuard<'_> {
	fn drop(&mut self) {
		*self.local_cursor = self.next_cursor;
		*self.local_slot = self.next_slot;
		self.shared.store(self.next_cursor, Ordering::Release);
	}
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
	value.checked_add(alignment - 1).map(|value| value & !(alignment - 1))
}

fn unreachable_type_collision<M>() -> MessageRouteError {
	panic!(
		"Message route type collision for '{}'. The most likely cause is an internal TypeId registry error.",
		type_name::<M>()
	)
}

#[cfg(test)]
mod tests {
	use std::{
		any::type_name,
		panic::{AssertUnwindSafe, catch_unwind},
		sync::atomic::{AtomicUsize, Ordering},
		sync::{Arc, Barrier},
		time::{Duration, Instant},
	};

	use super::{MessageBus, MessageBusConfig, MessageRouteError};
	use crate::core::{
		channel::{Channel as _, DefaultChannel, TrySendError},
		listener::Listener as _,
		message_observer::MessageObservationError,
	};

	/// Creates a compact arena whose cells satisfy ordinary Rust value alignment.
	fn test_config(max_topics: usize, cells_per_topic: usize, cell_bytes: usize, max_listeners: usize) -> MessageBusConfig {
		MessageBusConfig::new(max_topics, cells_per_topic, cell_bytes)
			.with_cell_alignment(8)
			.with_max_listeners_per_topic(max_listeners)
	}

	/// Publishes one producer's tagged sequence after every producer is ready.
	fn publish_tagged_sequence(
		channel: DefaultChannel<(usize, usize)>,
		start: Arc<Barrier>,
		producer: usize,
		message_count: usize,
	) {
		start.wait();
		for sequence in 0..message_count {
			channel.send((producer, sequence));
		}
	}

	#[derive(Clone, Debug, PartialEq, Eq)]
	/// The `ApplicationMessage` struct represents an application-defined type unknown at startup.
	struct ApplicationMessage {
		value: u32,
	}

	#[test]
	fn custom_message_types_register_only_when_first_requested() {
		let bus = MessageBus::new(test_config(1, 4, 8, 1)).expect("valid test bus");
		let messages = bus.new_scope("application");
		assert!(bus.topics().is_empty());

		let channel = messages.channel::<ApplicationMessage>();
		let mut listener = channel.listener();
		channel
			.try_send(ApplicationMessage { value: 42 })
			.expect("registered route has capacity");

		assert_eq!(bus.topics().len(), 1);
		assert_eq!(listener.read(), Some(ApplicationMessage { value: 42 }));
	}

	#[test]
	fn passive_observation_resolves_distinct_generic_routes_without_payload_access() {
		let bus = MessageBus::new(test_config(2, 4, 8, 1)).expect("valid test bus");
		let observer = bus.observe().expect("attach observer");
		let messages = bus.new_scope("application");
		let integers = messages.channel::<Option<u32>>();
		let counters = messages.channel::<Option<u64>>();
		let _integer_listener = integers.listener();
		let _counter_listener = counters.listener();

		integers.send(Some(7));
		counters.send(Some(11));

		let batch = observer.drain_messages(&bus.topics());
		let topics = bus.topics();
		let observed_types = batch
			.messages()
			.iter()
			.map(|message| {
				topics
					.iter()
					.find(|topic| topic.topic_id == message.topic_id())
					.expect("observed topic remains registered")
					.message_type
			})
			.collect::<Vec<_>>();

		assert_eq!(observed_types, [type_name::<Option<u32>>(), type_name::<Option<u64>>()]);
		assert!(batch.messages().iter().all(|message| message.first_sequence() == 0));
		assert!(batch.messages().iter().all(|message| message.count() == 1));
	}

	#[test]
	fn passive_observation_collapses_publications_without_applying_backpressure() {
		let bus = MessageBus::new(test_config(1, 4, 8, 1)).expect("valid test bus");
		let observer = bus.observe().expect("attach observer");
		let channel = bus.new_scope("application").channel::<u64>();
		let mut listener = channel.listener();

		channel.try_send(3).expect("first message fits");
		channel
			.try_send(5)
			.expect("observer saturation does not fill the message route");

		assert_eq!(listener.to_vec(), [3, 5]);
		let batch = observer.drain_messages(&bus.topics());
		assert_eq!(batch.messages().len(), 1);
		assert_eq!(batch.messages()[0].first_sequence(), 0);
		assert_eq!(batch.messages()[0].count(), 2);
		assert!(observer.drain_messages(&bus.topics()).messages().is_empty());
	}

	#[test]
	fn one_bus_rejects_a_second_passive_observer() {
		let bus = MessageBus::new(test_config(1, 1, 8, 1)).expect("valid test bus");
		let _observer = bus.observe().expect("attach first observer");

		assert!(matches!(bus.observe(), Err(MessageObservationError::AlreadyAttached)));
	}

	#[test]
	fn passive_observation_must_start_before_route_registration() {
		let bus = MessageBus::new(test_config(1, 1, 8, 1)).expect("valid test bus");
		let _channel = bus.new_scope("application").channel::<u64>();

		assert!(matches!(bus.observe(), Err(MessageObservationError::RoutesAlreadyRegistered)));
	}

	#[test]
	fn equal_message_types_in_independent_scopes_do_not_cross_routes() {
		let bus = MessageBus::new(test_config(2, 4, 8, 1)).expect("valid test bus");
		let left = bus.new_scope("left").channel::<u64>();
		let right = bus.new_scope("right").channel::<u64>();
		let mut left_listener = left.listener();
		let mut right_listener = right.listener();

		left.try_send(11).expect("left route has capacity");
		assert_eq!(left_listener.read(), Some(11));
		assert_eq!(right_listener.read(), None);

		right.try_send(29).expect("right route has capacity");
		assert_eq!(right_listener.read(), Some(29));
		assert_eq!(left_listener.read(), None);
		assert_eq!(bus.topics().len(), 2);
	}

	#[test]
	fn diagnostic_topics_have_a_stable_scope_and_type_order() {
		let bus = MessageBus::new(test_config(4, 4, 8, 1)).expect("valid test bus");
		let first = bus.new_scope("first");
		let second = bus.new_scope("second");
		let _second_u64 = second.channel::<u64>();
		let _first_u64 = first.channel::<u64>();
		let _first_u32 = first.channel::<u32>();

		let topics = bus.topics();
		assert!(topics.windows(2).all(|pair| {
			pair[0].scope_id < pair[1].scope_id
				|| (pair[0].scope_id == pair[1].scope_id && pair[0].message_type <= pair[1].message_type)
		}));
	}

	#[test]
	fn one_full_topic_does_not_consume_another_topics_capacity() {
		let bus = MessageBus::new(test_config(2, 2, 8, 1)).expect("valid test bus");
		let messages = bus.new_scope("application");
		let congested = messages.channel::<u64>();
		let independent = messages.channel::<i64>();
		let _slow_listener = congested.listener();
		let mut independent_listener = independent.listener();

		congested.try_send(1).expect("first slot");
		congested.try_send(2).expect("second slot");
		match congested.try_send(3) {
			Err(TrySendError::Full(value)) => assert_eq!(value, 3),
			Err(error) => panic!("expected full topic, got {error:?}"),
			Ok(()) => panic!("a full topic accepted another value"),
		}

		independent.try_send(-7).expect("independent topic retains its own capacity");
		assert_eq!(independent_listener.read(), Some(-7));
	}

	#[test]
	fn dropping_the_slowest_listener_releases_topic_capacity() {
		let bus = MessageBus::new(test_config(1, 2, 8, 2)).expect("valid test bus");
		let channel = bus.new_scope("application").channel::<u64>();
		let mut current_listener = channel.listener();
		let stale_listener = channel.listener();

		channel.try_send(1).expect("first slot");
		channel.try_send(2).expect("second slot");
		assert_eq!(current_listener.to_vec(), [1, 2]);
		assert!(matches!(channel.try_send(3), Err(TrySendError::Full(3))));

		drop(stale_listener);
		channel.try_send(3).expect("dropping the stale cursor releases capacity");
		assert_eq!(current_listener.read(), Some(3));
	}

	#[test]
	fn blocking_send_counts_one_full_event_across_all_retries() {
		let bus = MessageBus::new(test_config(1, 1, 8, 1)).expect("valid test bus");
		let channel = bus.new_scope("application").channel::<u64>();
		let mut listener = channel.listener();
		channel.send(1);
		let publisher = channel;
		let blocked = std::thread::spawn(move || publisher.send(2));
		let deadline = Instant::now() + Duration::from_secs(1);

		while bus.topics()[0].full == 0 {
			assert!(
				Instant::now() < deadline,
				"the second publication did not observe the full route"
			);
			std::thread::yield_now();
		}

		assert_eq!(listener.read(), Some(1));
		blocked
			.join()
			.expect("blocked publisher completes after capacity is released");
		assert_eq!(listener.read(), Some(2));
		assert_eq!(bus.topics()[0].full, 1);
	}

	#[repr(align(16))]
	#[derive(Clone, Copy)]
	/// The `Overaligned` struct exercises route alignment validation.
	struct Overaligned;

	#[test]
	fn lazy_routes_report_payload_topic_and_listener_limits() {
		let bus = MessageBus::new(test_config(1, 2, 8, 1)).expect("valid test bus");
		let messages = bus.new_scope("application");

		match messages.try_channel::<[u8; 17]>() {
			Err(MessageRouteError::MessageTooLarge {
				message_bytes,
				available_bytes,
				..
			}) => assert_eq!((message_bytes, available_bytes), (17, 16)),
			Err(error) => panic!("expected oversized payload error, got {error}"),
			Ok(_) => panic!("an oversized payload acquired a topic"),
		}

		match messages.try_channel::<Overaligned>() {
			Err(MessageRouteError::MessageOveraligned {
				required_alignment,
				cell_alignment,
				..
			}) => assert_eq!((required_alignment, cell_alignment), (16, 8)),
			Err(error) => panic!("expected over-aligned payload error, got {error}"),
			Ok(_) => panic!("an over-aligned payload acquired a topic"),
		}

		let channel = messages.channel::<u64>();
		match messages.try_channel::<u32>() {
			Err(MessageRouteError::TopicLimit { max_topics, .. }) => assert_eq!(max_topics, 1),
			Err(error) => panic!("expected topic limit error, got {error}"),
			Ok(_) => panic!("a route was registered beyond the topic limit"),
		}

		let _listener = channel.listener();
		match channel.try_listener() {
			Err(MessageRouteError::ListenerLimit { max_listeners, .. }) => assert_eq!(max_listeners, 1),
			Err(error) => panic!("expected listener limit error, got {error}"),
			Ok(_) => panic!("a listener was registered beyond the route limit"),
		}
	}

	#[derive(Clone)]
	/// The `DropTracked` struct counts destruction of retained values during bus shutdown.
	struct DropTracked {
		drops: Arc<AtomicUsize>,
	}

	impl Drop for DropTracked {
		fn drop(&mut self) {
			self.drops.fetch_add(1, Ordering::Relaxed);
		}
	}

	#[test]
	fn unread_values_drop_when_the_last_listener_leaves() {
		let drops = Arc::new(AtomicUsize::new(0));
		let bus = MessageBus::new(test_config(1, 2, 8, 2)).expect("valid test bus");
		let messages = bus.new_scope("application");
		let channel = messages.channel::<DropTracked>();
		let first = channel.listener();
		let second = channel.listener();

		for _ in 0..2 {
			channel
				.try_send(DropTracked {
					drops: Arc::clone(&drops),
				})
				.expect("retained value fits");
		}

		drop(first);
		assert_eq!(drops.load(Ordering::Relaxed), 0, "the second listener still owns both values");
		drop(second);
		assert_eq!(drops.load(Ordering::Relaxed), 2);

		drop(channel);
		drop(messages);
		drop(bus);
		assert_eq!(drops.load(Ordering::Relaxed), 2);
	}

	/// The `CloneTracked` struct reports whether delivery cloned or moved its retained value.
	struct CloneTracked {
		value: u32,
		clones: Arc<AtomicUsize>,
		drops: Arc<AtomicUsize>,
	}

	impl Clone for CloneTracked {
		fn clone(&self) -> Self {
			self.clones.fetch_add(1, Ordering::Relaxed);
			Self {
				value: self.value,
				clones: Arc::clone(&self.clones),
				drops: Arc::clone(&self.drops),
			}
		}
	}

	impl Drop for CloneTracked {
		fn drop(&mut self) {
			self.drops.fetch_add(1, Ordering::Relaxed);
		}
	}

	#[test]
	fn a_single_listener_moves_drop_bearing_messages_without_cloning() {
		let clones = Arc::new(AtomicUsize::new(0));
		let drops = Arc::new(AtomicUsize::new(0));
		let bus = MessageBus::new(test_config(1, 2, 64, 1)).expect("valid test bus");
		let channel = bus.new_scope("application").channel::<CloneTracked>();
		let mut listener = channel.listener();
		channel
			.try_send(CloneTracked {
				value: 41,
				clones: Arc::clone(&clones),
				drops: Arc::clone(&drops),
			})
			.expect("message fits");

		let received = listener.read().expect("single listener receives message");
		assert_eq!(received.value, 41);
		assert_eq!(clones.load(Ordering::Relaxed), 0);
		drop(received);
		assert_eq!(drops.load(Ordering::Relaxed), 1);
	}

	#[test]
	fn multiple_listeners_clone_then_move_one_retained_message() {
		let clones = Arc::new(AtomicUsize::new(0));
		let drops = Arc::new(AtomicUsize::new(0));
		let bus = MessageBus::new(test_config(1, 2, 64, 2)).expect("valid test bus");
		let channel = bus.new_scope("application").channel::<CloneTracked>();
		let mut first = channel.listener();
		let mut second = channel.listener();
		channel
			.try_send(CloneTracked {
				value: 73,
				clones: Arc::clone(&clones),
				drops: Arc::clone(&drops),
			})
			.expect("message fits");

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
		clone_attempts: Arc<AtomicUsize>,
		drops: Arc<AtomicUsize>,
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
	fn a_panicking_clone_advances_its_cursor_without_wedging_capacity() {
		let clone_attempts = Arc::new(AtomicUsize::new(0));
		let drops = Arc::new(AtomicUsize::new(0));
		let bus = MessageBus::new(test_config(1, 1, 64, 2)).expect("valid test bus");
		let channel = bus.new_scope("application").channel::<PanicClone>();
		let mut panicking = channel.listener();
		let mut last = channel.listener();
		let make_message = |value| PanicClone {
			value,
			clone_attempts: Arc::clone(&clone_attempts),
			drops: Arc::clone(&drops),
		};
		channel.try_send(make_message(1)).expect("first message fits");

		let failure = catch_unwind(AssertUnwindSafe(|| panicking.read()));
		assert!(failure.is_err());
		assert_eq!(clone_attempts.load(Ordering::Relaxed), 1);
		let retained = last.read().expect("last listener moves the retained original");
		assert_eq!(retained.value, 1);
		drop(retained);

		channel
			.try_send(make_message(2))
			.expect("the advanced cursor releases the one-slot route");
		drop(panicking);
		drop(last);
		assert_eq!(drops.load(Ordering::Relaxed), 2);
	}

	#[test]
	fn incremental_slots_do_not_narrow_large_tickets() {
		let bus = MessageBus::new(test_config(1, 3, 8, 1)).expect("valid test bus");
		let channel = bus.new_scope("application").channel::<u64>();
		let route = &channel.topic.arena.routes[channel.topic.index];
		let ticket = u64::from(u32::MAX) + 1;
		{
			let mut writer = route.writer.lock();
			writer.next_ticket = ticket;
			writer.next_slot = 1;
			writer.cached_minimum = ticket;
		}
		let mut listener = channel.listener();

		channel.try_send(71).expect("the synthetic high ticket has capacity");

		assert_eq!(listener.read(), Some(71));
	}

	#[test]
	fn exhausted_sequence_is_a_stable_terminal_state() {
		let bus = MessageBus::new(test_config(1, 3, 8, 1)).expect("valid test bus");
		let channel = bus.new_scope("application").channel::<u64>();
		let route = &channel.topic.arena.routes[channel.topic.index];
		route.writer.lock().next_ticket = u64::MAX;
		let mut listener = channel.listener();

		assert_eq!(listener.read(), None);
		assert!(matches!(channel.try_send(71), Err(TrySendError::SequenceExhausted(71))));
	}

	#[test]
	fn exhausted_scope_ids_do_not_wrap_after_a_caught_panic() {
		let bus = MessageBus::new(test_config(1, 3, 8, 1)).expect("valid test bus");
		bus.inner.next_scope.store(u64::MAX, Ordering::Relaxed);

		let exhausted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| bus.new_scope("exhausted")));

		assert!(exhausted.is_err());
		assert_eq!(bus.inner.next_scope.load(Ordering::Relaxed), u64::MAX);
	}

	#[test]
	fn concurrent_producers_preserve_the_complete_broadcast_order() {
		const PRODUCERS: usize = 4;
		const MESSAGES_PER_PRODUCER: usize = 1_000;
		const TOTAL_MESSAGES: usize = PRODUCERS * MESSAGES_PER_PRODUCER;

		let bus = MessageBus::new(test_config(1, TOTAL_MESSAGES, 16, 2)).expect("valid test bus");
		let channel = bus.new_scope("application").channel::<(usize, usize)>();
		let mut first_listener = channel.listener();
		let mut second_listener = channel.listener();
		let start = Arc::new(Barrier::new(PRODUCERS));

		std::thread::scope(|threads| {
			for producer in 0..PRODUCERS {
				let channel = channel.clone();
				let start = Arc::clone(&start);
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
}
