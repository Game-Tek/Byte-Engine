//! Collect physical input once, then process each [`ActionProcessor`](super::ActionProcessor)
//! in priority order. Consumption stays on each record, including the release
//! that ends a claimed press. Call [`InputEvents::end_tick`] after the last layer.

use std::alloc::{Allocator, Global};

use hashbrown::HashMap;
use log::warn;
use utils::hash::GxBuildHasher;

use super::action::{InputValue, TriggerMapping};
use super::device::{Device, DeviceClass, DeviceClassHandle};
use super::trigger::{Trigger, TriggerDescription, TriggerReference, TriggerRegistry};
use super::{ActionBindingDescription, DeviceHandle, SeatHandle, TriggerHandle, Types, Value};

/// The `SourceEvent` struct provides a serializable control sample for remote input replay.
///
/// Read pending samples with [`InputEvents::source_events`] and serialize them
/// with Facet. Handles identify the sender's registry: map the seat, device, and
/// trigger to the receiver's registered handles before calling [`InputEvents::record`].
/// Preserve arrival order; sequence numbers let the transport detect duplicates
/// or gaps but do not provide timestamps or identify a connection.
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

/// Identifies one physical control for a device and seat.
type Source = (SeatHandle, DeviceHandle, TriggerHandle);

/// The `Record` struct keeps a control value and its consuming layer together.
///
/// The queue retains arrival order; the state table retains the last record
/// between ticks. Only a persistent press carries ownership into the next tick.
#[derive(Copy, Clone)]
pub(super) struct Record {
	pub(super) seat_handle: SeatHandle,
	pub(super) device_handle: DeviceHandle,
	pub(super) trigger_handle: TriggerHandle,
	pub(super) value: Value,
	pub(super) sequence: u64,
	consumer: Option<ConsumerHandle>,
}

impl Record {
	fn source(&self) -> Source {
		(self.seat_handle, self.device_handle, self.trigger_handle)
	}
}

/// The `ConsumerHandle` struct identifies one input layer reading the event queue.
///
/// Take one per layer from [`InputEvents::add_consumer`] and give it to that
/// layer's [`ActionProcessor`](super::ActionProcessor). Processing order decides priority.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ConsumerHandle(u32);

/// The `InputEvents` struct provides shared physical input for application layers.
///
/// Register controls through [`TriggerRegistry`], create devices with
/// [`Self::create_device`], and queue their values with [`Self::record`]. After
/// processing every layer, call [`Self::end_tick`] to retain control state.
pub struct InputEvents<A: Allocator + Clone = Global> {
	device_classes: Vec<DeviceClass<A>, A>,
	triggers: Vec<Trigger<A>, A>,
	devices: Vec<Device, A>,
	pub(super) records: Vec<Record, A>,
	/// Each source keeps its value and any press ownership from the previous tick.
	sources: HashMap<Source, Record, GxBuildHasher, A>,
	consumers: u32,
	sequence: u64,
}

impl<A: Allocator + Clone + Default> Default for InputEvents<A> {
	fn default() -> Self {
		Self::new_in(A::default())
	}
}

impl InputEvents {
	/// Creates an empty queue. Next, register controls through [`TriggerRegistry`].
	pub fn new() -> Self {
		Self::new_in(Global)
	}
}

impl<A: Allocator + Clone> InputEvents<A> {
	/// Creates an input queue whose buffers and registered names use `allocator`.
	///
	/// Retain the allocator for the queue's lifetime; [`Self::end_tick`] reuses its
	/// storage and preserves held controls. Next, register controls through
	/// [`TriggerRegistry`] and create a device with [`Self::create_device`].
	///
	/// ```
	/// use byte_engine::{
	///     core::channel::DefaultChannel,
	///     input::{ActionBindingDescription, ActionProcessor, Consumption, InputEvents,
	///         SeatHandle, TickPolicy, TriggerReference, Types, Value, utils},
	/// };
	///
	/// let arena = bumpalo::Bump::new();
	/// let mut events = InputEvents::new_in(&arena);
	/// let keyboard = utils::register_keyboard_device_class(&mut events);
	/// let device = events.create_device(&keyboard);
	/// let mut layer = ActionProcessor::new_in(events.add_consumer(), DefaultChannel::new(), &arena);
	/// let action = layer.create_action(&events, Types::Boolean,
	///     &[ActionBindingDescription::new("Keyboard.W")], TickPolicy::WhileActive);
	/// events.record(SeatHandle::stub(), device, TriggerReference::Name("Keyboard.W"), Value::Bool(true));
	/// layer.process(&mut events, |_| Consumption::Consumed);
	/// events.end_tick();
	/// assert_eq!(layer.action_state(SeatHandle::stub(), action, device), Value::Bool(true));
	/// ```
	pub fn new_in(allocator: A) -> Self {
		Self {
			device_classes: Vec::new_in(allocator.clone()),
			triggers: Vec::new_in(allocator.clone()),
			devices: Vec::new_in(allocator.clone()),
			records: Vec::with_capacity_in(64, allocator.clone()),
			sources: HashMap::with_capacity_and_hasher_in(512, GxBuildHasher::default(), allocator),
			consumers: 0,
			sequence: 0,
		}
	}

