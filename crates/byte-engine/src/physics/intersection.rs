//! The broad phase finds the world bounds that overlap, so the narrow phase only tests those pairs.

use std::{
	ops::Range,
	simd::{cmp::SimdPartialOrd as _, f32x4},
};

use math::AABB;

use crate::physics::dynabit::contact::Pair;

/// The number of bounds one sweep step tests at once.
const WIDTH: usize = 4;

/// The `SweepBounds` struct keeps a step's world bounds sorted along one axis, so the pairs that overlap are found by
/// scanning each box's neighbors instead of testing every pair.
///
/// Call [`Self::rebuild`] with the step's bounds, then [`Self::overlaps`] to read the overlapping pairs. To spread
/// the scan over lanes, give each lane its own range of `0..`[`Self::count`].
#[derive(Default)]
pub struct SweepBounds {
	/// The identifier each sorted box was staged with.
	ids: Vec<u32>,
	/// The smallest corner of each sorted box as one array per axis. Index 0 is the sweep axis.
	///
	/// Every array ends in `WIDTH` entries that overlap nothing, so a scan can read whole groups past the last box.
	min: [Vec<f32>; 3],
	/// The largest corner of each sorted box, laid out like `min`.
	max: [Vec<f32>; 3],
	/// The boxes as they were received, kept between rebuilds so staging them does not allocate.
	staged: Vec<StagedBounds>,
	/// The order the last rebuild sorted `staged` into. Boxes move little between steps, so the next rebuild
	/// usually only has to repair it.
	order: Vec<SweepKey>,
	/// The axis the last rebuild sorted along.
	axis: usize,
}

/// The `StagedBounds` struct holds one box between being received and being sorted along the sweep axis.
struct StagedBounds {
	id: u32,
	min: [f32; 3],
	max: [f32; 3],
}

/// The `SweepKey` struct places one staged box in the sweep order.
#[derive(Clone, Copy)]
struct SweepKey {
	/// Where the box starts on the sweep axis.
	start: f32,
	id: u32,
	/// The box's index in the staged boxes.
	staged: u32,
}

impl SweepKey {
	/// Orders keys by where their boxes start. Identifiers break ties, so equal boxes come out in the same order
	/// whatever order they were staged in and whatever order an earlier rebuild left.
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.start.total_cmp(&other.start).then_with(|| self.id.cmp(&other.id))
	}
}

impl SweepBounds {
	/// Returns the number of boxes [`Self::overlaps`] can scan.
	pub fn count(&self) -> usize {
		self.ids.len()
	}

	/// Replaces the stored boxes with `bounds`, each identified by the distinct `usize` it comes with.
	///
	/// The boxes are sorted along the axis on which the fewest of them share a stretch, which keeps each box's scan
	/// in [`Self::overlaps`] short. When `bounds` yields the boxes of the last rebuild again, in the same order,
	/// the sort starts from the order they had then, which is quick when they have moved little.
	pub fn rebuild(&mut self, bounds: impl Iterator<Item = (usize, AABB)>) {
		// Staging over the last rebuild's boxes shows whether these are the same boxes.
		let mut count = 0;
		let mut same_boxes = true;
		for (id, bounds) in bounds {
			debug_assert!(
				u32::try_from(id).is_ok(),
				"Broad-phase identifier is too large. The most likely cause is a body index above the 32-bit slot range."
			);
			let staged = StagedBounds {
				id: id as u32,
				min: bounds.min().to_array(),
				max: bounds.max().to_array(),
			};
			match self.staged.get_mut(count) {
				Some(previous) => {
					same_boxes &= previous.id == staged.id;
					*previous = staged;
				}
				None => {
					same_boxes = false;
					self.staged.push(staged);
				}
			}
			count += 1;
		}
		same_boxes &= count == self.staged.len();
		self.staged.truncate(count);

		let sweep_axis = least_crowded_axis(&self.staged);
		let axes = [sweep_axis, (sweep_axis + 1) % 3, (sweep_axis + 2) % 3];

		// The last order only helps for the same boxes along the same axis.
		let repairable = same_boxes && sweep_axis == self.axis;
		if repairable {
			for key in &mut self.order {
				key.start = self.staged[key.staged as usize].min[sweep_axis];
			}
		} else {
			self.order.clear();
			self.order
				.extend(self.staged.iter().enumerate().map(|(index, staged)| SweepKey {
					start: staged.min[sweep_axis],
					id: staged.id,
					staged: index as u32,
				}));
		}
		if !(repairable && repair(&mut self.order)) {
			self.order.sort_unstable_by(SweepKey::cmp);
		}
		self.axis = sweep_axis;

		self.ids.clear();
		self.ids.extend(self.order.iter().map(|key| key.id));
		let staged = |key: &SweepKey| &self.staged[key.staged as usize];
		for (slot, axis) in axes.into_iter().enumerate() {
			// A comparison with NaN is never true, so the padding overlaps nothing, not even unbounded boxes.
			let padding = std::iter::repeat_n(f32::NAN, WIDTH);
			self.min[slot].clear();
			self.min[slot].extend(self.order.iter().map(|key| staged(key).min[axis]).chain(padding.clone()));
			self.max[slot].clear();
			self.max[slot].extend(self.order.iter().map(|key| staged(key).max[axis]).chain(padding));
		}
	}

