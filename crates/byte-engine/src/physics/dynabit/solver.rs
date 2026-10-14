//! Contact response for Dynabit: which contacts can be solved together, and the velocity changes that solve them.
//!
//! [`ContactSolver`] takes a step's contacts through it. The alley's [`PairBatches`] sort simultaneous contacts into
//! batches that touch every movable body at most once. The contacts of one batch do not depend on each other, so
//! they are solved several at a time and lanes can share a batch. [`prepare`] works out what each group of contacts
//! needs, and [`solve`] passes over a group, changing the [`BodyMotion`] of its bodies.

use std::{
	ops::{Add, Mul, Sub},
	simd::{
		Select as _, StdFloat as _,
		cmp::{SimdPartialEq as _, SimdPartialOrd as _},
		f32x4, mask32x4,
		num::SimdFloat as _,
		simd_swizzle,
	},
	sync::atomic::{AtomicU32, AtomicU64, Ordering},
};

use math::{Point, Vector};

use crate::{
	core::alley::{AlleyId, BatchOwned, Claim, ConcreteLane, LaneCells, LaneRanges, PairBatches, RangeOwned, Stage},
	physics::dynabit::{
		body::PhysicsBody,
		contact::{Contact, Contacts, Side},
	},
};

/// The number of contacts one [`WideContact`] holds.
const WIDTH: usize = 4;

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

/// Multiplies the matrix with rows `rows` by `vector`.
#[inline]
fn transform(rows: &[WideVector; 3], vector: WideVector) -> WideVector {
	WideVector {
		x: rows[0].dot(vector),
		y: rows[1].dot(vector),
		z: rows[2].dot(vector),
	}
}

/// The `BodyState` struct holds what a step's contacts need to know about one body, so that they read it from one
/// cache line and work none of it out again. A body in a pile is in four or five contacts, and each reads it.
///
/// The first three lanes of each vector are a vector's coordinates. [`ContactSolver`] keeps one for every body, by
/// body slot. The body's angular velocity before contacts change it would make this longer than a cache line, so it is kept in the
/// body's [`BodyMotion`], which has room for it.
#[derive(Clone, Copy, Default)]
#[repr(align(64))]
pub(super) struct BodyState {
	/// The center of mass in world space, followed by the inverse mass.
	center: f32x4,
	/// The diagonal of the inverse inertia tensor in world space, followed by the body's elasticity.
	moments: f32x4,
	/// The entries of that tensor off its diagonal, which is symmetric: the one for x and y, for x and z, and for y
	/// and z. They are followed by the body's friction.
	products: f32x4,
	/// The linear velocity the body has before any contact changes it.
	linear_velocity: f32x4,
}

impl BodyState {
	/// Works out the state of `body` where it is now.
	fn of(body: &PhysicsBody) -> Self {
		let inverse_moments = body.inverse_moments();
		let offset = body.center_of_mass.to_array();
		// A body that turns as easily about every axis, or not at all, does so however it is turned, and a center of
		// mass at the body's own position stays there. That is every ball, and every body that does not rotate, so
		// most bodies of most scenes need none of the work below.
		let even = inverse_moments[0] == inverse_moments[1] && inverse_moments[1] == inverse_moments[2];
		let (offset, moments, products) = if even && offset == [0.0; 3] {
			(offset, inverse_moments, [0.0; 3])
		} else {
			// The rotation matrix of the body's orientation, by rows.
			let [x, y, z, w] = body.orientation.to_array();
			let rotation = [
				[
					1.0 - 2.0 * y * y - 2.0 * z * z,
					2.0 * x * y - 2.0 * z * w,
					2.0 * x * z + 2.0 * y * w,
				],
				[
					2.0 * x * y + 2.0 * z * w,
					1.0 - 2.0 * x * x - 2.0 * z * z,
					2.0 * y * z - 2.0 * x * w,
				],
				[
					2.0 * x * z - 2.0 * y * w,
					2.0 * y * z + 2.0 * x * w,
					1.0 - 2.0 * x * x - 2.0 * y * y,
				],
			];
			// Every supported shape has a diagonal local inertia tensor, so its inverse is the reciprocal of each
			// moment. Rotating that diagonal into world space is `rotation * diagonal * transpose(rotation)`, of
			// which this is one entry.
			let entry = |row: usize, column: usize| {
				(0..3)
					.map(|axis| rotation[row][axis] * inverse_moments[axis] * rotation[column][axis])
					.sum::<f32>()
			};
			(
				rotation.map(|row| row[0] * offset[0] + row[1] * offset[1] + row[2] * offset[2]),
				[entry(0, 0), entry(1, 1), entry(2, 2)],
				[entry(0, 1), entry(0, 2), entry(1, 2)],
			)
		};

		let position = body.position.to_array();
		let with = |[x, y, z]: [f32; 3], last: f32| f32x4::from_array([x, y, z, last]);
		Self {
			center: with(
				[position[0] + offset[0], position[1] + offset[1], position[2] + offset[2]],
				body.inv_mass,
			),
			moments: with(moments, body.elasticity),
			products: with(products, body.friction),
			linear_velocity: with(body.linear_velocity.to_array(), 0.0),
		}
	}

	/// Returns the center of mass in world space.
	pub(super) fn center(&self) -> Point {
		let [x, y, z, _] = self.center.to_array();
		Point::new(x, y, z)
	}

	/// Returns the inverse mass, which is zero for a body that nothing moves.
	pub(super) fn inverse_mass(&self) -> f32 {
		self.center[3]
	}

	/// Returns how an impulse changes the body's spin. `moment` is the impulse's moment about the center of mass:
	/// the cross product of where it acts, from that center, with the impulse.
	pub(super) fn turn(&self, moment: Vector) -> Vector {
		let [xx, yy, zz, _] = self.moments.to_array();
		let [xy, xz, yz, _] = self.products.to_array();
		let [x, y, z] = moment.to_array();
		Vector::new(xx * x + xy * y + xz * z, xy * x + yy * y + yz * z, xz * x + yz * y + zz * z)
	}
}

