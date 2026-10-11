//! Contact response for Dynabit: which contacts can be resolved together, and the changes that resolve them.
//!
//! [`ContactBatches`] sorts simultaneous contacts into batches that touch every movable body at most once. The
//! contacts of one batch do not depend on each other, so [`respond`] computes several at once and lanes can share a
//! batch. [`apply`] then writes the responses to the bodies.

use std::{
	ops::{Add, Mul, Neg, Sub},
	simd::{
		Select as _, StdFloat as _,
		cmp::{SimdPartialEq as _, SimdPartialOrd as _},
		f32x4, mask32x4,
		num::SimdFloat as _,
	},
};

use math::{Point, Vector};
use utils::StableVec;

use crate::physics::dynabit::{
	body::{MAX_ANGULAR_SPEED, PhysicsBody},
	contact::{Contact, Side},
};

/// The number of contacts one [`respond`] call resolves.
pub(crate) const WIDTH: usize = 4;

/// The number of batches [`ContactBatches`] can form, one for each bit of a body's record of the batches it is in.
const BATCH_LIMIT: usize = u64::BITS as usize;

/// The `ContactBatches` struct sorts a step's simultaneous contacts into batches in which no movable body appears
/// twice, so the contacts of one batch can be resolved in any order, or at the same time, with the same result.
///
/// Call [`Self::rebuild`] once the step's contacts are known, resolve [`Self::batch`] `0` to [`Self::len`] in order,
/// and then resolve [`Self::leftover`] one contact at a time.
#[derive(Default)]
pub(crate) struct ContactBatches {
	/// Contact indices, grouped by batch.
	order: Vec<u32>,
	/// Where each batch ends in `order`.
	ends: Vec<usize>,
	/// The contacts that fit in no batch, in the order they were given.
	leftover: Vec<u32>,
	/// The batch each contact was given, kept between rebuilds so batching does not allocate.
	assigned: Vec<u8>,
	/// For each body, the batches that already hold one of its contacts, one bit per batch.
	taken: Vec<u64>,
}

impl ContactBatches {
	/// Replaces the batches with ones for `contacts`, each given as the movable bodies it changes.
	///
	/// A body that contacts never change, such as a static one, is `None`: any number of a batch's contacts can
	/// read it. Bodies are indices below `body_count`. Each contact joins the first batch that holds neither of its
	/// bodies, so the batches depend only on the order of `contacts`.
	pub(crate) fn rebuild(&mut self, body_count: usize, contacts: impl Iterator<Item = [Option<usize>; 2]>) {
		self.taken.clear();
		self.taken.resize(body_count, 0);
		self.assigned.clear();
		self.leftover.clear();
		let mut sizes = [0usize; BATCH_LIMIT];
		for (index, bodies) in contacts.enumerate() {
			let taken = bodies.iter().flatten().fold(0, |taken, body| taken | self.taken[*body]);
			let batch = (!taken).trailing_zeros() as usize;
			if batch == BATCH_LIMIT {
				// One of the bodies is already in every batch.
				self.leftover.push(index as u32);
			} else {
				for body in bodies.iter().flatten() {
					self.taken[*body] |= 1 << batch;
				}
				sizes[batch] += 1;
			}
			self.assigned.push(batch as u8);
		}

		// A contact only opens a batch when its bodies fill every earlier one, so the used batches are the first ones.
		let mut starts = [0usize; BATCH_LIMIT];
		self.ends.clear();
		let mut end = 0;
		for (batch, size) in sizes.into_iter().enumerate().take_while(|(_, size)| *size != 0) {
			starts[batch] = end;
			end += size;
			self.ends.push(end);
		}
		self.order.clear();
		self.order.resize(end, 0);
		for (index, batch) in self.assigned.iter().map(|batch| usize::from(*batch)).enumerate() {
			if batch != BATCH_LIMIT {
				self.order[starts[batch]] = index as u32;
				starts[batch] += 1;
			}
		}
	}

	/// Returns the number of batches.
	pub(crate) fn len(&self) -> usize {
		self.ends.len()
	}

	/// Returns the contacts of one batch, in the order they were given to [`Self::rebuild`].
	pub(crate) fn batch(&self, batch: usize) -> &[u32] {
		let start = if batch == 0 { 0 } else { self.ends[batch - 1] };
		&self.order[start..self.ends[batch]]
	}

