use std::cmp::Ordering;
use std::time::SystemTime;

use super::{DeviceHandle, SeatHandle, TriggerHandle, Value};

/// The `Record` struct stores one timestamped trigger value until the input manager resolves it.
#[derive(Copy, Clone, PartialEq)]
pub(super) struct Record {
	pub(super) seat_handle: SeatHandle,
	pub(super) device_handle: DeviceHandle,
	pub(super) trigger_handle: TriggerHandle,
	pub(super) value: Value,
	pub(super) time: SystemTime,
}

impl PartialOrd for Record {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		self.time.partial_cmp(&other.time)
	}
}

pub(super) fn compare_source_then_time(left: &Record, right: &Record) -> Ordering {
	left.seat_handle
		.0
		.cmp(&right.seat_handle.0)
		.then(left.device_handle.0.cmp(&right.device_handle.0))
		.then(left.trigger_handle.0.cmp(&right.trigger_handle.0))
		.then(left.time.cmp(&right.time))
}

fn same_source(left: &Record, right: &Record) -> bool {
	left.seat_handle == right.seat_handle
		&& left.device_handle == right.device_handle
		&& left.trigger_handle == right.trigger_handle
}

/// Keeps only the most recent record for each source in a source-sorted slice.
pub(super) fn compact_latest_by_source(records: &mut [Record]) -> usize {
	if records.is_empty() {
		return 0;
	}

	let mut write_index = 0;
	let mut read_index = 0;

	while read_index < records.len() {
		let mut latest = records[read_index];
		read_index += 1;

		// Sorting by source and then time makes the final record in each run the current value.
		while read_index < records.len() && same_source(&latest, &records[read_index]) {
			latest = records[read_index];
			read_index += 1;
		}

		records[write_index] = latest;
		write_index += 1;
	}

	write_index
}