/// The `WideSide` struct holds the state [`prepare`] reads from one body of each contact in a group.
struct WideSide {
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
	/// Reads the states of the bodies `sides` name. `motions` are the motions of those bodies, which tell how they
	/// spin before contacts change it.
	#[inline]
	fn gather(states: &[BodyState], motions: [&BodyMotion; WIDTH], sides: [&Side; WIDTH]) -> Self {
		let bodies = sides.map(|side| states[side.object]);
		let [x, y, z, inverse_mass] = columns(bodies.map(|body| body.center));
		let [xx, yy, zz, elasticity] = columns(bodies.map(|body| body.moments));
		let [xy, xz, yz, friction] = columns(bodies.map(|body| body.products));
		Self {
			center: WideVector { x, y, z },
			linear_velocity: coordinates(bodies.map(|body| body.linear_velocity)),
			angular_velocity: coordinates(motions.map(|motion| motion.0[3])),
			inverse_mass,
			inverse_inertia: [
				WideVector { x: xx, y: xy, z: xz },
				WideVector { x: xy, y: yy, z: yz },
				WideVector { x: xz, y: yz, z: zz },
			],
			elasticity,
			friction,
			point: WideVector::from_lanes(sides.map(|side| side.point.to_array())),
		}
	}
}

/// The `BodyMotion` struct holds one body's linear velocity, angular velocity, and shift while a step's contacts are
/// solved, in that order, each in the first three lanes of a vector. The shift is how far contacts move the body out
/// of overlap. They are followed by the angular velocity the body had before the step's contacts changed it, which
/// [`prepare`] reads with the rest and nothing changes. It fills one cache line, so reading a body never waits for
/// two.
///
/// Solving passes over the contacts several times, and every pass reads and changes the motions of the bodies in
/// contact. Keeping them apart from the bodies lets lanes solve different contacts of a batch at the same time: a
/// batch never has two contacts that change the same body, and each group of contacts reaches the motions of its
/// bodies through its [`Claim`].
///
/// Each body's values are three whole vectors, which a group of contacts loads and stores as such. Reading them one
/// number at a time took a quarter of a step of a large pile.
#[derive(Clone, Copy, Default)]
#[repr(align(64))]
pub(super) struct BodyMotion([f32x4; 4]);

impl BodyMotion {
	/// Returns the motion of a body that starts the step's contacts at these velocities, with no shift.
	fn starting(linear: f32x4, angular: f32x4) -> Self {
		Self([linear, angular, f32x4::splat(0.0), angular])
	}

	/// Returns the linear velocity, the angular velocity, and the shift, in that order.
	pub(super) fn parts(&self) -> [Vector; 3] {
		[self.0[0], self.0[1], self.0[2]].map(|vector| {
			let [x, y, z, _] = vector.to_array();
			Vector::new(x, y, z)
		})
	}

	/// Adds `linear` to the linear velocity and `angular` to the angular velocity.
	pub(super) fn push(&mut self, linear: Vector, angular: Vector) {
		// The shift, and the angular velocity from before the step, stay as they are.
		for (part, change) in self.0.iter_mut().zip([linear, angular]) {
			let [x, y, z] = change.to_array();
			*part += f32x4::from_array([x, y, z, 0.0]);
		}
	}

	/// Gives `body` these velocities, and shifts it out of overlap.
	fn give(&self, body: &mut PhysicsBody) {
		// A body that contacts cannot move is left as it is.
		if body.inv_mass == 0.0 {
			return;
		}
		let [linear, angular, shift] = self.parts();
		body.position += shift;
		body.linear_velocity = linear;
		// A contact far from the center of mass can spin a body faster than the simulation can follow, so the spin
		// is capped.
		let speed_squared = angular.length_squared();
		body.angular_velocity = if speed_squared > MAX_ANGULAR_SPEED * MAX_ANGULAR_SPEED {
			angular * (MAX_ANGULAR_SPEED / speed_squared.sqrt())
		} else {
			angular
		};
	}
}

/// The `WideMotion` struct holds the velocities and the shift of one body of each contact in a group.
#[derive(Clone, Copy)]
struct WideMotion {
	linear: WideVector,
	angular: WideVector,
	/// How far contacts move the body out of overlap this step, in meters.
	shift: WideVector,
}

/// Returns four bodies' vectors as one vector for each of their four lanes, with a lane for each body.
#[inline]
fn columns(bodies: [f32x4; WIDTH]) -> [f32x4; 4] {
	// The first of each pair holds the first lane of two bodies and then their second, and the other one their
	// third and then their fourth.
	let (low_first, low_second) = bodies[0].interleave(bodies[1]);
	let (high_first, high_second) = bodies[2].interleave(bodies[3]);
	[
		simd_swizzle!(low_first, high_first, [0, 1, 4, 5]),
		simd_swizzle!(low_first, high_first, [2, 3, 6, 7]),
		simd_swizzle!(low_second, high_second, [0, 1, 4, 5]),
		simd_swizzle!(low_second, high_second, [2, 3, 6, 7]),
	]
}

/// Returns the first three lanes of four bodies' vectors as one vector for each coordinate, with a lane for each
/// body.
#[inline]
fn coordinates(bodies: [f32x4; WIDTH]) -> WideVector {
	let [x, y, z, _] = columns(bodies);
	WideVector { x, y, z }
}

/// Returns one vector for each body from one vector for each coordinate, the reverse of [`coordinates`]. The fourth
/// lane of each is zero.
#[inline]
fn per_body(coordinates: WideVector) -> [f32x4; WIDTH] {
	// `xz` holds the x and z of one body and then of the next, and `y` their y, each followed by zero.
	let (low_xz, high_xz) = coordinates.x.interleave(coordinates.z);
	let (low_y, high_y) = coordinates.y.interleave(f32x4::splat(0.0));
	let (first, second) = low_xz.interleave(low_y);
	let (third, fourth) = high_xz.interleave(high_y);
	[first, second, third, fourth]
}