	/// Registers one layer. Pass its handle to [`ActionProcessor::new`](super::ActionProcessor::new).
	pub fn add_consumer(&mut self) -> ConsumerHandle {
		let handle = ConsumerHandle(self.consumers);
		self.consumers += 1;
		handle
	}

	/// Queues one control value, preserving arrival order and any existing press owner.
	///
	/// Unknown triggers and values of the wrong [`Types`] are dropped with a warning.
	/// Next, process the queue through each layer's [`ActionProcessor`](super::ActionProcessor).
	pub fn record(&mut self, seat: SeatHandle, device: DeviceHandle, reference: TriggerReference, value: Value) {
		let Some((trigger_handle, trigger)) = self.resolve_trigger(&reference) else {
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

		// A release and platform repeats inherit the press owner, even when they
		// arrive on a later tick. Impulses and analog controls never claim input.
		let consumer = if trigger.r#type == Types::Boolean && !trigger.transient {
			self.record_at(self.records.len(), &(seat, device, trigger_handle))
				.filter(|record| record.value == Value::Bool(true))
				.and_then(|record| record.consumer)
		} else {
			None
		};
		self.sequence += 1;
		self.records.push(Record {
			seat_handle: seat,
			device_handle: device,
			trigger_handle,
			value,
			sequence: self.sequence,
			consumer,
		});
	}

	/// Iterates over this tick's accepted source events in arrival order without allocating.
	///
	/// Serialize these samples before [`Self::end_tick`] (or
	/// [`InputManager::update`](super::InputManager::update)) clears the queue.
	/// Consumption by local layers does not remove samples from this stream.
	pub fn source_events(&self) -> impl ExactSizeIterator<Item = SourceEvent> + '_ {
		self.records.iter().map(|record| SourceEvent {
			seat_handle: record.seat_handle,
			device_handle: record.device_handle,
			trigger_handle: record.trigger_handle,
			value: record.value,
			sequence: record.sequence,
		})
	}

	/// Retains the last value of each control and clears this tick's queue.
	///
	/// Call this after every layer processed. A press keeps its owner until release.
	pub fn end_tick(&mut self) {
		for mut record in self.records.drain(..) {
			if record.value != Value::Bool(true) || self.triggers[record.trigger_handle.0 as usize].transient {
				record.consumer = None;
			}
			self.sources.insert(record.source(), record);
		}
	}

	/// Returns a control's latest physical value, or its default before the first record.
	pub fn value(&self, seat: SeatHandle, device: DeviceHandle, reference: TriggerReference) -> Result<Value, ()> {
		let (trigger_handle, trigger) = self.resolve_trigger(&reference).ok_or(())?;
		Ok(self
			.record_at(self.records.len(), &(seat, device, trigger_handle))
			.map_or(trigger.default, |record| record.value))
	}

	/// Returns a record this layer may handle, excluding platform key repeats.
	pub(super) fn pending(&self, consumer: ConsumerHandle, index: usize) -> Option<Record> {
		let record = self.records[index];
		if record.consumer.is_some_and(|owner| owner != consumer) {
			return None;
		}
		if record.value == Value::Bool(true)
			&& !self.is_transient(record.trigger_handle)
			&& self
				.record_at(index, &record.source())
				.is_some_and(|previous| previous.value == Value::Bool(true))
		{
			return None;
		}
		Some(record)
	}

	/// Assigns a record to this layer, including the remainder of a consumed press.
	pub(super) fn consume(&mut self, consumer: ConsumerHandle, index: usize) {
		let record = self.records[index];
		debug_assert!(
			record.consumer.is_none_or(|owner| owner == consumer),
			"Input record is already consumed. The layer must process only its pending records."
		);
		self.records[index].consumer = Some(consumer);
		if record.value != Value::Bool(true) || self.is_transient(record.trigger_handle) {
			return;
		}
		// Mark records already queued through the release. Future records inherit
		// the same owner in `record`, so ownership needs no separate claim table.
		for next in &mut self.records[index + 1..] {
			if next.source() == record.source() {
				// An earlier layer may already have consumed a later record.
				next.consumer.get_or_insert(consumer);
				if next.value == Value::Bool(false) {
					break;
				}
			}
		}
	}

	/// Borrows a control at one queue position when this layer may read it.
	pub(super) fn visible_record(&self, consumer: ConsumerHandle, position: usize, source: &Source) -> Option<&Record> {
		self.record_at(position, source)
			.filter(|record| record.consumer.is_none_or(|owner| owner == consumer))
	}

	/// Reads backwards so snapshots only see records up to their own queue position.
	fn record_at(&self, position: usize, source: &Source) -> Option<&Record> {
		self.records[..position]
			.iter()
			.rev()
			.find(|record| record.source() == *source)
			.or_else(|| self.sources.get(source))
	}

	pub(super) fn is_transient(&self, trigger: TriggerHandle) -> bool {
		self.triggers[trigger.0 as usize].transient
	}

	/// Creates one concrete device of a registered class.
	///
	/// Call this once for each physical or virtual device, such as each connected
	/// gamepad.
	pub fn create_device(&mut self, device_class: &DeviceClassHandle) -> DeviceHandle {
		debug_assert!(
			(device_class.0 as usize) < self.device_classes.len(),
			"Device class is unknown. The most likely cause is using a handle from another input pipeline."
		);
		debug_assert!(
			self.devices.len() < u32::MAX as usize,
			"Device handle space is exhausted. The most likely cause is creating devices without retiring old state."
		);

		let device = Device {
			device_class_handle: *device_class,
		};

		let handle = DeviceHandle(self.devices.len() as u32);
		self.devices.push(device);
		handle
	}

	/// Returns every device that belongs to the named class.
	pub fn devices_by_class_name(&self, class_name: &str) -> Option<impl Iterator<Item = DeviceHandle> + '_> {
		let class = self.class_by_name(class_name)?;

		Some(
			self.devices
				.iter()
				.enumerate()
				.filter_map(move |(index, device)| (device.device_class_handle == class).then_some(DeviceHandle(index as u32))),
		)
	}

	/// Reports whether a device class declares a control under `name`.
	pub fn has_trigger(&self, reference: &TriggerReference) -> bool {
		self.resolve_trigger(reference).is_some()
	}

	/// Resolves a registered trigger by handle or its `DeviceClass.Trigger` name.
	pub(super) fn resolve_trigger(&self, reference: &TriggerReference) -> Option<(TriggerHandle, &Trigger<A>)> {
		let handle = match reference {
			TriggerReference::Handle(handle) => *handle,
			TriggerReference::Name(name) => {
				let (class_name, trigger_name) = name.split_once('.')?;
				let class = self.class_by_name(class_name)?;
				let index = self
					.triggers
					.iter()
					.position(|trigger| trigger.device_class_handle == class && trigger.name.as_ref() == trigger_name)?;
				TriggerHandle(index as u32)
			}
		};

		self.triggers.get(handle.0 as usize).map(|trigger| (handle, trigger))
	}

	/// Resolves one action binding's value source and its optional snapshot trigger.
	///
	/// Returns `None` for an unknown name, or for a snapshot trigger that is not a
	/// boolean control of the value source's own device class.
	pub(super) fn resolve_binding(&self, binding: &ActionBindingDescription) -> Option<TriggerMapping> {
		let (trigger_handle, source) = self.resolve_trigger(&binding.input_source)?;
		let trigger = if let Some(reference) = &binding.trigger {
			let (handle, gate) = self.resolve_trigger(reference)?;
			if gate.r#type != Types::Boolean
				|| gate.device_class_handle != source.device_class_handle
				|| (binding.trigger_mode == super::TriggerMode::Drag && gate.transient)
			{
				warn!(
					"Input binding is invalid. The trigger must be boolean, share the value source's device class, and be retained for drags."
				);
				return None;
			}
			Some(handle)
		} else {
			None
		};

		Some(TriggerMapping {
			input_source: trigger_handle,
			trigger,
			trigger_mode: binding.trigger_mode,
			mapping: binding.mapping,
		})
	}

	/// Finds a registered class by its application-facing name.
	fn class_by_name(&self, name: &str) -> Option<DeviceClassHandle> {
		self.device_classes
			.iter()
			.position(|class| class.name.as_ref() == name)
			.map(|index| DeviceClassHandle(index as u32))
	}
}

