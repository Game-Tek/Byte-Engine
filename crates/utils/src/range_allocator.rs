//! First-fit free-space tracking for preallocated arenas.

use std::ops::Range;

/// The `RangeAllocator` struct tracks which offsets of a fixed-size arena are free, so arena owners can lease
/// contiguous ranges without tracking free space themselves.
///
/// It only does the bookkeeping. The owner keeps the storage (a PCM buffer, a word arena, a mapped staging
/// buffer) and indexes it with the ranges this allocator hands out. Call [`Self::take`] to lease a range and
/// [`Self::give_back`] when the range is free again. Use [`Self::fits`] to decide whether the owner must
/// evict something before it takes a range.
#[derive(Debug, Clone)]
pub struct RangeAllocator {
	/// Free ranges, sorted by offset, never empty, and never adjacent to each other.
	free: Vec<Range<usize>>,
}

impl RangeAllocator {
	/// Creates an allocator whose whole `0..length` arena is free.
	///
	/// `free_list_capacity` preallocates room for that many free ranges. An owner on a real-time thread sets
	/// it to the most free ranges its lease count allows, then checks [`Self::free_range_count`] against that
	/// bound, so taking and returning ranges never allocates.
	pub fn new(length: usize, free_list_capacity: usize) -> Self {
		let mut free = Vec::with_capacity(free_list_capacity.max(1));
		if length > 0 {
			free.push(0..length);
		}
		Self { free }
	}

	/// Returns how many separate free ranges the arena has, which grows as leases fragment it.
	pub fn free_range_count(&self) -> usize {
		self.free.len()
	}

	/// Returns whether [`Self::take`] would succeed for `length` units at `alignment`.
	pub fn fits(&self, length: usize, alignment: usize) -> bool {
		self.first_fit(length, alignment).is_some()
	}

	/// Leases the lowest free range of `length` units whose start is a multiple of `alignment`.
	///
	/// `alignment` must be non-zero. Returns `None` when no free range is large enough; the owner can then
	/// evict leases and return their ranges with [`Self::give_back`].
	pub fn take(&mut self, length: usize, alignment: usize) -> Option<Range<usize>> {
		let (index, start) = self.first_fit(length, alignment)?;
		let taken = start..start + length;
		if taken.is_empty() {
			return Some(taken);
		}
		// Replace the free range with its unused prefix and suffix in place, so the list stays sorted.
		let free = self.free[index].clone();
		match (free.start < taken.start, taken.end < free.end) {
			(false, false) => {
				self.free.remove(index);
			}
			(true, false) => self.free[index].end = taken.start,
			(false, true) => self.free[index].start = taken.end,
			(true, true) => {
				self.free[index].end = taken.start;
				self.free.insert(index + 1, taken.end..free.end);
			}
		}
		Some(taken)
	}

	/// Returns a range obtained from [`Self::take`] and merges it with its free neighbours.
	pub fn give_back(&mut self, range: Range<usize>) {
		if range.is_empty() {
			return;
		}
		let index = self.free.partition_point(|free| free.start < range.start);
		debug_assert!(
			(index == 0 || self.free[index - 1].end <= range.start)
				&& self.free.get(index).is_none_or(|next| range.end <= next.start),
			"A returned range must not overlap free space. The most likely cause is a range returned twice."
		);
		let merges_left = index > 0 && self.free[index - 1].end == range.start;
		let merges_right = self.free.get(index).is_some_and(|next| next.start == range.end);
		match (merges_left, merges_right) {
			(true, true) => {
				self.free[index - 1].end = self.free[index].end;
				self.free.remove(index);
			}
			(true, false) => self.free[index - 1].end = range.end,
			(false, true) => self.free[index].start = range.start,
			(false, false) => self.free.insert(index, range),
		}
	}

	/// Finds the first free range that holds `length` units at `alignment`, and the aligned start inside it.
	fn first_fit(&self, length: usize, alignment: usize) -> Option<(usize, usize)> {
		self.free.iter().enumerate().find_map(|(index, free)| {
			let start = free.start.checked_next_multiple_of(alignment)?;
			(start.checked_add(length)? <= free.end).then_some((index, start))
		})
	}
}

#[cfg(test)]
mod tests {
	use super::RangeAllocator;

	#[test]
	fn returned_ranges_coalesce_after_fragmentation() {
		let mut allocator = RangeAllocator::new(6, 4);
		let first = allocator.take(2, 1).expect("first range");
		let second = allocator.take(2, 1).expect("second range");
		let third = allocator.take(2, 1).expect("third range");

		assert!(!allocator.fits(1, 1));

		allocator.give_back(first);
		allocator.give_back(third);

		assert!(allocator.fits(2, 1));
		assert!(!allocator.fits(4, 1));

		allocator.give_back(second);

		assert_eq!(allocator.take(6, 1), Some(0..6));
	}

	#[test]
	fn exact_fits_keep_later_returns_coalescing() {
		let mut allocator = RangeAllocator::new(8, 4);
		let ranges: Vec<_> = (0..4).map(|_| allocator.take(2, 1).expect("range")).collect();
		allocator.give_back(ranges[0].clone());
		allocator.give_back(ranges[2].clone());

		// An exact fit consumes the first free range; the remaining free range must stay mergeable.
		assert_eq!(allocator.take(2, 1), Some(0..2));
		allocator.give_back(ranges[3].clone());
		allocator.give_back(ranges[1].clone());
		allocator.give_back(0..2);

		assert_eq!(allocator.take(8, 1), Some(0..8));
	}

	#[test]
	fn aligned_takes_leave_prefix_and_suffix_free() {
		let mut allocator = RangeAllocator::new(64, 4);
		assert_eq!(allocator.take(24, 16), Some(0..24));
		assert_eq!(allocator.take(24, 16), Some(32..56));

		// The padding between the two leases stays free for unaligned requests.
		assert_eq!(allocator.take(8, 1), Some(24..32));
		assert_eq!(allocator.take(8, 1), Some(56..64));
		assert_eq!(allocator.take(1, 1), None);
	}
}