/// Returns the motions of the two bodies of each contact of a group as one vector for each coordinate.
#[inline]
fn gather(motions: [[&BodyMotion; WIDTH]; 2]) -> [WideMotion; 2] {
	motions.map(|bodies| WideMotion {
		linear: coordinates(bodies.map(|motion| motion.0[0])),
		angular: coordinates(bodies.map(|motion| motion.0[1])),
		shift: coordinates(bodies.map(|motion| motion.0[2])),
	})
}

/// Writes `motions` to the bodies of the group of `claim` that contacts can move.
#[inline]
fn scatter(bodies: &mut BatchOwned<'_, BodyMotion>, claim: &Claim<WIDTH>, motions: [WideMotion; 2]) {
	let motions = motions.map(|motion| [per_body(motion.linear), per_body(motion.angular), per_body(motion.shift)]);
	bodies.write(claim, |side, place, body| {
		let [linear, angular, shift] = &motions[side];
		// The angular velocity from before the contacts stays as it is.
		body.0[..3].copy_from_slice(&[linear[place], angular[place], shift[place]]);
	});
}

/// The fastest a contact may leave a body spinning, in radians per second.
const MAX_ANGULAR_SPEED: f32 = 30.0;

/// The number of times [`solve`] passes over a step's contacts.
///
/// One pass lets each contact react to its own bodies. A body in several contacts, such as one under a stack, only
/// carries its load once the contacts have reacted to each other, which takes more passes the taller the stack.
pub(crate) const ITERATIONS: usize = 4;

/// The speed, in meters per second, bodies must close at before they bounce.
///
/// Slower contacts are bodies that rest on each other, which would jitter if gravity's pull bounced every step.
const BOUNCE_SPEED: f32 = 1.0;

/// The share of an overlap that contacts close in one step.
///
/// Closing all of it at once makes stacked bodies jitter, because each contact undoes part of its neighbor's shift.
const OVERLAP_SHARE: f32 = 0.2;

/// The overlap, in meters, that contacts leave alone, so that resting bodies stay in contact from step to step.
const OVERLAP_SLACK: f32 = 0.005;

/// The farthest, in meters, that one contact shifts its bodies apart in one step.
///
/// Bodies that start deep inside each other then drift apart instead of jumping.
const MAX_SHIFT: f32 = 0.05;

/// The `WideContact` struct holds what [`solve`] needs to know about up to [`WIDTH`] contacts, worked out once so
/// that every pass only reads and writes velocities.
///
/// [`prepare`] builds it. Each contact acts along three directions: its normal, where the bodies can only push
/// each other apart, and two directions across it, where friction holds them.
pub(crate) struct WideContact {
	/// The slots of the two bodies of each contact, and the right to change the motion of the movable ones while
	/// the contacts' batch is solved.
	bodies: Claim<WIDTH>,
	/// Which of the contacts between its two bodies each contact is.
	corners: [u8; WIDTH],
	/// The lanes that hold a contact, one bit per lane.
	in_use: u64,
	/// The normal, from the first body toward the second, and the two friction directions.
	directions: [WideVector; 3],
	/// For each body and direction, how the body's spin moves the contact point along the direction.
	arms: [[WideVector; 3]; 2],
	/// For each body and direction, how a unit impulse along the direction changes the body's spin.
	turns: [[WideVector; 3]; 2],
	inverse_mass: [f32x4; 2],
	/// For each direction, the impulse that changes the bodies' closing speed along it by one. Zero for a contact
	/// that cannot be solved, which leaves its bodies alone.
	masses: [f32x4; 3],
	friction: f32x4,
	/// The speed the bodies should part at: their bounce when they touch, and for bodies that only meet later in the
	/// step, the negative of the speed they may still close at.
	parting: f32x4,
	/// How far the bodies should be shifted apart this step, in meters.
	overlap: f32x4,
	/// The share of a shift each body takes, by how easily it moves. Zero for a contact that cannot be solved.
	shares: [f32x4; 2],
	/// The impulse applied so far along each direction, and then the shift applied so far, kept so that a later
	/// pass can take some back.
	applied: [[AtomicU32; WIDTH]; 4],
}

impl WideContact {
	/// Returns the two bodies of each contact, and which of their contacts it is, with the impulse that holds them
	/// apart, for [`prepare`] to start from in the next step.
	fn pushes(&self) -> impl Iterator<Item = ([usize; 2], u8, f32)> + '_ {
		(0..WIDTH).filter(|lane| (self.in_use >> lane) & 1 == 1).map(|lane| {
			let push = f32::from_bits(self.applied[0][lane].load(Ordering::Relaxed));
			(
				[self.bodies.entry(0, lane), self.bodies.entry(1, lane)],
				self.corners[lane],
				push,
			)
		})
	}
}

