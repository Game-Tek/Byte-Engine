//! How a run of work is split between lanes so that each share takes about as long.

use std::ops::Range;

/// The `WorkShares` struct splits a run of items between lanes so that every lane's share takes about as long.
///
/// Equal shares of the items are uneven work when some items cost more than others, and every lane then waits for
/// the slowest one. Use this for a stage that runs over about the same items every step: [`Self::range`] gives each
/// lane its share, and [`Self::record`] tells the next step how long each share took.
#[derive(Default)]
pub(crate) struct WorkShares {
	/// How many items each share of the last split had, and how long it took, in order.
	last: Vec<(usize, u64)>,
}

impl WorkShares {
	/// Returns the `share`th of `shares` consecutive ranges that cover `count` items.
	///
	/// The ranges are sized so that each would have taken as long in the last recorded split, as if the time of a
	/// share was spread evenly over its items. Without a record they are equal. Every lane gets the ranges of one
	/// split when all of them pass the same `shares` and `count` between two calls to [`Self::record`].
	pub(crate) fn range(&self, share: usize, shares: usize, count: usize) -> Range<usize> {
		let start = share.checked_sub(1).map_or(0, |previous| self.end(previous, shares, count));
		start..self.end(share, shares, count)
	}

	/// Replaces the record with how many items each share of a split had and how long it took, in share order.
	pub(crate) fn record(&mut self, shares: impl IntoIterator<Item = (usize, u64)>) {
		self.last.clear();
		self.last.extend(shares);
	}

	/// Returns where the `share`th of `shares` ranges of `count` items ends.
	fn end(&self, share: usize, shares: usize, count: usize) -> usize {
		if share + 1 >= shares {
			return count;
		}
		let (items, taken) = self
			.last
			.iter()
			.fold((0, 0), |(items, taken), share| (items + share.0, taken + share.1));
		if items == 0 || taken == 0 {
			return count * (share + 1) / shares;
		}
		// The share ends where the recorded shares up to there took its part of the whole time.
		let wanted = taken as f64 * (share + 1) as f64 / shares as f64;
		let (mut before, mut taken_before) = (0, 0);
		for (items_here, taken_here) in self.last.iter().copied() {
			if taken_here != 0 && (taken_before + taken_here) as f64 >= wanted {
				let within = items_here as f64 * (wanted - taken_before as f64) / taken_here as f64;
				// The number of items may have changed since the record, so positions scale with it.
				let end = (before as f64 + within) * count as f64 / items as f64;
				return (end.round() as usize).min(count);
			}
			before += items_here;
			taken_before += taken_here;
		}
		count
	}
}

#[cfg(test)]
mod tests {
	use super::WorkShares;

	fn ranges(shares: &WorkShares, lanes: usize, count: usize) -> Vec<std::ops::Range<usize>> {
		(0..lanes).map(|lane| shares.range(lane, lanes, count)).collect()
	}

	#[test]
	fn shares_are_equal_without_a_record() {
		let shares = WorkShares::default();

		assert_eq!(ranges(&shares, 4, 10), [0..2, 2..5, 5..7, 7..10]);
		assert_eq!(ranges(&shares, 1, 10), [0..10]);
		assert_eq!(ranges(&shares, 3, 0), [0..0, 0..0, 0..0]);
	}

	#[test]
	fn a_share_that_took_longer_gets_fewer_items() {
		let mut shares = WorkShares::default();
		// The first half took three times as long as the second, so half of the time is two thirds into it.
		shares.record([(100, 300), (100, 100)]);

		assert_eq!(ranges(&shares, 2, 200), [0..67, 67..200]);
	}

	#[test]
	fn shares_that_took_as_long_stay_as_they_are() {
		let mut shares = WorkShares::default();
		shares.record([(30, 500), (120, 500), (50, 500)]);

		assert_eq!(ranges(&shares, 3, 200), [0..30, 30..150, 150..200]);
	}

	#[test]
	fn shares_follow_a_changed_number_of_items_and_lanes() {
		let mut shares = WorkShares::default();
		shares.record([(50, 100), (50, 100)]);

		assert_eq!(ranges(&shares, 4, 200), [0..50, 50..100, 100..150, 150..200]);
	}

	#[test]
	fn shares_cover_every_item_once_whatever_the_record() {
		let mut shares = WorkShares::default();
		for record in [
			vec![(0, 40), (10, 0), (5, 900), (0, 0)],
			vec![(7, 1), (7, 1_000_000), (7, 1)],
			vec![(1_000, 3)],
		] {
			shares.record(record);
			for (lanes, count) in [(1, 9), (2, 9), (4, 9), (4, 1), (8, 1_000), (3, 0)] {
				let ranges = ranges(&shares, lanes, count);
				assert_eq!(ranges[0].start, 0);
				assert_eq!(ranges[lanes - 1].end, count);
				assert!(ranges.windows(2).all(|pair| pair[0].end == pair[1].start));
				assert!(ranges.iter().all(|range| range.start <= range.end));
			}
		}
	}
}