	/// Returns the contacts that fit in no batch, in the order they were given to [`Self::rebuild`].
	///
	/// Resolve them one at a time after every batch, because they share bodies with each other.
	pub(crate) fn leftover(&self) -> &[u32] {
		&self.leftover
	}
}

/// The `WideVector` struct holds one vector for each contact of a group, so vector math runs on the whole group.
#[derive(Clone, Copy)]
struct WideVector {
	x: f32x4,
	y: f32x4,
	z: f32x4,
}

impl WideVector {
	/// Builds the vectors from one vector for each lane.
	#[inline]
	fn from_lanes(lanes: [[f32; 3]; WIDTH]) -> Self {
		Self {
			x: f32x4::from_array(lanes.map(|lane| lane[0])),
			y: f32x4::from_array(lanes.map(|lane| lane[1])),
			z: f32x4::from_array(lanes.map(|lane| lane[2])),
		}
	}

	/// Returns the vector of each lane.
	#[inline]
	fn to_lanes(self) -> [[f32; 3]; WIDTH] {
		let (x, y, z) = (self.x.to_array(), self.y.to_array(), self.z.to_array());
		std::array::from_fn(|lane| [x[lane], y[lane], z[lane]])
	}

	#[inline]
	fn dot(self, other: Self) -> f32x4 {
		self.x * other.x + self.y * other.y + self.z * other.z
	}

	#[inline]
	fn cross(self, other: Self) -> Self {
		Self {
			x: self.y * other.z - self.z * other.y,
			y: self.z * other.x - self.x * other.z,
			z: self.x * other.y - self.y * other.x,
		}
	}

	/// Returns `chosen` in the lanes `mask` sets and `otherwise` in the others.
	#[inline]
	fn select(mask: mask32x4, chosen: Self, otherwise: Self) -> Self {
		Self {
			x: mask.select(chosen.x, otherwise.x),
			y: mask.select(chosen.y, otherwise.y),
			z: mask.select(chosen.z, otherwise.z),
		}
	}
}

impl Add for WideVector {
	type Output = Self;

	#[inline]
	fn add(self, other: Self) -> Self {
		Self {
			x: self.x + other.x,
			y: self.y + other.y,
			z: self.z + other.z,
		}
	}
}

impl Sub for WideVector {
	type Output = Self;

	#[inline]
	fn sub(self, other: Self) -> Self {
		Self {
			x: self.x - other.x,
			y: self.y - other.y,
			z: self.z - other.z,
		}
	}
}

impl Mul<f32x4> for WideVector {
	type Output = Self;

	#[inline]
	fn mul(self, scale: f32x4) -> Self {
		Self {
			x: self.x * scale,
			y: self.y * scale,
			z: self.z * scale,
		}
	}
}

impl Neg for WideVector {
	type Output = Self;

	#[inline]
	fn neg(self) -> Self {
		Self {
			x: -self.x,
			y: -self.y,
			z: -self.z,
		}
	}
}

/// Multiplies the matrix with rows `rows` by `vector`.
#[inline]
fn transform(rows: &[WideVector; 3], vector: WideVector) -> WideVector {
	WideVector {
		x: rows[0].dot(vector),
		y: rows[1].dot(vector),
		z: rows[2].dot(vector),
	}
}

/// The `WideSide` struct holds the state [`respond`] reads from one body of each contact in a group.
struct WideSide {
	/// The collider origin, which penetration correction moves.
	position: WideVector,
	/// The center of mass in world space.
	center: WideVector,
	linear_velocity: WideVector,
	angular_velocity: WideVector,
	inverse_mass: f32x4,
	/// The rows of the inverse inertia tensor in world space.
	inverse_inertia: [WideVector; 3],
	elasticity: f32x4,
	friction: f32x4,
	/// The contact point on this body.
	point: WideVector,
}