/// Works out what [`solve`] needs for the contacts `group` names in `contacts`, from the `states` of their bodies.
///
/// `group` holds at most [`WIDTH`] contacts, and `bodies` is the batches' claim on the motions of their bodies, in
/// `motions`. `dt` is the length of the step in seconds. `last_push` returns the impulse that held two bodies apart
/// in the last step, or zero: resting bodies need about the same again, so it is applied to `motions` right away and
/// [`solve`] only has to adjust it. Pass the result to [`solve`] once per pass.
fn prepare(
	states: &[BodyState],
	motions: &mut BatchOwned<'_, BodyMotion>,
	contacts: &Contacts,
	bodies: Claim<WIDTH>,
	group: &[u32],
	dt: f32,
	last_push: impl Fn([usize; 2], u8) -> f32,
) -> WideContact {
	debug_assert!(
		!group.is_empty() && group.len() <= WIDTH,
		"Contact group size is invalid. The most likely cause is a batch that was not split into groups of {WIDTH}."
	);
	// Unused lanes repeat the last contact so that every lane reads real bodies. Nothing is written for them.
	let contacts: [&Contact; WIDTH] = std::array::from_fn(|lane| contacts.get(group[lane.min(group.len() - 1)] as usize));
	let in_use = mask32x4::from_array(std::array::from_fn(|lane| lane < group.len()));
	let sides = [contacts.map(|contact| &contact.a), contacts.map(|contact| &contact.b)];
	let before = motions.get(&bodies);
	let [a, b] = [
		WideSide::gather(states, before[0], sides[0]),
		WideSide::gather(states, before[1], sides[1]),
	];
	// Start the bodies off from where they are, before the claim on them moves into what is prepared.
	let mut started = gather(before);
	let (zero, one) = (f32x4::splat(0.0), f32x4::splat(1.0));

	// The normal must point from A toward B.
	let normal = WideVector::from_lanes(contacts.map(|contact| contact.normal.into_vector().to_array()));
	let flipped = (b.center - a.center).dot(normal).simd_lt(zero);
	let normal = normal * flipped.select(f32x4::splat(-1.0), one);
	// Any two directions across the normal serve for friction. Crossing the normal with the axis it leans on least
	// gives one that is never close to zero length.
	let leans_on_x = normal.x.abs().simd_gt(f32x4::splat(0.57));
	let axis = WideVector {
		x: leans_on_x.select(zero, one),
		y: leans_on_x.select(one, zero),
		z: zero,
	};
	let across = normal.cross(axis);
	let across = across * (one / across.dot(across).sqrt());
	let directions = [normal, across, normal.cross(across)];

	let inverse_mass_sum = a.inverse_mass + b.inverse_mass;
	let (a_arm, b_arm) = (a.point - a.center, b.point - b.center);
	let arms = [
		directions.map(|direction| a_arm.cross(direction)),
		directions.map(|direction| b_arm.cross(direction)),
	];
	let turns = [
		arms[0].map(|arm| transform(&a.inverse_inertia, arm)),
		arms[1].map(|arm| transform(&b.inverse_inertia, arm)),
	];
	// A contact whose bodies cannot take an impulse, or whose mass or inertia data is invalid, changes nothing.
	let movable = in_use & inverse_mass_sum.simd_ne(zero);
	let mut solvable = [movable; 3];
	let masses = std::array::from_fn(|direction| {
		let resistance =
			inverse_mass_sum + arms[0][direction].dot(turns[0][direction]) + arms[1][direction].dot(turns[1][direction]);
		solvable[direction] &= resistance.is_finite() & resistance.simd_gt(f32x4::splat(f32::EPSILON));
		solvable[direction].select(one / resistance, zero)
	});

	let closing_speed = (a.linear_velocity - b.linear_velocity).dot(normal) + a.angular_velocity.dot(arms[0][0])
		- b.angular_velocity.dot(arms[1][0]);
	// Bodies that touch where they are bounce, and are shifted apart if they overlap. Bodies that only meet later in
	// the step are held to the closing speed that brings them together as the step ends, so that a fast body does not
	// pass through another one between two steps.
	let time = f32x4::from_array(contacts.map(|contact| contact.toi));
	let touching = time.simd_le(zero);
	let bounces = touching & closing_speed.simd_gt(f32x4::splat(BOUNCE_SPEED));
	let depth = f32x4::from_array(contacts.map(|contact| contact.depth));
	// A contact with a negative depth is between bodies that are that far short of touching there, as they can be at
	// a corner of the patch they touch over, or when they were moved up to each other along their paths. They may
	// close that gap in this step, after what they close until the time of the contact, and no more.
	let gap = (zero - depth).simd_max(zero);
	let allowance = (closing_speed.simd_max(zero) * time + gap) / f32x4::splat(dt);
	let overlapping = touching & movable;
	let overlap =
		((depth - f32x4::splat(OVERLAP_SLACK)).simd_max(zero) * f32x4::splat(OVERLAP_SHARE)).simd_min(f32x4::splat(MAX_SHIFT));

	let corners = contacts.map(|contact| contact.corner);
	let pushes = f32x4::from_array(std::array::from_fn(|lane| {
		if lane < group.len() {
			last_push([sides[0][lane].object, sides[1][lane].object], corners[lane])
		} else {
			0.0
		}
	}));
	let pushes = solvable[0].select(pushes.simd_max(zero), zero);
	let contact = WideContact {
		bodies,
		corners,
		in_use: in_use.to_bitmask(),
		directions,
		arms,
		turns,
		inverse_mass: [a.inverse_mass, b.inverse_mass],
		masses,
		friction: a.friction * b.friction,
		parting: bounces.select(a.elasticity * b.elasticity * closing_speed, zero) - allowance,
		overlap: overlapping.select(overlap, zero),
		shares: [
			movable.select(a.inverse_mass / inverse_mass_sum, zero),
			movable.select(b.inverse_mass / inverse_mass_sum, zero),
		],
		applied: [
			pushes.to_array().map(|push| AtomicU32::new(push.to_bits())),
			Default::default(),
			Default::default(),
			Default::default(),
		],
	};

	// The push that held the bodies apart in the last step is where the passes start from.
	push(&contact, &mut started, 0, pushes);
	scatter(motions, &contact.bodies, started);
	contact
}

