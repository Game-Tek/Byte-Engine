use math::{dot, normalize, Base, Vector3};
use smallvec::SmallVec;

use crate::physics::{
	dynabit::{body::PhysicsBody, contact::Pair},
	Body,
};

pub struct Intersection {
	pub(crate) normal: Vector3,
	pub(crate) depth: f32,
	pub(crate) point_on_a: Vector3,
	pub(crate) point_on_b: Vector3,
}

/// The `PseudoBody` struct provides a sortable one-dimensional bound endpoint
/// for broad-phase collision candidate generation.
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
			// Process opening endpoints before closing endpoints so touching bounds
			// remain broad-phase candidates.
			.then_with(|| other.is_min.cmp(&self.is_min))
			.then_with(|| self.id.cmp(&other.id))
	}
}

impl PartialOrd for PseudoBody {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

/// Projects swept body bounds onto the broad-phase axis and sorts their endpoints.
pub fn sort_bodies_bounds<'a>(bodies: impl Iterator<Item = (usize, &'a PhysicsBody)>, dt: f32) -> SmallVec<[PseudoBody; 32]> {
	let axis = normalize(Vector3::one());

	let mut pseudo_bodies = SmallVec::with_capacity(bodies.size_hint().0 * 2);

	for (i, body) in bodies {
		let mut bounds = body.bounds(); // TODO: bounds() does not adjust by orientation
		let future_bounds = bounds + body.linear_velocity * dt;
		// Continuous collision detection needs the whole path, not only the
		// predicted endpoint, to reach the narrow phase.
		bounds.expand(&future_bounds);

		let epsilon = 0.01f32;

		let bounds = bounds.expanded_by(Vector3::one() * epsilon);

		pseudo_bodies.push(PseudoBody {
			id: i,
			value: dot(axis, bounds.min()),
			is_min: true,
		});

		pseudo_bodies.push(PseudoBody {
			id: i,
			value: dot(axis, bounds.max()),
			is_min: false,
		});
	}

	pseudo_bodies.sort();

	pseudo_bodies
}

/// Builds every unique pair whose projected intervals overlap.
pub fn build_pairs(pseudo_bodies: &[PseudoBody]) -> SmallVec<[Pair; 32]> {
	let mut pairs = SmallVec::new();
	let mut active = SmallVec::<[usize; 32]>::new();

	for endpoint in pseudo_bodies {
		if endpoint.is_min {
			for &active_id in &active {
				pairs.push(Pair::new(active_id, endpoint.id));
			}
			active.push(endpoint.id);
		} else if let Some(index) = active.iter().position(|id| *id == endpoint.id) {
			// Preserve opening order so contact generation remains deterministic even
			// when an early interval closes while later intervals stay active.
			active.remove(index);
		}
	}

	pairs
}

pub fn sweep_and_prune_1d<'a>(bodies: impl Iterator<Item = (usize, &'a PhysicsBody)>, dt: f32) -> SmallVec<[Pair; 32]> {
	let e = sort_bodies_bounds(bodies, dt);

	build_pairs(&e)
}

pub fn broadphase<'a>(bodies: impl Iterator<Item = (usize, &'a PhysicsBody)>, dt: f32) -> SmallVec<[Pair; 32]> {
	sweep_and_prune_1d(bodies, dt)
}

#[cfg(test)]
mod tests {
	use math::Quaternion;

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

	fn body(position: Vector3, linear_velocity: Vector3) -> PhysicsBody {
		PhysicsBody {
			body_type: BodyTypes::Dynamic,
			collision_shape: Shapes::Sphere { radius: 0.5 },
			position,
			orientation: Quaternion::identity(),
			acceleration: Vector3::new(0.0, 0.0, 0.0),
			linear_velocity,
			angular_velocity: Vector3::new(0.0, 0.0, 0.0),
			inv_mass: 1.0,
			center_of_mass: Vector3::new(0.0, 0.0, 0.0),
			elasticity: 0.0,
			friction: 1.0,
			handle: Factory::<()>::new().create(()),
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
	fn sweep_generates_all_pairs_among_three_simultaneously_active_bodies() {
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
	fn closing_an_early_interval_preserves_pair_generation_order() {
		let endpoints = sorted_endpoints([
			endpoint(0, 0.0, true),
			endpoint(1, 1.0, true),
			endpoint(2, 2.0, true),
			endpoint(0, 3.0, false),
			endpoint(3, 4.0, true),
			endpoint(1, 5.0, false),
			endpoint(2, 6.0, false),
			endpoint(3, 7.0, false),
		]);

		let pairs = build_pairs(&endpoints)
			.into_iter()
			.map(|pair| (pair.a, pair.b))
			.collect::<Vec<_>>();
		assert_eq!(pairs, [(0, 1), (0, 2), (1, 2), (1, 3), (2, 3)]);
	}

	#[test]
	fn swept_bounds_keep_fast_crossing_bodies_in_the_candidate_set() {
		let moving = body(Vector3::new(-5.0, -5.0, -5.0), Vector3::new(10.0, 10.0, 10.0));
		let stationary = body(Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0));
		let bodies = [moving, stationary];

		assert_eq!(canonical_pairs(broadphase(bodies.iter().enumerate(), 1.0)), [(0, 1)]);
	}
}
