//! Bookkeeping of free and used element ranges in one fixed-size GPU stream.
//!
//! [`RangeAllocator`] only decides where things go. Whoever takes a range writes the buffers it names.

use std::ops::Range;

/// The `RangeAllocator` struct shares one stream's fixed capacity between owners that come and go.
///
/// The visibility pipeline uses it for the resident vertex streams: meshes take their vertices when they upload, and
/// posed skinned instances take theirs while they animate and give them back when they're removed. Call
/// [`Self::allocate`] for a range, and [`Self::free`] once nothing reads it anymore.
#[derive(Debug)]
pub(crate) struct RangeAllocator {
	/// Free ranges sorted by start, never empty, never touching, so neighbors are always merged.
	free: Vec<Range<u32>>,
}

impl RangeAllocator {
	/// Creates an allocator whose whole `capacity` is free.
	pub(crate) fn new(capacity: u32) -> Self {
		Self {
			free: if capacity == 0 { Vec::new() } else { vec![0..capacity] },
		}
	}

	/// Takes the first free range that holds `length` elements, or returns `None` when no free range is long enough.
	///
	/// A zero length takes nothing and returns an empty range.
	pub(crate) fn allocate(&mut self, length: u32) -> Option<Range<u32>> {
		if length == 0 {
			return Some(0..0);
		}
		let index = self.free.iter().position(|range| range.len() as u64 >= length as u64)?;
		let range = &mut self.free[index];
		let start = range.start;
		range.start += length;
		if range.start == range.end {
			self.free.remove(index);
		}
		Some(start..start + length)
	}

	/// Gives `range` back, merging it with the free ranges it touches.
	///
	/// `range` must be one [`Self::allocate`] returned and not freed since.
	pub(crate) fn free(&mut self, range: Range<u32>) {
		if range.start == range.end {
			return;
		}
		let index = self.free.partition_point(|free| free.start < range.start);
		debug_assert!(
			index == 0 || self.free[index - 1].end <= range.start,
			"Freed range overlaps a free range. The most likely cause is that the range was freed twice."
		);
		debug_assert!(
			self.free.get(index).is_none_or(|next| range.end <= next.start),
			"Freed range overlaps a free range. The most likely cause is that the range was freed twice."
		);
		let joins_previous = index > 0 && self.free[index - 1].end == range.start;
		let joins_next = self.free.get(index).is_some_and(|next| next.start == range.end);
		match (joins_previous, joins_next) {
			(true, true) => {
				self.free[index - 1].end = self.free[index].end;
				self.free.remove(index);
			}
			(true, false) => self.free[index - 1].end = range.end,
			(false, true) => self.free[index].start = range.start,
			(false, false) => self.free.insert(index, range),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn allocations_are_disjoint_until_capacity_runs_out() {
		let mut ranges = RangeAllocator::new(10);
		assert_eq!(ranges.allocate(4), Some(0..4));
		assert_eq!(ranges.allocate(6), Some(4..10));
		assert_eq!(ranges.allocate(1), None);
	}

	#[test]
	fn a_freed_range_is_reused() {
		let mut ranges = RangeAllocator::new(10);
		let first = ranges.allocate(4).unwrap();
		ranges.allocate(6).unwrap();
		ranges.free(first);
		assert_eq!(ranges.allocate(4), Some(0..4));
	}

	#[test]
	fn neighboring_free_ranges_merge_into_one_that_holds_both() {
		let mut ranges = RangeAllocator::new(9);
		let [a, b, c] = [3, 3, 3].map(|length| ranges.allocate(length).unwrap());
		ranges.free(a);
		ranges.free(c);
		// Two free ranges of three don't hold six.
		assert_eq!(ranges.allocate(6), None);
		ranges.free(b);
		assert_eq!(ranges.allocate(9), Some(0..9));
	}

	#[test]
	fn a_request_too_long_for_any_free_range_takes_nothing() {
		let mut ranges = RangeAllocator::new(8);
		let first = ranges.allocate(2).unwrap();
		ranges.allocate(2).unwrap();
		ranges.free(first);
		assert_eq!(ranges.allocate(5), None);
		// The failed request left both free ranges as they were.
		assert_eq!(ranges.allocate(2), Some(0..2));
		assert_eq!(ranges.allocate(4), Some(4..8));
	}
}