/// Passes once over the contacts of `contact`, changing the motion of their bodies in `bodies`.
///
/// Along the normal, the bodies are pushed until they close no faster than their contact allows: not at all when
/// they touch, apart from a bounce, and for bodies that only meet later in the step, just fast enough to meet as it
/// ends. The push only
/// ever holds bodies apart: when they are already parting, a later pass takes back what an earlier one applied
/// instead of pulling them together. Across the normal, friction slows their sliding by at most the push times
/// their friction. Bodies that overlap are also shifted apart, which moves them without changing their
/// velocities, so closing an overlap does not throw them.
fn solve(contact: &WideContact, bodies: &mut BatchOwned<'_, BodyMotion>) {
	let zero = f32x4::splat(0.0);
	let mut motions = gather(bodies.get(&contact.bodies));
	let mut applied = contact
		.applied
		.each_ref()
		.map(|lanes| f32x4::from_array(lanes.each_ref().map(|value| f32::from_bits(value.load(Ordering::Relaxed)))));

	let pushed = (applied[0] + (closing_speed(contact, &motions, 0) + contact.parting) * contact.masses[0]).simd_max(zero);
	push(contact, &mut motions, 0, pushed - applied[0]);
	applied[0] = pushed;

	// Friction acts in a plane, so the two directions are limited together, by length.
	let wanted =
		[1, 2].map(|direction| applied[direction] + closing_speed(contact, &motions, direction) * contact.masses[direction]);
	let limit = contact.friction * applied[0];
	let length = (wanted[0] * wanted[0] + wanted[1] * wanted[1]).sqrt();
	let scale = length.simd_gt(limit).select(limit / length, f32x4::splat(1.0));
	for (wanted, direction) in wanted.into_iter().zip([1, 2]) {
		let held = wanted * scale;
		push(contact, &mut motions, direction, held - applied[direction]);
		applied[direction] = held;
	}

	// The bodies' shifts so far may already have closed part of the overlap, or opened it further.
	let normal = contact.directions[0];
	let closed = (motions[1].shift - motions[0].shift).dot(normal);
	let shifted = (applied[3] + contact.overlap - closed).simd_max(zero);
	let shift = shifted - applied[3];
	motions[0].shift = motions[0].shift - normal * (shift * contact.shares[0]);
	motions[1].shift = motions[1].shift + normal * (shift * contact.shares[1]);
	applied[3] = shifted;

	for (lanes, applied) in contact.applied.iter().zip(applied) {
		for (lane, applied) in lanes.iter().zip(applied.to_array()) {
			lane.store(applied.to_bits(), Ordering::Relaxed);
		}
	}
	scatter(bodies, &contact.bodies, motions);
}

/// Returns how fast the two bodies of each contact close along one of its directions.
#[inline]
fn closing_speed(contact: &WideContact, motions: &[WideMotion; 2], direction: usize) -> f32x4 {
	(motions[0].linear - motions[1].linear).dot(contact.directions[direction])
		+ motions[0].angular.dot(contact.arms[0][direction])
		- motions[1].angular.dot(contact.arms[1][direction])
}

/// Applies `impulse` along one of the contacts' directions: against it on the first body and along it on the second.
#[inline]
fn push(contact: &WideContact, motions: &mut [WideMotion; 2], direction: usize, impulse: f32x4) {
	let along = contact.directions[direction];
	motions[0].linear = motions[0].linear - along * (impulse * contact.inverse_mass[0]);
	motions[0].angular = motions[0].angular - contact.turns[0][direction] * impulse;
	motions[1].linear = motions[1].linear + along * (impulse * contact.inverse_mass[1]);
	motions[1].angular = motions[1].angular + contact.turns[1][direction] * impulse;
}

/// The `LastPushes` struct keeps the impulse that held each pair of bodies apart at each of their contacts in the
/// last step, by [`pair_key`]. Resting bodies need about the same impulse again, so the next step's solve starts
/// from it.
///
/// Every step replaces all of them, which took a tenth of a step of a large pile when one lane did it. This is a
/// table that any number of lanes can add to at the same time: a lane claims a slot by stamping it with the step.
/// The stamp also tells which impulses are the last step's, so the table is never emptied.
///
/// Every step, call [`Self::turn`] with the number of contacts, read the last step's impulses with [`Self::get`],
/// and, once nothing reads them any more, [`Self::insert`] the step's own.
#[derive(Default)]
struct LastPushes {
	/// The slots. Their number is a power of two, or zero.
	slots: Vec<PushSlot>,
	/// The stamp of the impulses that [`Self::get`] reads. [`Self::insert`] stamps with the one after it.
	read: u32,
}

/// The `PushSlot` struct holds one impulse of [`LastPushes`] next to its key, so that finding an impulse reads one
/// cache line.
#[derive(Default)]
struct PushSlot {
	key: AtomicU64,
	/// The impulse, as the bits of an `f32`.
	push: AtomicU32,
	/// The step whose impulse the slot holds. A slot with another stamp than the one asked for is empty.
	stamp: AtomicU32,
}

