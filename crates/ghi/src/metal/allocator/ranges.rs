//! Bookkeeping for the free bytes of one fixed-size block of memory.
//!
//! This module does not know what the memory is or how a resource gets into it. [`super::Allocator`] keeps one
//! [`FreeRanges`] for each Metal heap and creates resources at the offsets it returns.

use std::cmp::Ordering;
use std::ops::Range;

/// The `FreeRanges` struct lets an allocator choose where each allocation goes inside one block of memory, and reuse
/// the bytes of allocations that ended.
///
/// Take bytes with [`Self::allocate`] and return them with [`Self::free`] once nothing uses them.
pub(crate) struct FreeRanges {
	/// The free byte ranges, in address order. Two ranges never touch: [`Self::free`] merges them.
	ranges: Vec<Range<u64>>,
	size: u64,
}

impl FreeRanges {
	/// Creates the bookkeeping for a block of `size` bytes that are all free.
	pub(crate) fn new(size: u64) -> Self {
		Self {
			ranges: std::iter::once(0..size).collect(),
			size,
		}
	}

	/// Takes `size` bytes at a multiple of `alignment` and returns their offset, or `None` when no free range fits
	/// them.
	///
	/// The lowest range that fits is used, which keeps allocations together at the start of the block. Next, pass
	/// the offset and the same `size` to [`Self::free`] when the allocation ends.
	pub(crate) fn allocate(&mut self, size: u64, alignment: u64) -> Option<u64> {
		let alignment = alignment.max(1);
		let (index, offset) = self.ranges.iter().enumerate().find_map(|(index, range)| {
			let offset = range.start.checked_next_multiple_of(alignment)?;
			(offset.checked_add(size)? <= range.end).then_some((index, offset))
		})?;

		let Range { start, end } = self.ranges[index].clone();
		let allocation_end = offset + size;
		// The bytes alignment skipped and the bytes after the allocation stay free.
		match (start < offset, allocation_end < end) {
			(true, true) => {
				self.ranges[index] = start..offset;
				self.ranges.insert(index + 1, allocation_end..end);
			}
			(true, false) => self.ranges[index] = start..offset,
			(false, true) => self.ranges[index] = allocation_end..end,
			(false, false) => {
				self.ranges.remove(index);
			}
		}

		Some(offset)
	}

	/// Returns the `size` bytes at `offset` that [`Self::allocate`] handed out, so later allocations can use them.
	///
	/// # Panics
	///
	/// Panics when some of the bytes are already free or lie outside the block.
	pub(crate) fn free(&mut self, offset: u64, size: u64) {
		let end = offset + size;
		// The freed bytes belong between the free range before them and the one after them.
		let index = self.ranges.partition_point(|range| range.start < offset);
		let touches_previous = index.checked_sub(1).map(|previous| self.ranges[previous].end.cmp(&offset));
		let touches_next = self.ranges.get(index).map(|next| end.cmp(&next.start));
		assert!(
			end <= self.size && touches_previous != Some(Ordering::Greater) && touches_next != Some(Ordering::Greater),
			"Freed bytes are not allocated. The most likely cause is that an allocation was freed twice, or freed with a different size than it was allocated with.",
		);

		match (
			touches_previous == Some(Ordering::Equal),
			touches_next == Some(Ordering::Equal),
		) {
			(true, true) => {
				self.ranges[index - 1].end = self.ranges[index].end;
				self.ranges.remove(index);
			}
			(true, false) => self.ranges[index - 1].end = end,
			(false, true) => self.ranges[index].start = offset,
			(false, false) => self.ranges.insert(index, offset..end),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::FreeRanges;

	#[test]
	fn allocations_fill_the_block_from_its_start_until_nothing_fits() {
		let mut ranges = FreeRanges::new(100);

		assert_eq!(ranges.allocate(40, 1), Some(0));
		assert_eq!(ranges.allocate(40, 1), Some(40));
		assert_eq!(ranges.allocate(40, 1), None);
		assert_eq!(ranges.allocate(20, 1), Some(80));
		assert_eq!(ranges.allocate(1, 1), None);
	}

	#[test]
	fn allocations_start_at_their_alignment_and_leave_the_skipped_bytes_free() {
		let mut ranges = FreeRanges::new(256);

		assert_eq!(ranges.allocate(10, 1), Some(0));
		assert_eq!(ranges.allocate(64, 64), Some(64));
		// The bytes between the two allocations still hold a small allocation.
		assert_eq!(ranges.allocate(54, 2), Some(10));
		assert_eq!(ranges.allocate(128, 128), Some(128));
		assert_eq!(ranges.allocate(1, 1), None);
	}

	#[test]
	fn freed_bytes_are_handed_out_again() {
		let mut ranges = FreeRanges::new(90);
		let offsets = [(); 3].map(|_| ranges.allocate(30, 1).unwrap());

		ranges.free(offsets[1], 30);

		assert_eq!(ranges.allocate(40, 1), None);
		assert_eq!(ranges.allocate(30, 1), Some(offsets[1]));
	}

	#[test]
	fn freed_neighbors_merge_into_room_for_a_larger_allocation() {
		let mut ranges = FreeRanges::new(120);
		let offsets = [(); 4].map(|_| ranges.allocate(30, 1).unwrap());

		// Free the middle two in the order that merges with a later range, then with an earlier one.
		ranges.free(offsets[2], 30);
		ranges.free(offsets[1], 30);
		assert_eq!(ranges.allocate(60, 1), Some(offsets[1]));

		// Free everything, ending with the range that joins both of its neighbors.
		ranges.free(offsets[0], 30);
		ranges.free(offsets[3], 30);
		ranges.free(offsets[1], 60);
		assert_eq!(ranges.allocate(120, 1), Some(0));
	}

	#[test]
	#[should_panic(expected = "Freed bytes are not allocated")]
	fn freeing_bytes_twice_is_rejected() {
		let mut ranges = FreeRanges::new(64);
		let offset = ranges.allocate(16, 1).unwrap();

		ranges.free(offset, 16);
		ranges.free(offset, 16);
	}
}
