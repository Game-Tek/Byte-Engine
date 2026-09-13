//! Step one: every source event ends up in one [`InputCollector`].
//!
//! Window buttons, keys, pointer motion, gamepad controls, and replayed
//! [`SourceEvent`] samples are all recorded here as control values. Sinks then
//! pull from the collector in priority order; each record is dropped once
//! every sink has pulled past it.

use std::alloc::{Allocator, Global};
use std::num::NonZeroU32;

use log::warn;

use super::device::DeviceClassHandle;
use super::gesture::TriggerMapping;
use super::queue::{Queue, Record};
use super::registry::Registry;
use super::trigger::{TriggerDescription, TriggerReference, TriggerRegistry};
use super::{ActionBindingDescription, DeviceHandle, InputValue, SeatHandle, TriggerHandle, Value};

/// The `SinkHandle` struct identifies one sink pulling from a collector.
///
/// Take one per sink from [`InputCollector::add_sink`] and give it to that
/// sink's [`InputSink`](super::InputSink). The order sinks pull in decides
/// their priority.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SinkHandle(NonZeroU32);

impl SinkHandle {
	/// Wraps a sink's index; the niche keeps a record's optional owner at four bytes.
	pub(super) fn new(index: u32) -> Self {
		Self(NonZeroU32::new(index + 1).expect("Sink handle space is exhausted."))
	}

	pub(super) fn index(self) -> usize {
		self.0.get() as usize - 1
	}
}

/// The `SourceEvent` struct provides a serializable control sample for remote input replay.
///
/// Read pending samples with [`InputCollector::source_events`] and serialize
/// them with Facet. Handles identify the sender's registry: map the seat,
/// device, and trigger to the receiver's registered handles before calling
/// [`InputCollector::record`]. Preserve arrival order; sequence numbers let the
/// transport detect duplicates or gaps but do not provide timestamps.
#[derive(Copy, Clone, Debug, PartialEq, facet::Facet)]
pub struct SourceEvent {
	/// The sender's player seat.
	pub seat_handle: SeatHandle,
	/// The sender's concrete input device.
	pub device_handle: DeviceHandle,
	/// The sender's registered control.
	pub trigger_handle: TriggerHandle,
	/// The recorded control value.
	pub value: Value,
	/// The increasing arrival sequence within the sender's input queue.
	pub sequence: u64,
}

/// The `InputCollector` struct is where every source event ends up before sinks act on it.
///
/// Register controls through [`TriggerRegistry`], create devices with
/// [`Self::create_device`], and queue their values with [`Self::record`]. Take
/// one [`SinkHandle`] per sink with [`Self::add_sink`]. Each tick, let the sinks
/// pull in priority order. Every sink must keep pulling: records stay queued
/// until the last sink has pulled past them.
pub struct InputCollector<A: Allocator + Clone = Global> {
	registry: Registry<A>,
	queue: Queue<A>,
}

impl<A: Allocator + Clone + Default> Default for InputCollector<A> {
	fn default() -> Self {
		Self::new_in(A::default())
	}
}

impl InputCollector {
	/// Creates an empty collector. Next, register controls through [`TriggerRegistry`].
	pub fn new() -> Self {
		Self::new_in(Global)
	}
}

impl<A: Allocator + Clone> InputCollector<A> {
	/// Creates a collector whose buffers and registered names use `allocator`.
	///
	/// Retain the allocator for the collector's lifetime; the queue reuses its
	/// storage and preserves held controls. Next, register controls through
	/// [`TriggerRegistry`] and create a device with [`Self::create_device`].
	///
	/// ```
	/// use byte_engine::{
	///     core::channel::DefaultChannel,
	///     input::{ActionBindingDescription, Capture, InputCollector, InputSink,
	///         SeatHandle, TickPolicy, TriggerReference, Types, Value, utils},
	/// };
	///
	/// let arena = bumpalo::Bump::new();
	/// let mut collector = InputCollector::new_in(&arena);
	/// let keyboard = utils::register_keyboard_device_class(&mut collector);
	/// let device = collector.create_device(&keyboard);
	/// let mut sink = InputSink::new_in(collector.add_sink(), DefaultChannel::new(), &arena);
	/// let action = sink.create_action(&collector, Types::Boolean,
	///     &[ActionBindingDescription::new("Keyboard.W")], TickPolicy::WhileActive);
	/// collector.record(SeatHandle::stub(), device, TriggerReference::Name("Keyboard.W"), Value::Bool(true));
	/// sink.pull(&mut collector, |_| Capture::Captured);
	/// assert_eq!(sink.action_state(SeatHandle::stub(), action, device), Value::Bool(true));
	/// ```
	pub fn new_in(allocator: A) -> Self {
		Self {
			registry: Registry::new_in(allocator.clone()),
			queue: Queue::new_in(allocator),
		}
	}