impl WideSide {
	/// Reads the bodies `sides` name, and derives each one's world-space center of mass and inverse inertia.
	#[inline]
	fn gather(bodies: &StableVec<PhysicsBody>, sides: [&Side; WIDTH]) -> Self {
		let bodies = sides.map(|side| {
			bodies.get_slot(side.object).expect(
				"Contact body is missing. The most likely cause is a body removed between contact detection and resolution.",
			)
		});
		let scalar = |read: fn(&PhysicsBody) -> f32| f32x4::from_array(bodies.map(read));
		let vector = |read: fn(&PhysicsBody) -> [f32; 3]| WideVector::from_lanes(bodies.map(read));
		let position = vector(|body| body.position.to_array());
		let inverse_mass = scalar(|body| body.inv_mass);

		// The rotation matrix of each orientation, by rows.
		let (x, y) = (
			scalar(|body| body.orientation.to_array()[0]),
			scalar(|body| body.orientation.to_array()[1]),
		);
		let (z, w) = (
			scalar(|body| body.orientation.to_array()[2]),
			scalar(|body| body.orientation.to_array()[3]),
		);
		let (one, two) = (f32x4::splat(1.0), f32x4::splat(2.0));
		let rotation = [
			WideVector {
				x: one - two * y * y - two * z * z,
				y: two * x * y - two * z * w,
				z: two * x * z + two * y * w,
			},
			WideVector {
				x: two * x * y + two * z * w,
				y: one - two * x * x - two * z * z,
				z: two * y * z - two * x * w,
			},
			WideVector {
				x: two * x * z - two * y * w,
				y: two * y * z + two * x * w,
				z: one - two * x * x - two * y * y,
			},
		];

		// Every supported shape has a diagonal local inertia tensor, so its inverse is the reciprocal of each moment.
		// Rotating that diagonal into world space is `rotation * diagonal * transpose(rotation)`.
		let moments = vector(|body| {
			let moments = body.collision_shape.principal_inertia();
			[moments.x, moments.y, moments.z]
		});
		let inverse_moments = WideVector {
			x: inverse_mass / moments.x,
			y: inverse_mass / moments.y,
			z: inverse_mass / moments.z,
		};
		let inverse_inertia = rotation.map(|row| {
			let scaled = WideVector {
				x: row.x * inverse_moments.x,
				y: row.y * inverse_moments.y,
				z: row.z * inverse_moments.z,
			};
			transform(&rotation, scaled)
		});

		Self {
			position,
			center: position + transform(&rotation, vector(|body| body.center_of_mass.to_array())),
			linear_velocity: vector(|body| body.linear_velocity.to_array()),
			angular_velocity: vector(|body| body.angular_velocity.to_array()),
			inverse_mass,
			inverse_inertia,
			elasticity: scalar(|body| body.elasticity),
			friction: scalar(|body| body.friction),
			point: WideVector::from_lanes(sides.map(|side| side.point.to_array())),
		}
	}

	/// Returns how much an impulse along `direction` at the end of `arm` changes this body's speed there, without
	/// the share its mass contributes.
	#[inline]
	fn angular_resistance(&self, arm: WideVector, direction: WideVector) -> WideVector {
		transform(&self.inverse_inertia, arm.cross(direction)).cross(arm)
	}

	/// Returns `velocities` after `impulse` acts on this body at the end of `arm`, measured from its center of mass.
	#[inline]
	fn pushed(&self, velocities: Velocities, arm: WideVector, impulse: WideVector) -> Velocities {
		let angular = velocities.angular + transform(&self.inverse_inertia, arm.cross(impulse));
		// A contact far from the center of mass can spin a body faster than the simulation can follow, so the spin
		// is capped.
		let speed_squared = angular.dot(angular);
		let too_fast = speed_squared.simd_gt(f32x4::splat(MAX_ANGULAR_SPEED * MAX_ANGULAR_SPEED));
		let scale = too_fast.select(f32x4::splat(MAX_ANGULAR_SPEED) / speed_squared.sqrt(), f32x4::splat(1.0));
		Velocities {
			linear: velocities.linear + impulse * self.inverse_mass,
			angular: angular * scale,
		}
	}
}

/// The `Velocities` struct pairs the linear and angular velocities of one body of each contact in a group.
#[derive(Clone, Copy)]
struct Velocities {
	linear: WideVector,
	angular: WideVector,
}

/// The `Response` struct holds the state that up to [`WIDTH`] contacts leave their bodies in.
///
/// [`respond`] computes it and [`apply`] writes it to the bodies.
pub(crate) struct Response {
	sides: [SideResponse; 2],
}

/// The `SideResponse` struct holds the new state of one body of each contact in a group.
struct SideResponse {
	bodies: [usize; WIDTH],
	/// The lanes whose body takes the state below, one bit per lane.
	changed: u64,
	position: WideVector,
	velocities: Velocities,
}

