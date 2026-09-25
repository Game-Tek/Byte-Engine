//! Arrival-ordered control records and which sink captured each one.
//!
//! The queue holds the records not every sink has pulled yet and retains the
//! last value of every control as records leave it. Retained values live in a
//! dense table with one cell per seat, device, and trigger, so reading a
//! control is an index computation rather than a hash. The queue applies the
//! capture rules on its own: a captured press stays with its sink through
//! platform repeats and the release, impulses never claim anything, and a
//! record captured by one sink is invisible to every other sink.
//! [`InputCollector`](super::InputCollector) validates values before pushing
//! them here, and [`InputSink`](super::InputSink) reads through
//! [`Queue::pending`] and [`Queue::visible`].

use std::alloc::Allocator;

use super::collector::SinkHandle;
use super::{DeviceHandle, SeatHandle, TriggerHandle, Value};

/// Identifies one physical control for a device and seat.
pub(super) type Source = (SeatHandle, DeviceHandle, TriggerHandle);

/// The `Record` struct keeps a control value and the sink that captured it together.
#[derive(Copy, Clone, Debug)]
pub(super) struct Record {
	pub(super) seat: SeatHandle,
	pub(super) device: DeviceHandle,
	pub(super) trigger: TriggerHandle,
	pub(super) value: Value,
	/// The increasing arrival number within the queue; zero marks an empty retained cell.
	pub(super) sequence: u64,
	/// An impulse: the record is a one-time sample, not the state of a held control.
	pub(super) transient: bool,
	owner: Option<SinkHandle>,
}

impl Record {
	const EMPTY: Self = Self {
		seat: SeatHandle(0),
		device: DeviceHandle(0),
		trigger: TriggerHandle(0),
		value: Value::Bool(false),
		sequence: 0,
		transient: false,
		owner: None,
	};

	pub(super) fn source(&self) -> Source {
		(self.seat, self.device, self.trigger)
	}

	/// Reports whether capturing this record keeps its control until release.
	fn claims(&self) -> bool {
		!self.transient && self.value == Value::Bool(true)
	}
}

/// The `Queue` struct provides arrival order and capture ownership for control records.
pub(super) struct Queue<A: Allocator + Clone> {
	records: Vec<Record, A>,
	/// The last record of every control, one cell per seat, device, and trigger.
	retained: Vec<Record, A>,
	seats: u32,
	devices: u32,
	triggers: u32,
	/// The sequence each sink has pulled through; records every sink passed are retained.
	cursors: Vec<u64, A>,
	sequence: u64,
}

impl<A: Allocator + Clone> Queue<A> {
	pub(super) fn new_in(allocator: A) -> Self {
		Self {
			records: Vec::with_capacity_in(64, allocator.clone()),
			retained: Vec::new_in(allocator.clone()),
			seats: 0,
			devices: 0,
			triggers: 0,
			cursors: Vec::new_in(allocator),
			sequence: 0,
		}
	}

	/// Registers a sink that starts reading at the next record.
	pub(super) fn add_sink(&mut self) -> SinkHandle {
		self.cursors.push(self.sequence);
		SinkHandle::new(self.cursors.len() as u32 - 1)
	}

	/// Sizes the retained table for the registered devices and triggers, keeping its records.
	pub(super) fn layout(&mut self, devices: u32, triggers: u32) {
		self.rebuild(self.seats, devices, triggers);
	}

	fn rebuild(&mut self, seats: u32, devices: u32, triggers: u32) {
		if (seats, devices, triggers) == (self.seats, self.devices, self.triggers) {
			return;
		}
		let cells = seats as usize * devices as usize * triggers as usize;
		let mut table = Vec::with_capacity_in(cells, self.retained.allocator().clone());
		table.resize(cells, Record::EMPTY);
		let previous = std::mem::replace(&mut self.retained, table);
		(self.seats, self.devices, self.triggers) = (seats, devices, triggers);
		for record in previous.iter().filter(|record| record.sequence != 0) {
			let cell = self.cell(&record.source());
			self.retained[cell] = *record;
		}
	}

	fn cell(&self, (seat, device, trigger): &Source) -> usize {
		((seat.0 as usize * self.devices as usize) + device.0 as usize) * self.triggers as usize + trigger.0 as usize
	}

	fn retained(&self, source: &Source) -> Option<&Record> {
		self.retained.get(self.cell(source)).filter(|record| record.sequence != 0)
	}