	/// Calls `found` with every pair of overlapping boxes whose first box, in sweep order, lies in `range`.
	///
	/// Boxes that only touch count as overlapping. Ranges that together cover `0..`[`Self::count`] report every pair
	/// exactly once, and reading them in ascending order gives the same sequence as one scan of the whole range.
	pub fn overlaps(&self, range: Range<usize>, mut found: impl FnMut(Pair)) {
		let count = self.ids.len();
		let [min_sweep, min_second, min_third] = &self.min;
		let [max_sweep, max_second, max_third] = &self.max;
		for first in range {
			let reach = f32x4::splat(max_sweep[first]);
			let (low_second, high_second) = (f32x4::splat(min_second[first]), f32x4::splat(max_second[first]));
			let (low_third, high_third) = (f32x4::splat(min_third[first]), f32x4::splat(max_third[first]));
			let mut next = first + 1;
			while next < count {
				let group = next..next + WIDTH;
				let within_reach = f32x4::from_slice(&min_sweep[group.clone()]).simd_le(reach);
				let overlapping = within_reach
					& f32x4::from_slice(&min_second[group.clone()]).simd_le(high_second)
					& f32x4::from_slice(&max_second[group.clone()]).simd_ge(low_second)
					& f32x4::from_slice(&min_third[group.clone()]).simd_le(high_third)
					& f32x4::from_slice(&max_third[group]).simd_ge(low_third);
				let mut lanes = overlapping.to_bitmask();
				while lanes != 0 {
					let lane = lanes.trailing_zeros() as usize;
					found(Pair::new(self.ids[first] as usize, self.ids[next + lane] as usize));
					lanes &= lanes - 1;
				}
				// Boxes are sorted by where they start on the sweep axis, so the first one that starts past this
				// box's end is followed only by boxes that do too.
				if !within_reach.all() {
					break;
				}
				next += WIDTH;
			}
		}
	}
}

/// Sorts `keys` that are already nearly in order and returns whether it finished.
///
/// It moves each key back past the keys that belong after it, which is quick while keys are close to their places.
/// It gives up, leaving the keys in some other order, once that has taken more moves than a full sort is worth.
fn repair(keys: &mut [SweepKey]) -> bool {
	/// The moves allowed for each key on average before a full sort is the better choice.
	const MOVES_PER_KEY: usize = 8;
	let mut moves_left = keys.len() * MOVES_PER_KEY;
	for sorted in 1..keys.len() {
		let key = keys[sorted];
		let mut place = sorted;
		while place > 0 && key.cmp(&keys[place - 1]).is_lt() {
			keys[place] = keys[place - 1];
			place -= 1;
		}
		keys[place] = key;
		let moves = sorted - place;
		if moves > moves_left {
			return false;
		}
		moves_left -= moves;
	}
	true
}

/// Returns the axis on which the fewest boxes overlap each other, judged from an evenly spaced sample of `boxes`.
///
/// Counting real overlaps, instead of measuring how far the boxes spread, keeps a few distant boxes from picking an
/// axis that the rest are stacked along. Ties go to the lower axis.
fn least_crowded_axis(boxes: &[StagedBounds]) -> usize {
	/// Enough boxes to tell a crowded axis from a sparse one, and few enough to compare every pair of them.
	const SAMPLES: usize = 64;
	let samples = boxes.len().min(SAMPLES);
	let sample = |index: usize| &boxes[index * boxes.len() / samples];
	let mut overlaps = [0u32; 3];
	for first in 0..samples {
		for second in first + 1..samples {
			let (first, second) = (sample(first), sample(second));
			for axis in 0..3 {
				overlaps[axis] += u32::from(first.min[axis] <= second.max[axis] && second.min[axis] <= first.max[axis]);
			}
		}
	}
	let mut least = 0;
	for axis in 1..3 {
		if overlaps[axis] < overlaps[least] {
			least = axis;
		}
	}
	least
}

#[cfg(test)]
mod tests {
	use math::{Point, Vector};

	use super::*;

	/// Returns a box from its center and half extents.
	fn bounds(center: [f32; 3], half_extents: [f32; 3]) -> AABB {
		AABB::from_center_and_half_extents(Point::from_array(center), Vector::from_array(half_extents))
	}

	/// Returns the pairs a sweep of `boxes` reports, each ordered by identifier and sorted.
	fn swept_pairs(boxes: &[AABB]) -> Vec<(usize, usize)> {
		let mut sweep = SweepBounds::default();
		sweep.rebuild(boxes.iter().copied().enumerate());
		let mut pairs = Vec::new();
		sweep.overlaps(0..sweep.count(), |pair| pairs.push((pair.a.min(pair.b), pair.a.max(pair.b))));
		pairs.sort_unstable();
		pairs
	}