impl<A: Allocator + Clone> TriggerRegistry for InputEvents<A> {
	fn register_device_class(&mut self, name: &str) -> DeviceClassHandle {
		debug_assert!(
			self.device_classes.len() < u32::MAX as usize,
			"Device-class handle space is exhausted. The most likely cause is registering classes continuously instead of reusing them."
		);

		let device_class = DeviceClass {
			name: Box::clone_from_ref_in(name, self.device_classes.allocator().clone()),
		};

		let handle = DeviceClassHandle(self.device_classes.len() as u32);
		self.device_classes.push(device_class);
		handle
	}

	fn register_trigger<T: InputValue + Into<Value>>(
		&mut self,
		device_class: &DeviceClassHandle,
		name: &str,
		description: TriggerDescription<T>,
	) -> TriggerHandle {
		let default: Value = description.default.into();
		let default_value_type: Types = default.into();

		assert_eq!(
			default_value_type,
			T::get_type(),
			"Default value type does not match input source type"
		);
		debug_assert!(
			(device_class.0 as usize) < self.device_classes.len(),
			"Trigger device class is unknown. The most likely cause is using a handle from another input pipeline."
		);
		debug_assert!(
			self.triggers.len() < u32::MAX as usize,
			"Trigger handle space is exhausted. The most likely cause is registering triggers continuously instead of reusing them."
		);

		let trigger = Trigger {
			device_class_handle: *device_class,
			name: Box::clone_from_ref_in(name, self.triggers.allocator().clone()),
			r#type: T::get_type(),
			default,
			transient: description.transient,
		};

		let handle = TriggerHandle(self.triggers.len() as u32);
		self.triggers.push(trigger);
		handle
	}
}