/// Computes the state the contacts `group` names in `contacts` leave their bodies in, from the bodies' current state.
///
/// Each contact pushes its bodies apart along the contact normal, slows their sliding, and, when the bodies overlap
/// at the start of the step, moves them out of each other. `group` holds at most [`WIDTH`] contacts, and no movable
/// body may be in two of them. Call [`apply`] with the result.
pub(crate) fn respond(bodies: &StableVec<PhysicsBody>, contacts: &[Contact], group: &[u32]) -> Response {
	debug_assert!(
		!group.is_empty() && group.len() <= WIDTH,
		"Contact group size is invalid. The most likely cause is a batch that was not split into groups of {WIDTH}."
	);
	// Unused lanes repeat the last contact so that every lane reads real bodies. Their results are never applied.
	let contacts: [&Contact; WIDTH] = std::array::from_fn(|lane| &contacts[group[lane.min(group.len() - 1)] as usize]);
	let in_use = mask32x4::from_array(std::array::from_fn(|lane| lane < group.len()));
	let a = WideSide::gather(bodies, contacts.map(|contact| &contact.a));
	let b = WideSide::gather(bodies, contacts.map(|contact| &contact.b));
	let depth = f32x4::from_array(contacts.map(|contact| contact.depth));
	// Only bodies that overlap at the start of the step are moved apart.
	let overlapping = f32x4::from_array(contacts.map(|contact| contact.toi)).simd_eq(f32x4::splat(0.0));
	let epsilon = f32x4::splat(f32::EPSILON);

	// The normal must point from A toward B.
	let normal = WideVector::from_lanes(contacts.map(|contact| contact.normal.into_vector().to_array()));
	let flipped = (b.center - a.center).dot(normal).simd_lt(f32x4::splat(0.0));
	let normal = WideVector::select(flipped, -normal, normal);

	let inverse_mass_sum = a.inverse_mass + b.inverse_mass;
	let (a_arm, b_arm) = (a.point - a.center, b.point - b.center);
	let a_point_velocity = a.linear_velocity + a.angular_velocity.cross(a_arm);
	let b_point_velocity = b.linear_velocity + b.angular_velocity.cross(b_arm);
	let relative_velocity = a_point_velocity - b_point_velocity;
	let normal_speed = relative_velocity.dot(normal);
	let denominator =
		inverse_mass_sum + (a.angular_resistance(a_arm, normal) + b.angular_resistance(b_arm, normal)).dot(normal);
	// A contact whose bodies cannot take an impulse, or whose mass or inertia data is invalid, changes nothing.
	let resolved =
		in_use & inverse_mass_sum.simd_ne(f32x4::splat(0.0)) & denominator.is_finite() & denominator.simd_gt(epsilon);

	let impulse = normal * ((f32x4::splat(1.0) + a.elasticity * b.elasticity) * normal_speed / denominator);
	let a_before = Velocities {
		linear: a.linear_velocity,
		angular: a.angular_velocity,
	};
	let b_before = Velocities {
		linear: b.linear_velocity,
		angular: b.angular_velocity,
	};
	let a_bounced = a.pushed(a_before, a_arm, -impulse);
	let b_bounced = b.pushed(b_before, b_arm, impulse);

	// Friction opposes the sliding the bodies had before the bounce.
	let sliding_velocity = relative_velocity - normal * normal_speed;
	let sliding_speed_squared = sliding_velocity.dot(sliding_velocity);
	let sliding = sliding_speed_squared.simd_gt(epsilon);
	// Lanes that are not sliding divide by one instead of by their zero speed.
	let tangent = sliding_velocity * (f32x4::splat(1.0) / sliding.select(sliding_speed_squared.sqrt(), f32x4::splat(1.0)));
	let friction_denominator =
		inverse_mass_sum + (a.angular_resistance(a_arm, tangent) + b.angular_resistance(b_arm, tangent)).dot(tangent);
	let gripping = sliding & friction_denominator.is_finite() & friction_denominator.simd_gt(epsilon);
	let friction_impulse = sliding_velocity * (a.friction * b.friction / friction_denominator);
	let a_slowed = a.pushed(a_bounced, a_arm, -friction_impulse);
	let b_slowed = b.pushed(b_bounced, b_arm, friction_impulse);

	// Each body moves out of the overlap in proportion to how easily it moves.
	let separation = normal * depth;
	let a_separated = a.position - separation * (a.inverse_mass / inverse_mass_sum);
	let b_separated = b.position + separation * (b.inverse_mass / inverse_mass_sum);

	let side = |side: &WideSide, sides: [&Side; WIDTH], separated, bounced: Velocities, slowed: Velocities| SideResponse {
		bodies: sides.map(|side| side.object),
		changed: (resolved & side.inverse_mass.simd_ne(f32x4::splat(0.0))).to_bitmask(),
		position: WideVector::select(overlapping, separated, side.position),
		velocities: Velocities {
			linear: WideVector::select(gripping, slowed.linear, bounced.linear),
			angular: WideVector::select(gripping, slowed.angular, bounced.angular),
		},
	};
	Response {
		sides: [
			side(&a, contacts.map(|contact| &contact.a), a_separated, a_bounced, a_slowed),
			side(&b, contacts.map(|contact| &contact.b), b_separated, b_bounced, b_slowed),
		],
	}
}