	/// Returns boxes scattered through a volume by a fixed integer hash, densely enough that many overlap.
	fn scattered_boxes(count: usize) -> Vec<AABB> {
		let unit = |seed: u32| (seed.wrapping_mul(0x9E37_79B9) >> 8) as f32 / (1u32 << 24) as f32;
		(0..count as u32)
			.map(|index| {
				let center = [unit(index * 3) * 12.0, unit(index * 3 + 1) * 4.0, unit(index * 3 + 2) * 9.0];
				let half_extents = [0.2 + unit(index + 500), 0.2 + unit(index + 900), 0.2 + unit(index + 1300)];
				bounds(center, half_extents)
			})
			.collect()
	}

	#[test]
	fn boxes_that_overlap_on_every_axis_are_paired() {
		let boxes = [
			bounds([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
			// Overlaps the first box on every axis.
			bounds([1.5, 0.5, -0.5], [1.0, 1.0, 1.0]),
			// Overlaps the first box on two axes only.
			bounds([0.0, 5.0, 0.0], [1.0, 1.0, 1.0]),
			// Lies inside the first box.
			bounds([0.2, 0.2, 0.2], [0.1, 0.1, 0.1]),
		];

		assert_eq!(swept_pairs(&boxes), [(0, 1), (0, 3)]);
	}

	#[test]
	fn boxes_that_only_touch_are_paired() {
		for axis in 0..3 {
			let mut center = [0.0; 3];
			center[axis] = 2.0;
			let boxes = [bounds([0.0; 3], [1.0; 3]), bounds(center, [1.0; 3])];

			assert_eq!(swept_pairs(&boxes), [(0, 1)]);
		}
	}

	#[test]
	fn sweep_reports_the_pairs_an_exhaustive_test_finds() {
		// More boxes than one group holds, in an order unrelated to their positions.
		let boxes = scattered_boxes(67);
		let mut expected = Vec::new();
		for first in 0..boxes.len() {
			for second in first + 1..boxes.len() {
				let (a, b) = (&boxes[first], &boxes[second]);
				let (a_min, a_max) = (a.min().to_array(), a.max().to_array());
				let (b_min, b_max) = (b.min().to_array(), b.max().to_array());
				if (0..3).all(|axis| a_min[axis] <= b_max[axis] && b_min[axis] <= a_max[axis]) {
					expected.push((first, second));
				}
			}
		}

		assert!(expected.len() > boxes.len());
		assert_eq!(swept_pairs(&boxes), expected);
	}

	#[test]
	fn split_ranges_report_the_same_sequence_as_one_scan() {
		let boxes = scattered_boxes(67);
		let mut sweep = SweepBounds::default();
		sweep.rebuild(boxes.iter().copied().enumerate());
		let mut whole = Vec::new();
		sweep.overlaps(0..sweep.count(), |pair| whole.push((pair.a, pair.b)));

		let mut split = Vec::new();
		for range in [0..9, 9..9, 9..40, 40..sweep.count()] {
			sweep.overlaps(range, |pair| split.push((pair.a, pair.b)));
		}

		assert_eq!(split, whole);
	}

	#[test]
	fn rebuilding_moved_boxes_reports_the_same_sequence_as_a_first_sweep() {
		let boxes = scattered_boxes(67);
		let mut sweep = SweepBounds::default();
		sweep.rebuild(boxes.iter().copied().enumerate());

		// The same boxes again, some moved a little and some moved far past their neighbors.
		let moved = boxes
			.iter()
			.enumerate()
			.map(|(index, bounds)| {
				let shift = match index % 5 {
					0 => 0.0,
					4 => 7.5,
					step => step as f32 * -0.15,
				};
				AABB::new(
					bounds.min() + Vector::new(shift, shift * 0.5, 0.0),
					bounds.max() + Vector::new(shift, shift * 0.5, 0.0),
				)
			})
			.collect::<Vec<_>>();
		sweep.rebuild(moved.iter().copied().enumerate());
		let mut repaired = Vec::new();
		sweep.overlaps(0..sweep.count(), |pair| repaired.push((pair.a, pair.b)));

		let mut first = SweepBounds::default();
		first.rebuild(moved.iter().copied().enumerate());
		let mut expected = Vec::new();
		first.overlaps(0..first.count(), |pair| expected.push((pair.a, pair.b)));

		assert!(!expected.is_empty());
		assert_eq!(repaired, expected);
	}

	#[test]
	fn rebuilding_replaces_the_previous_boxes() {
		let mut sweep = SweepBounds::default();
		sweep.rebuild(scattered_boxes(67).into_iter().enumerate());
		sweep.rebuild(
			[bounds([0.0; 3], [1.0; 3]), bounds([5.0, 0.0, 0.0], [1.0; 3])]
				.into_iter()
				.enumerate(),
		);
		let mut pairs = 0;
		sweep.overlaps(0..sweep.count(), |_| pairs += 1);

		assert_eq!(sweep.count(), 2);
		assert_eq!(pairs, 0);
	}
}