impl LastPushes {
	/// Returns the slots, from the one where the search for `key` starts and around the table to the one before it.
	///
	/// An impulse is in the first slot from there that has no other impulse of its step, so few slots are searched
	/// while at least half of them are empty.
	fn slots(&self, key: u64) -> impl Iterator<Item = &PushSlot> {
		let slots = self.slots.len();
		// The product spreads keys that differ in few bits over the table, and its top bits pick the slot.
		let first = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 1) as usize >> (usize::BITS - 1 - slots.trailing_zeros());
		(0..slots).map(move |step| &self.slots[(first + step) & (slots - 1)])
	}

	/// Returns the impulse the last step kept for `key`, or zero.
	#[inline]
	fn get(&self, key: u64) -> f32 {
		for slot in self.slots(key) {
			if slot.stamp.load(Ordering::Relaxed) != self.read {
				break;
			}
			if slot.key.load(Ordering::Relaxed) == key {
				return f32::from_bits(slot.push.load(Ordering::Relaxed));
			}
		}
		0.0
	}

	/// Keeps `push` as this step's impulse for `key`, which no other call may be given in this step. Lanes can each
	/// insert impulses at the same time, as long as nothing calls [`Self::get`] meanwhile.
	fn insert(&self, key: u64, push: f32) {
		let stamp = self.read + 1;
		for slot in self.slots(key) {
			// The lane that changes a slot's stamp to this step's has the slot. A slot that already has it is taken.
			let found = slot.stamp.load(Ordering::Relaxed);
			let claim = |found| {
				slot.stamp
					.compare_exchange(found, stamp, Ordering::Relaxed, Ordering::Relaxed)
			};
			if found != stamp && claim(found).is_ok() {
				slot.key.store(key, Ordering::Relaxed);
				return slot.push.store(push.to_bits(), Ordering::Relaxed);
			}
		}
		debug_assert!(
			false,
			"Contact impulse table is full. The most likely cause is a step with more contacts than the table was turned for."
		);
	}

	/// Starts a step with `count` contacts: the impulses inserted during the last step become the ones
	/// [`Self::get`] reads, the ones before them are forgotten, and there is room to insert `count` more.
	fn turn(&mut self, count: usize) {
		self.read += 1;
		// Stamps would repeat after this many steps, so the table starts over, without impulses for one step.
		if self.read == u32::MAX {
			self.slots.iter_mut().for_each(|slot| *slot.stamp.get_mut() = 0);
			self.read = 1;
		}
		// At least half of the table stays empty.
		let slots = (count * 2).next_power_of_two();
		if slots <= self.slots.len() {
			return;
		}
		let kept = std::mem::take(&mut self.slots);
		self.slots.resize_with(slots, Default::default);
		// The impulses to read move to the larger table as if they were inserted in the step before this one.
		self.read -= 1;
		for mut slot in kept {
			if *slot.stamp.get_mut() == self.read + 1 {
				self.insert(*slot.key.get_mut(), f32::from_bits(*slot.push.get_mut()));
			}
		}
		self.read += 1;
	}

	/// Sets to zero the impulses to read whose key `keep` returns `false` for, which is what a pair without one
	/// starts from.
	fn retain(&mut self, keep: impl Fn(u64) -> bool) {
		for slot in &mut self.slots {
			if *slot.stamp.get_mut() == self.read && !keep(*slot.key.get_mut()) {
				*slot.push.get_mut() = 0.0f32.to_bits();
			}
		}
	}
}

/// The `ContactSolver` struct takes a physics step's contacts from detection to the bodies' new velocities.
///
/// Every step, call [`Self::reserve`], and have every lane give it the bodies as they are before contacts change
/// them, with [`Self::take_bodies`]. Call [`Self::begin`] with the step's contacts. Then get every batch ready, in
/// order: the first ones with [`Self::prepare_batch`], whose groups lanes share in a stage for each batch, and the
/// others with [`Self::prepare_rest`]. Pass over the groups [`ITERATIONS`] times, batch after batch in the same
/// order, with [`Self::solve`] and [`Self::solve_rest`], and call [`Self::end_passes`]. Then give every body its
/// velocities with [`Self::solved`], and call [`Self::keep_pushes`] for every lane's groups and
/// [`Self::keep_rest_pushes`].
#[derive(Default)]
pub(crate) struct ContactSolver {
	/// The step's contacts, sorted into batches that can each be solved in any order.
	batches: PairBatches,
	/// What [`Self::prepare_rest`] worked out, in groups: the groups of each of its batches in turn, then one group
	/// for each contact that fit in no batch. They share bodies, so one lane passes over them in order.
	rest: Vec<WideContact>,
	/// What the step's contacts need to know about each body, as it is before they change it, by body slot. Lanes
	/// fill it side by side, each for its own range of the slots.
	states: LaneRanges<LaneCells<BodyState>>,
	/// The velocities of the bodies while the step's contacts are solved, by body slot. Lanes start them off like
	/// `states`.
	motions: LaneRanges<LaneCells<BodyMotion>>,
	/// The impulse that held each pair of bodies apart at each of their contacts in the last step.
	pushes: LastPushes,
	/// The slots whose body was replaced since the last step. Their pairs' impulses belonged to other bodies.
	replaced: Vec<usize>,
	/// Whether the step has contacts or constraints, which change the bodies' motions.
	active: bool,
}

/// The `TakenBodies` struct lets one lane give a [`ContactSolver`] the bodies in its range of the body slots.
/// [`ContactSolver::take_bodies`] returns it.
pub(crate) struct TakenBodies<'a> {
	states: RangeOwned<'a, LaneCells<BodyState>>,
	motions: RangeOwned<'a, LaneCells<BodyMotion>>,
	/// The first slot of the lane's range.
	first: usize,
}

impl TakenBodies<'_> {
	/// Takes what the step's contacts need of `body`, the one in `slot`: where it is, how easily it moves and
	/// turns, and its velocities. Call it once the body is as the step's contacts should find it.
	///
	/// # Panics
	///
	/// Panics if `slot` is outside the lane's range of the body slots.
	pub(crate) fn take(&mut self, slot: usize, body: &PhysicsBody) {
		let state = BodyState::of(body);
		let [x, y, z] = body.angular_velocity.to_array();
		let place = slot - self.first;
		self.states[place] = state;
		// Every body starts the step's contacts at its own velocities.
		self.motions[place] = BodyMotion::starting(state.linear_velocity, f32x4::from_array([x, y, z, 0.0]));
	}
}

/// The `SolvedBodies` struct holds the velocities that the passes over a step's contacts left every body with.
/// [`ContactSolver::solved`] returns it.
pub(crate) struct SolvedBodies<'a> {
	/// The motion of every body, by body slot. None for a step without contacts, where no pass changed a body.
	motions: Option<&'a [BodyMotion]>,
}

impl SolvedBodies<'_> {
	/// Gives `body`, the one in `slot`, the velocities the passes left it with, and shifts it out of overlap.
	pub(crate) fn give(&self, slot: usize, body: &mut PhysicsBody) {
		// Without contacts each body keeps the velocities it has.
		if let Some(motions) = self.motions {
			motions[slot].give(body);
		}
	}
}

