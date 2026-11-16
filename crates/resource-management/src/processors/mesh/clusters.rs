//! Grouping placed copies of a mesh into clusters of neighbors.
//!
//! A level that places one mesh hundreds of times is cheaper to draw as a few merged primitives than as an instance
//! per copy. A renderer builds each merged primitive from the copies of one cluster, so a cluster has to fit one
//! primitive, and has to stay compact for its primitive to cull and pick a detail level as a whole.
//! [`copies_that_fit`] gives the first limit and [`cluster_placements`] makes the clusters.

use std::ops::Range;

use crate::resources::mesh::{MAX_LEVEL_MESHLETS, MAX_PRIMITIVE_VERTICES, Primitive};

/// Returns how many copies of `primitive` fit in one merged primitive that repeats its vertices and meshlets once for
/// each copy.
///
/// Full detail bounds it, since every coarser level has fewer of both. Pass the least of a mesh's primitives to
/// [`cluster_placements`].
pub fn copies_that_fit(primitive: &Primitive) -> usize {
	let meshlets = primitive.lods.first().map_or(1, |lod| lod.meshlet_count as usize);
	(MAX_PRIMITIVE_VERTICES / (primitive.vertex_count as usize).max(1)).min(MAX_LEVEL_MESHLETS / meshlets.max(1))
}

/// Splits placements into groups of neighbors with at most `capacity` members each.
///
/// Returns the placements' indices reordered, and each group's range of that order. The split halves the placements
/// along the longest axis of their positions until a half fits, so groups stay compact and hold between half of
/// `capacity` and all of it, except when everything fits one group. Within a group the placements follow a Z-order
/// curve through the group's bounds, so placements next to each other in the order are near each other in space,
/// which lets a renderer pack neighboring copies into shared meshlets. It knows nothing of what is placed: pass the
/// number of copies one merged primitive holds, such as [`copies_that_fit`].
pub fn cluster_placements(positions: &[[f32; 3]], capacity: usize) -> (Vec<u32>, Vec<Range<usize>>) {
	assert!(
		capacity > 0,
		"Cluster capacity is zero. The most likely cause is a placed primitive too large for any copy to fit a merged primitive."
	);
	let mut order: Vec<u32> = (0..positions.len() as u32).collect();
	let mut clusters = Vec::with_capacity(positions.len().div_ceil(capacity));
	// Ranges still to split, which stands in for recursion.
	let mut pending = vec![0..order.len()];
	while let Some(range) = pending.pop() {
		if range.is_empty() {
			continue;
		}
		let members = &mut order[range.clone()];
		let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
		for &member in members.iter() {
			for axis in 0..3 {
				min[axis] = min[axis].min(positions[member as usize][axis]);
				max[axis] = max[axis].max(positions[member as usize][axis]);
			}
		}
		if range.len() <= capacity {
			members.sort_by_cached_key(|&member| z_order(positions[member as usize], min, max));
			clusters.push(range);
			continue;
		}
		let axis = (0..3)
			.max_by(|&a, &b| (max[a] - min[a]).total_cmp(&(max[b] - min[b])))
			.expect("a range of three axes has a largest");
		let middle = members.len() / 2;
		members.select_nth_unstable_by(middle, |&a, &b| {
			positions[a as usize][axis].total_cmp(&positions[b as usize][axis])
		});
		// The far half is pushed first so groups come out in order along the range.
		pending.push(range.start + middle..range.end);
		pending.push(range.start..range.start + middle);
	}
	(order, clusters)
}

/// Returns where `position` lies on a Z-order curve through the box from `min` to `max`: its coordinates, each
/// reduced to ten bits of the box's extent, with their bits interleaved.
fn z_order(position: [f32; 3], min: [f32; 3], max: [f32; 3]) -> u32 {
	let mut code = 0;
	for axis in 0..3 {
		let extent = max[axis] - min[axis];
		let cell = if extent > 0.0 {
			((position[axis] - min[axis]) / extent * 1023.0) as u32
		} else {
			0
		};
		for bit in 0..10 {
			code |= (cell >> bit & 1) << (bit * 3 + axis);
		}
	}
	code
}

#[cfg(test)]
mod tests {
	use super::cluster_placements;

	#[test]
	fn clusters_hold_every_placement_once_within_capacity_and_apart_from_each_other() {
		// Scattered along x in an order that isn't sorted, so the split has to sort them out.
		let positions: Vec<[f32; 3]> = (0..1000u32)
			.map(|index| [(index * 7919 % 1000) as f32, (index % 3) as f32, 0.0])
			.collect();

		let (order, clusters) = cluster_placements(&positions, 70);

		let mut seen = order.clone();
		seen.sort_unstable();
		assert_eq!(seen, (0..1000).collect::<Vec<u32>>());
		assert_eq!(clusters.iter().map(|cluster| cluster.len()).sum::<usize>(), 1000);
		let mut spans: Vec<(f32, f32)> = clusters
			.iter()
			.map(|cluster| {
				assert!(
					cluster.len() <= 70,
					"A cluster holds {} placements, past its capacity.",
					cluster.len()
				);
				let xs = order[cluster.clone()].iter().map(|&member| positions[member as usize][0]);
				(xs.clone().fold(f32::INFINITY, f32::min), xs.fold(f32::NEG_INFINITY, f32::max))
			})
			.collect();
		spans.sort_by(|a, b| a.0.total_cmp(&b.0));
		for pair in spans.windows(2) {
			assert!(
				pair[0].1 < pair[1].0,
				"Two clusters overlap along the axis the placements spread on, so neither culls apart from the other."
			);
		}
	}

	#[test]
	fn placements_that_fit_one_group_stay_together() {
		let positions = [[0.0; 3], [5.0, 0.0, 0.0], [9.0, 0.0, 0.0]];

		let (order, clusters) = cluster_placements(&positions, 3);

		assert_eq!(order, [0, 1, 2]);
		assert_eq!(clusters, [0..3]);
	}

	#[test]
	fn a_group_lists_its_placements_with_neighbors_next_to_each_other() {
		// Two tight pairs far apart, given interleaved.
		let positions = [[0.0; 3], [100.0, 0.0, 100.0], [1.0, 0.0, 0.0], [101.0, 0.0, 100.0]];

		let (order, clusters) = cluster_placements(&positions, 4);

		assert_eq!(clusters, [0..4]);
		// Members 0 and 2 are one pair and 1 and 3 the other. Each pair stays together, whichever comes first.
		let pair = |member: u32| member % 2;
		assert_eq!(pair(order[0]), pair(order[1]));
		assert_eq!(pair(order[2]), pair(order[3]));
		assert_ne!(pair(order[0]), pair(order[2]));
	}
}
