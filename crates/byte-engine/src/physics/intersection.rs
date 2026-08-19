use math::{UnitVector, Vector};
use smallvec::SmallVec;

use crate::physics::dynabit::{body::PhysicsBody, contact::Pair};

/// The `Intersection` struct records a physics contact with world-space geometry.
#[derive(Debug, Clone, Copy)]
pub struct Intersection {
	pub(crate) normal: UnitVector,
	pub(crate) depth: f32,
	pub(crate) point_on_a: math::Point,
	pub(crate) point_on_b: math::Point,
}

/// The `PseudoBody` struct represents one sortable swept-bounds endpoint.
pub struct PseudoBody {
	id: usize,
	value: f32,
	is_min: bool,
}

impl Eq for PseudoBody {}

impl PartialEq for PseudoBody {
	fn eq(&self, other: &Self) -> bool {
		self.cmp(other) == std::cmp::Ordering::Equal
	}
}

impl Ord for PseudoBody {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.value
			.total_cmp(&other.value)
			.then_with(|| other.is_min.cmp(&self.is_min))
			.then_with(|| self.id.cmp(&other.id))
	}
}

impl PartialOrd for PseudoBody {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

/// Projects swept world bounds onto a stable axis and sorts their endpoints.
pub fn sort_bodies_bounds<'a>(bodies: impl Iterator<Item = (usize, &'a PhysicsBody)>, dt: f32) -> SmallVec<[PseudoBody; 32]> {
	debug_assert!(
		dt.is_finite() && dt >= 0.0,
		"Broad-phase delta is invalid. The most likely cause is passing a negative or non-finite frame interval."
	);
	let minimum_body_count = bodies.size_hint().0;
	debug_assert!(
		minimum_body_count.checked_mul(2).is_some(),
		"Broad-phase endpoint capacity overflowed. The most likely cause is an invalid iterator size hint."
	);
	// A positive scaling of this axis preserves endpoint ordering, so unit length is unnecessary.
	let axis: Vector = Vector::new(1.0, 1.0, 1.0);
	let mut endpoints = SmallVec::with_capacity(minimum_body_count.saturating_mul(2));
	for (id, body) in bodies {
		let current = body.bounds();
		let future_min = current.min() + body.linear_velocity * dt;
		let future_max = current.max() + body.linear_velocity * dt;
		let min = math::Point::new(
			current.min().x().min(future_min.x()) - 0.01,
			current.min().y().min(future_min.y()) - 0.01,
			current.min().z().min(future_min.z()) - 0.01,
		);
		let max = math::Point::new(
			current.max().x().max(future_max.x()) + 0.01,
			current.max().y().max(future_max.y()) + 0.01,
			current.max().z().max(future_max.z()) + 0.01,
		);
		endpoints.push(PseudoBody {
			id,
			value: axis.dot(min - math::Point::origin()),
			is_min: true,
		});
		endpoints.push(PseudoBody {
			id,
			value: axis.dot(max - math::Point::origin()),
			is_min: false,
		});
	}
	endpoints.sort();
	endpoints
}

/// Builds every unique pair whose projected intervals overlap.
pub fn build_pairs(endpoints: &[PseudoBody]) -> SmallVec<[Pair; 32]> {
	let mut pairs = SmallVec::new();
	let mut active = SmallVec::<[usize; 32]>::new();
	for endpoint in endpoints {
		if endpoint.is_min {
			pairs.extend(active.iter().copied().map(|id| Pair::new(id, endpoint.id)));
			active.push(endpoint.id);
		} else if let Some(index) = active.iter().position(|id| *id == endpoint.id) {
			// Preserve opening order so contact generation remains deterministic.
			active.remove(index);
		} else {
			debug_assert!(
				false,
				"Broad-phase interval closes before opening. The most likely cause is malformed or unsorted endpoint input."
			);
		}
	}
	debug_assert!(
		active.is_empty(),
		"Broad-phase intervals remain open. The most likely cause is a missing maximum endpoint."
	);
	pairs
}

/// Returns broad-phase candidate pairs using sweep and prune.
pub fn broadphase<'a>(bodies: impl Iterator<Item = (usize, &'a PhysicsBody)>, dt: f32) -> SmallVec<[Pair; 32]> {
	build_pairs(&sort_bodies_bounds(bodies, dt))
}

