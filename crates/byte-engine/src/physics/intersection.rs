use math::{dot, normalize, Base, Vector3};

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

pub struct PseudoBody {
	id: usize,
	value: f32,
	is_min: bool,
}

impl Eq for PseudoBody {}

impl PartialEq for PseudoBody {
	fn eq(&self, other: &Self) -> bool {
		self.value == other.value
	}
}

impl Ord for PseudoBody {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.value.partial_cmp(&other.value).unwrap_or(std::cmp::Ordering::Equal)
	}
}

impl PartialOrd for PseudoBody {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		self.value.partial_cmp(&other.value)
	}
}

pub fn sort_bodies_bounds<'a>(bodies: impl Iterator<Item = (usize, &'a PhysicsBody)>, dt: f32) -> Vec<PseudoBody> {
	let axis = normalize(Vector3::one());

	let mut pseudo_bodies = Vec::with_capacity(bodies.size_hint().0 * 2);

	for (i, body) in bodies {
		let bounds = body.bounds(); // TODO: bounds() does not adjust by orientation

		let bounds = bounds + body.linear_velocity * dt;

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

pub fn build_pairs(pseudo_bodies: &[PseudoBody]) -> Vec<Pair> {
	let mut pairs = Vec::new();

	for a in pseudo_bodies.iter() {
		if !a.is_min {
			continue;
		}

		let a_id = a.id;

		for b in pseudo_bodies.iter().skip(1) {
			if b.id == a.id {
				break;
			}

			if !b.is_min {
				continue;
			}

			let b_id = b.id;

			pairs.push(Pair::new(a_id, b_id));
		}
	}

	pairs
}

pub fn sweep_and_prune_1d<'a>(bodies: impl Iterator<Item = (usize, &'a PhysicsBody)>, dt: f32) -> Vec<Pair> {
	let e = sort_bodies_bounds(bodies, dt);
	let pairs = build_pairs(&e);
	pairs
}

pub fn broadphase<'a>(bodies: impl Iterator<Item = (usize, &'a PhysicsBody)>, dt: f32) -> Vec<Pair> {
	let pairs = sweep_and_prune_1d(bodies, dt);
	pairs
}