impl ContactSolver {
	/// Forgets the impulses of the pairs that include the body in `slot`. Call it when another body takes the slot,
	/// so that the new body does not start from impulses the old one needed.
	pub(crate) fn forget(&mut self, slot: usize) {
		self.replaced.push(slot);
	}

	/// Makes room for what the solver keeps of every body slot below `slots`, and gives each of `lanes` lanes of
	/// `alley` a range of those slots to fill with [`Self::take_bodies`].
	pub(crate) fn reserve(&mut self, slots: usize, alley: AlleyId, lanes: usize) {
		self.states.get_mut().resize(slots, BodyState::default());
		self.motions.get_mut().resize(slots, BodyMotion::default());
		self.states.lend(alley);
		self.motions.lend(alley);
		self.batches.lend(alley);
		self.states.split(lanes);
		self.motions.split(lanes);
	}

	/// Returns what `lane` takes the bodies in its range of the body slots with. Have every lane take every body in
	/// its range before [`Self::begin`].
	///
	/// Each body is worked out on its own, so lanes can each take their bodies at the same time.
	pub(crate) fn take_bodies(&self, lane: &ConcreteLane<'_>) -> TakenBodies<'_> {
		let states = self.states.own(lane);
		TakenBodies {
			first: states.range().start,
			states,
			motions: self.motions.own(lane),
		}
	}

	/// Sorts a step's `contacts` into batches, ready for `lanes` lanes to prepare them. Call it once every lane has
	/// taken its bodies, with [`Self::take_bodies`]. `constrained` is whether the step also changes the motions of
	/// bodies through [`Self::pair_mut`], as constraints do.
	///
	/// The contacts of a step are solved together, so no order among them is more correct than another. Batching
	/// picks one that lets a batch's contacts be solved at the same time.
	pub(crate) fn begin(&mut self, contacts: &Contacts, lanes: usize, constrained: bool) {
		self.rest.clear();
		// The impulses kept in the last step are the ones this step starts from.
		self.pushes.turn(contacts.len());
		if !self.replaced.is_empty() {
			self.replaced.sort_unstable();
			let replaced = |slot: u64| self.replaced.binary_search(&(slot as usize)).is_ok();
			self.pushes
				.retain(|pair| pair_slots(pair).into_iter().all(|slot| !replaced(slot)));
			self.replaced.clear();
		}
		self.active = contacts.len() != 0 || constrained;
		// The contacts come with the bodies each one changes, so this reads neither the contacts nor the bodies.
		self.batches.rebuild(self.states.get_mut().len(), contacts.bodies());
		// From here until the passes end, every lane reads what was taken of every body, and only reaches the motions
		// of the bodies in its groups of a batch.
		self.states.join();
		self.motions.split_by(&self.batches, lanes);
	}

	/// Returns the number of batches the step's contacts were sorted into. Batches fill from the first one, so the
	/// large ones lead.
	pub(crate) fn batches(&self) -> usize {
		self.batches.len()
	}

	/// Returns the number of contacts in `batch`.
	pub(crate) fn batch_contacts(&self, batch: usize) -> usize {
		self.batches.batch(batch).len()
	}

	/// Returns whether any contact is left for [`Self::prepare_rest`] once the first `batches` batches are prepared.
	pub(crate) fn has_rest(&self, batches: usize) -> bool {
		batches < self.batches.len() || !self.batches.leftover().is_empty()
	}

	/// Works out what solving this lane's share of the groups of the stage's batch needs, and adds it to `prepared`
	/// in order. `dt` is the length of the step in seconds.
	///
	/// The groups of one batch share no movable body, so every lane prepares its share in the same stage, whose key
	/// is the batch. Batches have to follow each other in order, because preparing a group starts its bodies off
	/// with the last step's impulses.
	pub(crate) fn prepare_batch(&self, contacts: &Contacts, dt: f32, stage: &Stage<'_, '_>, prepared: &mut Vec<WideContact>) {
		let (states, mut motions) = (self.states.all(), self.motions.batch(stage));
		let last_push = |pair, corner| self.pushes.get(pair_key(pair, corner));
		let groups = self.batches.share(stage);
		prepared.reserve(groups.size_hint().0);
		for (bodies, group) in groups {
			prepared.push(prepare(states, &mut motions, contacts, bodies, group, dt, last_push));
		}
	}

	/// Prepares the batches from `first_batch` on, and then the contacts that fit in no batch, for one lane to
	/// solve with [`Self::solve_rest`].
	pub(crate) fn prepare_rest(&mut self, contacts: &Contacts, dt: f32, first_batch: usize) {
		let (states, mut motions) = (self.states.all(), self.motions.batch_mut());
		let last_push = |pair, corner| self.pushes.get(pair_key(pair, corner));
		for batch in first_batch..self.batches.len() {
			for (bodies, group) in self.batches.batch_groups(batch) {
				self.rest
					.push(prepare(states, &mut motions, contacts, bodies, group, dt, last_push));
			}
		}
		// Contacts that fit in no batch share bodies, so each one is a group of its own and they are solved in turn.
		for (bodies, group) in self.batches.leftover_groups() {
			self.rest
				.push(prepare(states, &mut motions, contacts, bodies, group, dt, last_push));
		}
	}

	/// Passes once over `groups`, which this lane prepared of the stage's batch with [`Self::prepare_batch`]. Every
	/// lane passes over its groups of the batch in the same stage; batches, and [`Self::solve_rest`], have to follow
	/// each other.
	pub(crate) fn solve(&self, stage: &Stage<'_, '_>, groups: &[WideContact]) {
		let mut motions = self.motions.batch(stage);
		for group in groups {
			solve(group, &mut motions);
		}
	}

	/// Passes once over the groups [`Self::prepare_rest`] prepared.
	pub(crate) fn solve_rest(&mut self) {
		let mut motions = self.motions.batch_mut();
		for group in &self.rest {
			solve(group, &mut motions);
		}
	}

	/// Returns what was taken of the body in `slot` before the step's contacts changed it. Call it after
	/// [`Self::begin`].
	pub(super) fn state(&self, slot: usize) -> BodyState {
		self.states.all()[slot]
	}

	/// Returns the motions of the two bodies in `slots`, to change them between the passes over the contacts, in a
	/// stage that one lane runs. Tell [`Self::begin`] about it, so that the bodies are given what is changed.
	///
	/// # Panics
	///
	/// Panics if both slots are the same one, or one is not a body slot.
	pub(super) fn pair_mut(&mut self, slots: [usize; 2]) -> [&mut BodyMotion; 2] {
		self.motions.get_mut().get_disjoint_mut(slots).expect(
			"Body slots do not name two bodies. The most likely cause is a constraint between a body and itself, or one made ready before a body was removed.",
		)
	}

	/// Lets every lane read the motions the passes left, which [`Self::solved`] does. Call it once the passes are
	/// done.
	pub(crate) fn end_passes(&mut self) {
		self.motions.join();
	}

	/// Returns the velocities the passes left every body with. Call it once the passes are done, and
	/// [`SolvedBodies::give`] for every body.
	///
	/// Each body is given its own, so lanes can each finish their bodies at the same time.
	pub(crate) fn solved(&self) -> SolvedBodies<'_> {
		SolvedBodies {
			motions: self.active.then(|| &self.motions.all()[..]),
		}
	}