	/// Appends a record, inheriting the sink that captured the press it continues.
	pub(super) fn push(
		&mut self,
		seat: SeatHandle,
		device: DeviceHandle,
		trigger: TriggerHandle,
		value: Value,
		transient: bool,
	) {
		debug_assert!(
			device.0 < self.devices && trigger.0 < self.triggers,
			"Input record names an unknown device or trigger. The most likely cause is a handle from another collector."
		);
		if seat.0 >= self.seats {
			self.rebuild(seat.0 + 1, self.devices, self.triggers);
		}
		let source = (seat, device, trigger);
		// A release and platform repeats belong to the sink that took the press,
		// even when they arrive on a later tick. Impulses and analog values never claim.
		let owner = if transient || !matches!(value, Value::Bool(_)) {
			None
		} else {
			self.record_at(self.records.len(), &source)
				.filter(|record| record.claims())
				.and_then(|record| record.owner)
		};
		self.sequence += 1;
		self.records.push(Record {
			seat,
			device,
			trigger,
			value,
			sequence: self.sequence,
			transient,
			owner,
		});
	}

	/// Returns the index of the first record `sink` has not pulled yet.
	pub(super) fn unseen(&self, sink: SinkHandle) -> usize {
		let cursor = self.cursors[sink.index()];
		self.records.partition_point(|record| record.sequence <= cursor)
	}

	/// Marks every queued record as pulled by `sink` and retains the records every sink passed.
	///
	/// Only a captured press keeps its owner; every other record is released.
	pub(super) fn advance(&mut self, sink: SinkHandle) {
		self.cursors[sink.index()] = self.sequence;
		let floor = self.cursors.iter().copied().min().unwrap_or(self.sequence);
		let passed = self.records.partition_point(|record| record.sequence <= floor);
		for index in 0..passed {
			let mut record = self.records[index];
			if !record.claims() {
				record.owner = None;
			}
			let cell = self.cell(&record.source());
			self.retained[cell] = record;
		}
		self.records.drain(..passed);
	}

	pub(super) fn len(&self) -> usize {
		self.records.len()
	}

	pub(super) fn records(&self) -> &[Record] {
		&self.records
	}

	/// Returns the record at `index` when `sink` may act on it.
	///
	/// A record captured by another sink is hidden. A repeated `true` on a held
	/// control is a platform key repeat, not a new press, and is hidden from everyone.
	pub(super) fn pending(&self, sink: SinkHandle, index: usize) -> Option<Record> {
		let record = self.records[index];
		if record.owner.is_some_and(|owner| owner != sink) {
			return None;
		}
		if record.claims()
			&& self
				.record_at(index, &record.source())
				.is_some_and(|previous| previous.value == Value::Bool(true))
		{
			return None;
		}
		Some(record)
	}

	/// Gives the record at `index` to `sink`, including the rest of a captured press.
	pub(super) fn claim(&mut self, sink: SinkHandle, index: usize) {
		let record = self.records[index];
		debug_assert!(
			record.owner.is_none_or(|owner| owner == sink),
			"Input record is already captured. The most likely cause is a sink acting on a record it was not offered."
		);
		self.records[index].owner = Some(sink);
		if !record.claims() {
			return;
		}
		// Records already queued through the release take the same owner now;
		// records pushed later inherit it from the retained press.
		for next in &mut self.records[index + 1..] {
			if next.source() == record.source() {
				// An earlier sink may already hold a later record.
				next.owner.get_or_insert(sink);
				if next.value == Value::Bool(false) {
					break;
				}
			}
		}
	}

	/// Gives every record `sink` could still act on to that sink.
	pub(super) fn claim_pending(&mut self, sink: SinkHandle) {
		for index in 0..self.records.len() {
			if self.pending(sink, index).is_some() {
				self.claim(sink, index);
			}
		}
	}

	/// Returns a control's value as `sink` sees it at `position` in the queue.
	///
	/// Reading at a position instead of the queue end keeps a click's snapshot at
	/// the pointer position recorded before the click, not after it.
	pub(super) fn visible(&self, sink: SinkHandle, position: usize, source: &Source) -> Option<Record> {
		self.record_at(position, source)
			.filter(|record| record.owner.is_none_or(|owner| owner == sink))
			.copied()
	}

	/// Returns a control's latest record, queued or retained.
	pub(super) fn latest(&self, source: &Source) -> Option<Record> {
		self.record_at(self.records.len(), source).copied()
	}

	/// Discards queued and retained records for a seat, releasing any capture.
	pub(super) fn reset_seat(&mut self, seat: SeatHandle) {
		self.records.retain(|record| record.seat != seat);
		if seat.0 < self.seats {
			let block = self.devices as usize * self.triggers as usize;
			let start = seat.0 as usize * block;
			self.retained[start..start + block].fill(Record::EMPTY);
		}
	}

	/// Reads backwards so a snapshot only sees records up to its own position.
	fn record_at(&self, position: usize, source: &Source) -> Option<&Record> {
		self.records[..position]
			.iter()
			.rev()
			.find(|record| record.source() == *source)
			.or_else(|| self.retained(source))
	}
}