	/// Registers one sink. Pass its handle to [`InputSink::new`](super::InputSink::new).
	pub fn add_sink(&mut self) -> SinkHandle {
		self.queue.add_sink()
	}

	/// Queues one control value in arrival order.
	///
	/// Unknown triggers and values of the wrong [`Types`](super::Types) are
	/// dropped with a warning. Next, let each [`InputSink`](super::InputSink) pull.
	pub fn record(&mut self, seat: SeatHandle, device: DeviceHandle, reference: TriggerReference, value: Value) {
		let Some((trigger_handle, trigger)) = self.registry.resolve(&reference) else {
			warn!("Input trigger is unknown. The most likely cause is an unregistered trigger name or handle.");
			return;
		};
		if trigger.r#type != value.into() {
			warn!(
				"Input value type is incorrect. The trigger {} requires {:?}.",
				trigger.name, trigger.r#type
			);
			return;
		}
		if device.0 >= self.registry.device_count() {
			warn!("Input device is unknown. The most likely cause is a device handle from another collector.");
			return;
		}
		self.queue.push(seat, device, trigger_handle, value, trigger.transient);
	}

	/// Iterates over the queued source events in arrival order without allocating.
	///
	/// A sample stays queued until every sink has pulled past it, so skip the
	/// sequences already serialized. Capture by local sinks does not remove
	/// samples from this stream.
	pub fn source_events(&self) -> impl ExactSizeIterator<Item = SourceEvent> + '_ {
		self.queue.records().iter().map(|record| SourceEvent {
			seat_handle: record.seat,
			device_handle: record.device,
			trigger_handle: record.trigger,
			value: record.value,
			sequence: record.sequence,
		})
	}

	/// Discards queued and held input for a seat that lost focus.
	///
	/// Resetting releases every capture even when the platform missed a button
	/// release. Also call each sink's [`InputSink::cancel_seat`](super::InputSink::cancel_seat)
	/// so its interactions end as cancellations.
	pub fn reset_seat(&mut self, seat: SeatHandle) {
		self.queue.reset_seat(seat);
	}

	/// Returns a control's latest value, or its default before the first record.
	pub fn value(&self, seat: SeatHandle, device: DeviceHandle, reference: TriggerReference) -> Result<Value, ()> {
		let (trigger_handle, trigger) = self.registry.resolve(&reference).ok_or(())?;
		Ok(self
			.queue
			.latest(&(seat, device, trigger_handle))
			.map_or(trigger.default, |record| record.value))
	}

	/// Creates one concrete device of a registered class.
	///
	/// Call this once for each physical or virtual device, such as each connected gamepad.
	pub fn create_device(&mut self, device_class: &DeviceClassHandle) -> DeviceHandle {
		let device = self.registry.create_device(device_class);
		self.queue.layout(self.registry.device_count(), self.registry.trigger_count());
		device
	}

	/// Returns every device that belongs to the named class.
	pub fn devices_by_class_name(&self, class_name: &str) -> Option<impl Iterator<Item = DeviceHandle> + '_> {
		self.registry.devices_by_class_name(class_name)
	}

	/// Resolves a control's handle, for recording without a name lookup each time.
	pub fn trigger(&self, reference: TriggerReference) -> Option<TriggerHandle> {
		self.registry.resolve(&reference).map(|(handle, _)| handle)
	}

	/// Resolves one binding's names into handles for a sink's action.
	pub(super) fn resolve_binding(&self, binding: &ActionBindingDescription) -> Option<TriggerMapping> {
		self.registry.resolve_binding(binding)
	}

	/// Exposes the queue to a pulling sink.
	pub(super) fn queue_mut(&mut self) -> &mut Queue<A> {
		&mut self.queue
	}

	/// Returns a control's latest record, for sinks that read state outside a pull.
	pub(super) fn latest(&self, seat: SeatHandle, device: DeviceHandle, trigger: TriggerHandle) -> Option<Record> {
		self.queue.latest(&(seat, device, trigger))
	}
}

impl<A: Allocator + Clone> TriggerRegistry for InputCollector<A> {
	fn register_device_class(&mut self, name: &str) -> DeviceClassHandle {
		self.registry.register_device_class(name)
	}

	fn register_trigger<T: InputValue + Into<Value>>(
		&mut self,
		device_class: &DeviceClassHandle,
		name: &str,
		description: TriggerDescription<T>,
	) -> TriggerHandle {
		let trigger = self.registry.register_trigger(device_class, name, description);
		self.queue.layout(self.registry.device_count(), self.registry.trigger_count());
		trigger
	}
}
