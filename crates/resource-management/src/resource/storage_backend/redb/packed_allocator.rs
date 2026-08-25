/// The `PackedRange` struct identifies one exact byte range in the shared resource pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PackedRange {
	pub(super) offset: u64,
	pub(super) size: u64,
}

impl PackedRange {
	pub(super) fn new(offset: u64, size: u64) -> Self {
		Self { offset, size }
	}

	fn end(self) -> Option<u64> {
		self.offset.checked_add(self.size)
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AllocationStatus {
	Reserved,
	Published,
}

#[derive(Debug)]
/// The `PackedAllocation` struct tracks the lifecycle and mapped readers of one occupied slot.
struct PackedAllocation {
	size: u64,
	status: AllocationStatus,
	lease: Option<Arc<()>>,
}

/// The `PackedAllocatorState` struct keeps metadata publication and mapped-reader leasing consistent.
pub(super) struct PackedAllocatorState {
	high_water: u64,
	allocations: BTreeMap<u64, PackedAllocation>,
	retired: Vec<(PackedRange, Arc<()>)>,
	free_slots: BTreeMap<u64, u64>,
	fragmentation_warning_emitted: bool,
}

impl PackedAllocatorState {
	/// Marks a completed reservation as live and retires the range replaced by the metadata transaction.
	pub(super) fn publish(&mut self, reservation: PackedRange, replaced: Option<PackedRange>) {
		if reservation.size != 0 {
			let allocation = self
				.allocations
				.get_mut(&reservation.offset)
				.expect("packed reservation should remain allocated until publication");
			assert_eq!(allocation.size, reservation.size);
			assert_eq!(allocation.status, AllocationStatus::Reserved);
			allocation.status = AllocationStatus::Published;
		}

		if replaced.is_some_and(|replaced| replaced != reservation) {
			self.retire(replaced.unwrap());
		}
	}

	/// Retires a published range and recycles it after its last mapped reader drops.
	pub(super) fn retire(&mut self, range: PackedRange) {
		if range.size == 0 {
			return;
		}

		let Some(allocation) = self.allocations.get(&range.offset) else {
			log::error!(
				"Packed resource range could not be retired. The most likely cause is inconsistent in-memory allocation state."
			);
			return;
		};
		if allocation.size != range.size || allocation.status != AllocationStatus::Published {
			log::error!(
				"Packed resource range could not be retired. The most likely cause is inconsistent resource metadata or overlapping publication."
			);
			return;
		}

		let allocation = self.allocations.remove(&range.offset).unwrap();
		match allocation.lease {
			Some(lease) if Arc::strong_count(&lease) > 1 => self.retired.push((range, lease)),
			_ => self.add_free_slot(range),
		}
	}

	/// Keeps one published range unavailable while a mapped reader owns it.
	pub(super) fn lease(&mut self, range: PackedRange) -> Result<Option<Arc<()>>, ()> {
		if range.size == 0 {
			return Ok(None);
		}

		let allocation = self.allocations.get_mut(&range.offset).ok_or(())?;
		if allocation.size != range.size || allocation.status != AllocationStatus::Published {
			return Err(());
		}
		let lease = allocation.lease.get_or_insert_with(|| Arc::new(()));
		Ok(Some(Arc::clone(lease)))
	}

	/// Returns one reserved range using best fit before extending the file.
	fn reserve(&mut self, size: u64, pack_path: &Path) -> Result<PackedRange, String> {
		self.reclaim_retired();
		if size == 0 {
			return Ok(PackedRange::new(self.high_water, 0));
		}

		let best_fit = self
			.free_slots
			.iter()
			.filter(|(_, available)| **available >= size)
			.map(|(offset, available)| (*offset, *available))
			.min_by_key(|(offset, available)| (available - size, *offset));

		let range = if let Some((offset, available)) = best_fit {
			self.free_slots.remove(&offset);
			if available > size {
				self.free_slots.insert(offset + size, available - size);
			}
			PackedRange::new(offset, size)
		} else {
			let offset = self.high_water;
			let end = offset.checked_add(size).ok_or_else(|| {
				"Packed resource reservation is too large. The most likely cause is a payload size that exceeds the platform file limit."
					.to_string()
			})?;
			let file = std::fs::OpenOptions::new().write(true).open(pack_path).map_err(|error| {
				format!(
					"Packed resource file could not be grown: {error}. The most likely cause is that '{}' is not writable.",
					pack_path.display()
				)
			})?;
			file.set_len(end).map_err(|error| {
				format!(
					"Packed resource file could not be grown: {error}. The most likely cause is insufficient disk space or an inaccessible destination."
				)
			})?;
			self.high_water = end;
			PackedRange::new(offset, size)
		};

		let previous = self.allocations.insert(
			range.offset,
			PackedAllocation {
				size,
				status: AllocationStatus::Reserved,
				lease: None,
			},
		);
		assert!(previous.is_none(), "packed reservations must not overlap");
		Ok(range)
	}

	fn abort(&mut self, range: PackedRange) {
		// Only unpublished ranges can be returned without checking reader leases.
		if range.size == 0 {
			return;
		}

		let Some(allocation) = self.allocations.get(&range.offset) else {
			return;
		};
		if allocation.size != range.size || allocation.status != AllocationStatus::Reserved {
			return;
		}

		self.allocations.remove(&range.offset);
		self.add_free_slot(range);
	}

	/// Returns retired ranges after their last mapped reader releases its lease.
	fn reclaim_retired(&mut self) {
		let mut index = 0;
		while index < self.retired.len() {
			if Arc::strong_count(&self.retired[index].1) == 1 {
				let (range, _) = self.retired.swap_remove(index);
				self.add_free_slot(range);
			} else {
				index += 1;
			}
		}
	}

	/// Coalesces one released range with its address-adjacent free slots.
	fn add_free_slot(&mut self, range: PackedRange) {
		let mut offset = range.offset;
		let mut size = range.size;

		if let Some((&previous_offset, &previous_size)) = self.free_slots.range(..offset).next_back() {
			if previous_offset.checked_add(previous_size) == Some(offset) {
				self.free_slots.remove(&previous_offset);
				offset = previous_offset;
				size += previous_size;
			}
		}

		if let Some((&next_offset, &next_size)) = self.free_slots.range(offset..).next() {
			if offset.checked_add(size) == Some(next_offset) {
				self.free_slots.remove(&next_offset);
				size += next_size;
			}
		}

		self.free_slots.insert(offset, size);
	}

	fn fragmentation(&self) -> PackedFragmentation {
		// A large total is useful only when at least one slot can satisfy a
		// similarly large replacement, so retain both total and largest values.
		let free_bytes = self.free_slots.values().copied().sum();
		let largest_free_slot = self.free_slots.values().copied().max().unwrap_or(0);
		PackedFragmentation {
			file_size: self.high_water,
			free_bytes,
			largest_free_slot,
			free_slot_count: self.free_slots.len(),
		}
	}
}

/// The `PackedResourceAllocator` struct reuses copy-on-write payload slots in one resource pack.
pub(super) struct PackedResourceAllocator {
	pack_path: PathBuf,
	state: Mutex<PackedAllocatorState>,
}

impl PackedResourceAllocator {
	/// Opens a writable pack and reconstructs reusable gaps from the live ranges recorded in redb.
	pub(super) fn open(pack_path: PathBuf, live_ranges: Vec<PackedRange>) -> Result<Self, String> {
		let file = std::fs::OpenOptions::new()
			.create(true)
			.read(true)
			.write(true)
			.truncate(false)
			.open(&pack_path)
			.map_err(|error| {
				format!(
					"Packed resource file could not be opened: {error}. The most likely cause is that the resource directory is not writable."
				)
			})?;
		let high_water = file.metadata().map_err(|error| {
			format!(
				"Packed resource file size could not be read: {error}. The most likely cause is an inaccessible or incomplete resource file."
			)
		})?.len();
		let state = reconstruct_state(&pack_path, high_water, live_ranges)?;
		let allocator = Self {
			pack_path,
			state: Mutex::new(state),
		};
		allocator.warn_if_fragmented();
		Ok(allocator)
	}

	/// Reserves the smallest reusable slot that fits, or grows the pack once no slot fits.
	pub(super) fn reserve(&self, size: u64) -> Result<PackedRange, String> {
		let range = self.lock_state().reserve(size, &self.pack_path)?;
		self.warn_if_fragmented();
		Ok(range)
	}

	/// Returns an unpublished reservation immediately after cancellation or failure.
	pub(super) fn abort(&self, range: PackedRange) {
		self.lock_state().abort(range);
		self.warn_if_fragmented();
	}

	pub(super) fn lock_state(&self) -> MutexGuard<'_, PackedAllocatorState> {
		self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
	}

	/// Logs one actionable warning after scattered free slots cross the fragmentation threshold.
	pub(super) fn warn_if_fragmented(&self) {
		let mut state = self.lock_state();
		if state.fragmentation_warning_emitted {
			return;
		}

		let fragmentation = state.fragmentation();
		if !fragmentation.is_excessive() {
			return;
		}
		state.fragmentation_warning_emitted = true;
		drop(state);

		let resources_path = self.pack_path.parent().unwrap_or_else(|| Path::new("."));
		log::warn!(
			"Resource pack is fragmented: {}% of '{}' is free across {} reusable slots, but the largest slot is only {} bytes. The most likely cause is repeated packed-resource rebuilds with changing payload sizes. Delete '{}' and bake again to compact it. See {}.",
			fragmentation.free_percent(),
			self.pack_path.display(),
			fragmentation.free_slot_count,
			fragmentation.largest_free_slot,
			resources_path.display(),
			crate::online_docs_url(BAKING_APP_RESOURCES_DOCS_PATH)
		);
	}

	#[cfg(test)]
	pub(super) fn high_water(&self) -> u64 {
		self.lock_state().high_water
	}
}

/// Reconstructs occupied and free slots while validating every database-visible range.
fn reconstruct_state(pack_path: &Path, high_water: u64, live_ranges: Vec<PackedRange>) -> Result<PackedAllocatorState, String> {
	let mut allocations: BTreeMap<u64, PackedAllocation> = BTreeMap::new();
	for range in live_ranges {
		if range.size == 0 {
			continue;
		}
		let end = range.end().ok_or_else(|| invalid_layout_message(pack_path))?;
		if end > high_water {
			return Err(invalid_layout_message(pack_path));
		}

		if let Some((&previous_offset, previous)) = allocations.range(..=range.offset).next_back() {
			if previous_offset + previous.size > range.offset {
				return Err(invalid_layout_message(pack_path));
			}
		}
		if let Some((&next_offset, _)) = allocations.range(range.offset..).next() {
			if end > next_offset {
				return Err(invalid_layout_message(pack_path));
			}
		}

		allocations.insert(
			range.offset,
			PackedAllocation {
				size: range.size,
				status: AllocationStatus::Published,
				lease: None,
			},
		);
	}

	let mut free_slots = BTreeMap::new();
	let mut cursor = 0;
	for (&offset, allocation) in &allocations {
		if cursor < offset {
			free_slots.insert(cursor, offset - cursor);
		}
		cursor = offset + allocation.size;
	}
	if cursor < high_water {
		free_slots.insert(cursor, high_water - cursor);
	}

	Ok(PackedAllocatorState {
		high_water,
		allocations,
		retired: Vec::new(),
		free_slots,
		fragmentation_warning_emitted: false,
	})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The `PackedFragmentation` struct measures whether free space is too scattered for effective reuse.
struct PackedFragmentation {
	file_size: u64,
	free_bytes: u64,
	largest_free_slot: u64,
	free_slot_count: usize,
}

impl PackedFragmentation {
	/// Requires both substantial free space and substantial external fragmentation before warning.
	fn is_excessive(self) -> bool {
		self.file_size >= FRAGMENTATION_MIN_FILE_SIZE
			&& self.free_slot_count >= FRAGMENTATION_MIN_FREE_SLOTS
			&& u128::from(self.free_bytes) * 4 >= u128::from(self.file_size)
			&& u128::from(self.largest_free_slot) * 2 < u128::from(self.free_bytes)
	}

	fn free_percent(self) -> u64 {
		if self.file_size == 0 {
			0
		} else {
			(u128::from(self.free_bytes) * 100 / u128::from(self.file_size)) as u64
		}
	}
}

fn invalid_layout_message(pack_path: &Path) -> String {
	format!(
		"Packed resource layout is invalid. The most likely cause is an incomplete or corrupt resource database or pack at '{}'. Delete the resource directory and bake again. See {}.",
		pack_path.display(),
		crate::online_docs_url(BAKING_APP_RESOURCES_DOCS_PATH)
	)
}

use super::BAKING_APP_RESOURCES_DOCS_PATH;

const FRAGMENTATION_MIN_FILE_SIZE: u64 = 64 * 1024 * 1024;
const FRAGMENTATION_MIN_FREE_SLOTS: usize = 32;

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use super::{
		AllocationStatus, FRAGMENTATION_MIN_FILE_SIZE, FRAGMENTATION_MIN_FREE_SLOTS, PackedAllocation, PackedAllocatorState,
		PackedFragmentation, PackedRange,
	};

	#[test]
	fn best_fit_prefers_the_smallest_sufficient_gap() {
		let mut state = PackedAllocatorState {
			high_water: 40,
			allocations: BTreeMap::from([(
				16,
				PackedAllocation {
					size: 16,
					status: AllocationStatus::Published,
					lease: None,
				},
			)]),
			retired: Vec::new(),
			free_slots: BTreeMap::from([(0, 16), (32, 8)]),
			fragmentation_warning_emitted: false,
		};

		let range = state.reserve(7, std::path::Path::new("unused")).unwrap();

		assert_eq!(range, PackedRange::new(32, 7));
		assert_eq!(state.free_slots, BTreeMap::from([(0, 16), (39, 1)]));
	}

	#[test]
	fn released_slots_coalesce_on_both_sides() {
		let mut state = PackedAllocatorState {
			high_water: 24,
			allocations: BTreeMap::new(),
			retired: Vec::new(),
			free_slots: BTreeMap::from([(0, 8), (16, 8)]),
			fragmentation_warning_emitted: false,
		};

		state.add_free_slot(PackedRange::new(8, 8));

		assert_eq!(state.free_slots, BTreeMap::from([(0, 24)]));
	}

	#[test]
	fn fragmentation_requires_many_scattered_slots_and_substantial_free_space() {
		let excessive = PackedFragmentation {
			file_size: FRAGMENTATION_MIN_FILE_SIZE,
			free_bytes: FRAGMENTATION_MIN_FILE_SIZE / 2,
			largest_free_slot: FRAGMENTATION_MIN_FILE_SIZE / 16,
			free_slot_count: FRAGMENTATION_MIN_FREE_SLOTS,
		};
		let one_useful_slot = PackedFragmentation {
			largest_free_slot: excessive.free_bytes,
			free_slot_count: 1,
			..excessive
		};
		let small_pack = PackedFragmentation {
			file_size: FRAGMENTATION_MIN_FILE_SIZE - 1,
			..excessive
		};

		assert!(excessive.is_excessive());
		assert!(!one_useful_slot.is_excessive());
		assert!(!small_pack.is_excessive());
	}
}

use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
	sync::{Arc, Mutex, MutexGuard},
};