/// Writes the state [`respond`] computed to the bodies it computed it for.
pub(crate) fn apply(bodies: &mut StableVec<PhysicsBody>, response: &Response) {
	for side in &response.sides {
		let (positions, linear, angular) = (
			side.position.to_lanes(),
			side.velocities.linear.to_lanes(),
			side.velocities.angular.to_lanes(),
		);
		let mut lanes = side.changed;
		while lanes != 0 {
			let lane = lanes.trailing_zeros() as usize;
			lanes &= lanes - 1;
			if let Some(body) = bodies.get_slot_mut(side.bodies[lane]) {
				body.position = Point::from_array(positions[lane]);
				body.linear_velocity = Vector::from_array(linear[lane]);
				body.angular_velocity = Vector::from_array(angular[lane]);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Returns the bodies of contacts that chain `count` movable bodies to their neighbors and to one static body.
	fn chained_contacts(count: usize) -> Vec<[Option<usize>; 2]> {
		let mut contacts = Vec::new();
		for body in 0..count {
			contacts.push([Some(body), None]);
			if body + 1 < count {
				contacts.push([Some(body), Some(body + 1)]);
			}
		}
		contacts
	}

	#[test]
	fn a_batch_holds_each_movable_body_once() {
		let contacts = chained_contacts(9);
		let mut batches = ContactBatches::default();
		batches.rebuild(9, contacts.iter().copied());

		for batch in 0..batches.len() {
			let mut seen = [false; 9];
			for body in batches
				.batch(batch)
				.iter()
				.flat_map(|contact| contacts[*contact as usize])
				.flatten()
			{
				assert!(!seen[body]);
				seen[body] = true;
			}
		}
	}

	#[test]
	fn every_contact_is_placed_once() {
		let contacts = chained_contacts(9);
		let mut batches = ContactBatches::default();
		batches.rebuild(9, contacts.iter().copied());

		let mut placed = (0..batches.len())
			.flat_map(|batch| batches.batch(batch).iter().copied())
			.chain(batches.leftover().iter().copied())
			.collect::<Vec<_>>();
		placed.sort_unstable();

		assert_eq!(placed, (0..contacts.len() as u32).collect::<Vec<_>>());
	}

	#[test]
	fn contacts_that_only_share_immovable_bodies_fill_one_batch() {
		let contacts = (0..5).map(|body| [Some(body), None]).collect::<Vec<_>>();
		let mut batches = ContactBatches::default();
		batches.rebuild(5, contacts.iter().copied());

		assert_eq!(batches.len(), 1);
		assert_eq!(batches.batch(0), [0, 1, 2, 3, 4]);
	}

	#[test]
	fn contacts_beyond_the_batch_limit_are_left_over_in_order() {
		// One body in more contacts than there are batches.
		let contacts = (0..BATCH_LIMIT + 3)
			.map(|other| [Some(0), Some(other + 1)])
			.collect::<Vec<_>>();
		let mut batches = ContactBatches::default();
		batches.rebuild(BATCH_LIMIT + 4, contacts.iter().copied());

		assert_eq!(batches.len(), BATCH_LIMIT);
		assert_eq!(
			batches.leftover(),
			[BATCH_LIMIT as u32, BATCH_LIMIT as u32 + 1, BATCH_LIMIT as u32 + 2]
		);
	}

	#[test]
	fn rebuilding_replaces_the_previous_batches() {
		let mut batches = ContactBatches::default();
		batches.rebuild(9, chained_contacts(9).into_iter());
		batches.rebuild(2, [[Some(0), Some(1)]].into_iter());

		assert_eq!(batches.len(), 1);
		assert_eq!(batches.batch(0), [0]);
		assert!(batches.leftover().is_empty());
	}
}