	/// Keeps the impulses of the contacts in `groups`, which [`Self::prepare_batch`] prepared, for the next step to
	/// start from. Call it once the passes are done.
	///
	/// Lanes can each keep the impulses of their own groups at the same time.
	pub(crate) fn keep_pushes(&self, groups: &[WideContact]) {
		for (pair, corner, push) in groups.iter().flat_map(WideContact::pushes) {
			self.pushes.insert(pair_key(pair, corner), push);
		}
	}

	/// Keeps the impulses of the contacts that [`Self::prepare_rest`] prepared, as [`Self::keep_pushes`] does for
	/// the others.
	pub(crate) fn keep_rest_pushes(&self) {
		self.keep_pushes(&self.rest);
	}
}

/// The bits of a [`pair_key`] that hold one body slot, which leaves two bits for the contact's number.
const SLOT_BITS: u32 = 31;

/// Returns one value for one of the contacts between a pair of body slots, the same whichever body the contact
/// lists first. `corner` is which of the pair's contacts it is.
fn pair_key([first, second]: [usize; 2], corner: u8) -> u64 {
	debug_assert!(
		first.max(second) >> SLOT_BITS == 0 && corner < 4,
		"Contact key is out of range. The most likely cause is a pair with more than four contacts."
	);
	(first.min(second) as u64) << (SLOT_BITS + 2) | (first.max(second) as u64) << 2 | u64::from(corner)
}

/// Returns the two body slots of a [`pair_key`].
fn pair_slots(key: u64) -> [u64; 2] {
	[key >> (SLOT_BITS + 2), (key >> 2) & ((1 << SLOT_BITS) - 1)]
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn impulses_are_read_in_the_step_after_they_are_kept() {
		let mut pushes = LastPushes::default();
		let keys = (0..100usize).map(|body| pair_key([body, body + 1], (body % 4) as u8));
		let holds = |pushes: &LastPushes, kept: &dyn Fn(usize) -> f32| {
			(keys.clone().enumerate()).all(|(index, key)| pushes.get(key) == kept(index))
		};

		pushes.turn(100);
		for (index, key) in keys.clone().enumerate() {
			pushes.insert(key, index as f32);
		}
		// The step that keeps an impulse does not read it.
		assert!(holds(&pushes, &|_| 0.0));

		// The next step does, also when it needs a larger table.
		pushes.turn(10_000);
		assert!(holds(&pushes, &|index| index as f32));
		// A pair without an impulse starts from none.
		assert_eq!(pushes.get(pair_key([3, 500], 0)), 0.0);

		pushes.retain(|key| pair_slots(key)[0] >= 50);
		assert!(holds(&pushes, &|index| if index < 50 { 0.0 } else { index as f32 }));

		// A step keeps only the impulses of its own contacts.
		pushes.insert(pair_key([7, 8], 3), 5.0);
		pushes.turn(1);
		assert!(holds(&pushes, &|index| if index == 7 { 5.0 } else { 0.0 }));
	}

	#[test]
	fn threads_keep_impulses_at_the_same_time() {
		let mut pushes = LastPushes::default();
		pushes.turn(4_000);
		std::thread::scope(|scope| {
			for thread in 0..4usize {
				let pushes = &pushes;
				scope.spawn(move || {
					for body in (thread..4_000).step_by(4) {
						pushes.insert(pair_key([body, body + 7], 1), body as f32);
					}
				});
			}
		});
		pushes.turn(0);

		assert!((0..4_000usize).all(|body| pushes.get(pair_key([body, body + 7], 1)) == body as f32));
	}

	#[test]
	fn every_contact_of_a_pair_has_a_key_of_its_own() {
		let keys: Vec<u64> = (0..4).map(|corner| pair_key([7, 3], corner)).collect();

		for (corner, key) in keys.iter().enumerate() {
			assert_eq!(keys.iter().filter(|other| *other == key).count(), 1);
			// The same whichever body is listed first, and its slots can be read back.
			assert_eq!(*key, pair_key([3, 7], corner as u8));
			assert_eq!(pair_slots(*key), [3, 7]);
		}
		assert_ne!(pair_key([7, 3], 0), pair_key([7, 4], 0));

		let last = (1 << SLOT_BITS) - 1;
		assert_eq!(pair_slots(pair_key([last, last - 1], 3)), [last as u64 - 1, last as u64]);
	}
}