#[cfg(test)]
mod tests {
	use math::Orientation;

	use super::*;
	use crate::{
		core::factory::Factory,
		physics::{body::BodyTypes, collider::Shapes},
	};

	fn endpoint(id: usize, value: f32, is_min: bool) -> PseudoBody {
		PseudoBody { id, value, is_min }
	}

	fn sorted_endpoints(endpoints: impl IntoIterator<Item = PseudoBody>) -> Vec<PseudoBody> {
		let mut endpoints = endpoints.into_iter().collect::<Vec<_>>();
		endpoints.sort();
		endpoints
	}

	fn canonical_pairs(pairs: impl IntoIterator<Item = Pair>) -> Vec<(usize, usize)> {
		let mut pairs = pairs
			.into_iter()
			.map(|pair| (pair.a.min(pair.b), pair.a.max(pair.b)))
			.collect::<Vec<_>>();
		pairs.sort_unstable();
		pairs
	}

	fn body(position: math::Point, linear_velocity: Vector) -> PhysicsBody {
		let mut factory = Factory::<()>::new();
		PhysicsBody {
			body_type: BodyTypes::Dynamic,
			collision_shape: Shapes::Sphere { radius: 0.5 },
			position,
			orientation: Orientation::identity(),
			acceleration: Vector::zero(),
			linear_velocity,
			angular_velocity: Vector::zero(),
			inv_mass: 1.0,
			center_of_mass: math::Point::origin(),
			elasticity: 0.0,
			friction: 1.0,
			handle: factory.create(()),
		}
	}

	#[test]
	fn endpoint_order_is_total_and_opens_touching_intervals_before_closing() {
		let endpoints = sorted_endpoints([
			endpoint(2, 1.0, false),
			endpoint(1, 1.0, false),
			endpoint(2, 1.0, true),
			endpoint(1, 1.0, true),
		]);

		assert_eq!(
			endpoints
				.iter()
				.map(|endpoint| (endpoint.id, endpoint.is_min))
				.collect::<Vec<_>>(),
			[(1, true), (2, true), (1, false), (2, false)]
		);
	}

	#[test]
	fn sweep_generates_all_pairs_among_simultaneously_active_bodies() {
		let endpoints = sorted_endpoints([
			endpoint(0, 0.0, true),
			endpoint(0, 3.0, false),
			endpoint(1, 1.0, true),
			endpoint(1, 4.0, false),
			endpoint(2, 2.0, true),
			endpoint(2, 5.0, false),
		]);

		assert_eq!(canonical_pairs(build_pairs(&endpoints)), [(0, 1), (0, 2), (1, 2)]);
	}

	#[test]
	fn sweep_distinguishes_disjoint_nested_and_touching_intervals() {
		let disjoint = sorted_endpoints([
			endpoint(0, 0.0, true),
			endpoint(0, 1.0, false),
			endpoint(1, 2.0, true),
			endpoint(1, 3.0, false),
		]);

		assert!(build_pairs(&disjoint).is_empty());

		let nested = sorted_endpoints([
			endpoint(0, 0.0, true),
			endpoint(0, 3.0, false),
			endpoint(1, 1.0, true),
			endpoint(1, 2.0, false),
		]);

		assert_eq!(canonical_pairs(build_pairs(&nested)), [(0, 1)]);

		let touching = sorted_endpoints([
			endpoint(0, 0.0, true),
			endpoint(0, 1.0, false),
			endpoint(1, 1.0, true),
			endpoint(1, 2.0, false),
		]);

		assert_eq!(canonical_pairs(build_pairs(&touching)), [(0, 1)]);
	}

	#[test]
	fn swept_bounds_keep_fast_crossing_bodies_in_the_candidate_set() {
		let bodies = [
			body(math::Point::new(-5.0, -5.0, -5.0), Vector::new(10.0, 10.0, 10.0)),
			body(math::Point::origin(), Vector::zero()),
		];

		assert_eq!(canonical_pairs(broadphase(bodies.iter().enumerate(), 1.0)), [(0, 1)]);
	}
}
