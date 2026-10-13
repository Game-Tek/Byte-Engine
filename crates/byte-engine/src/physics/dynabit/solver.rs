//! Contact response for Dynabit: which contacts can be solved together, and the velocity changes that solve them.
//!
//! [`ContactSolver`] takes a step's contacts through it. [`ContactBatches`] sorts simultaneous contacts into batches
//! that touch every movable body at most once. The contacts of one batch do not depend on each other, so they are
//! solved several at a time and lanes can share a batch. [`prepare`] works out what each group of contacts needs,
//! and [`solve`] passes over a group, changing the [`Velocities`] of its bodies.

use std::{
	cell::UnsafeCell,
	ops::{Add, Mul, Range, Sub},
	simd::{
		Select as _, StdFloat as _,
		cmp::{SimdPartialEq as _, SimdPartialOrd as _},
		f32x4, mask32x4,
		num::SimdFloat as _,
		simd_swizzle,
	},
	sync::atomic::{AtomicU32, Ordering},
};

use math::Vector;
use utils::{StableVec, hash::HashMap};

use crate::physics::dynabit::{
	body::PhysicsBody,
	contact::{Contact, Contacts, Side},
};

/// The number of contacts one [`WideContact`] holds.
const WIDTH: usize = 4;

/// The number of batches [`ContactBatches`] can form, one for each bit of a body's record of the batches it is in.
const BATCH_LIMIT: usize = u64::BITS as usize;

/// The `ContactBatches` struct sorts a step's simultaneous contacts into batches in which no movable body appears
/// twice, so the contacts of one batch can be solved in any order, or at the same time, with the same result.
///
/// Call [`Self::rebuild`] once the step's contacts are known. In every pass, solve [`Self::batch`] `0` to
/// [`Self::len`] in order, and then [`Self::leftover`] one contact at a time.
#[derive(Default)]
struct ContactBatches {
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
	fn rebuild(&mut self, body_count: usize, contacts: impl Iterator<Item = [Option<usize>; 2]>) {
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
	fn len(&self) -> usize {
		self.ends.len()
	}

	/// Returns the contacts of one batch, in the order they were given to [`Self::rebuild`].
	fn batch(&self, batch: usize) -> &[u32] {
		let start = if batch == 0 { 0 } else { self.ends[batch - 1] };
		&self.order[start..self.ends[batch]]
	}

	/// Returns the contacts that fit in no batch, in the order they were given to [`Self::rebuild`].
	///
	/// Solve them one at a time after every batch, because they share bodies with each other.
	fn leftover(&self) -> &[u32] {
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
		// Each component is read on its own: mapping over the components to build the four vectors was not inlined.
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
		let inverse_moments = vector(PhysicsBody::inverse_moments);
		let inverse_inertia = rotation.map(|row| {
			let scaled = WideVector {
				x: row.x * inverse_moments.x,
				y: row.y * inverse_moments.y,
				z: row.z * inverse_moments.z,
			};
			transform(&rotation, scaled)
		});

		Self {
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
}

/// The `Velocities` struct holds the linear and angular velocity of every body while a step's contacts are solved,
/// along with how far contacts shift the body out of overlap.
///
/// Solving passes over the contacts several times, and every pass reads and changes the velocities of the bodies in
/// contact. Keeping them here, apart from the bodies, lets lanes solve different contacts of a batch at the same
/// time: lanes write them through a shared reference, and a batch never has two contacts that change the same body.
///
/// Each body's values are three whole vectors, which a group of contacts loads and stores as such. Reading them one
/// number at a time took a quarter of a step of a large pile.
///
/// Fill it with [`Self::load`], pass it to [`solve`], and write the result back with [`Self::store`].
#[derive(Default)]
struct Velocities {
	/// Each body's motion, indexed by body slot.
	bodies: Vec<BodyMotion>,
}

/// The `BodyMotion` struct holds one body's linear velocity, angular velocity, and shift, in that order, each in the
/// first three lanes of a vector. It fills one cache line, so reading a body never waits for two.
#[derive(Default)]
#[repr(align(64))]
struct BodyMotion(UnsafeCell<[f32x4; 3]>);

// SAFETY: A `BodyMotion` is only reached through `Velocities`. Its methods that take `&mut self` have exclusive
// access, and the others are `unsafe` functions whose callers keep any two threads from reaching the motion of one
// body while either writes it.
#[allow(unsafe_code, reason = "Batches keep lanes from writing the motion of the same body.")]
unsafe impl Sync for BodyMotion {}

/// The `WideMotion` struct holds the velocities and the shift of one body of each contact in a group.
#[derive(Clone, Copy)]
struct WideMotion {
	linear: WideVector,
	angular: WideVector,
	/// How far contacts move the body out of overlap this step, in meters.
	shift: WideVector,
}

/// Returns the first three lanes of four bodies' vectors as one vector for each coordinate, with a lane for each
/// body.
#[inline]
fn coordinates(bodies: [f32x4; WIDTH]) -> WideVector {
	// `xy` holds the x of two bodies and then their y, and `z` starts with their z.
	let (low_xy, low_z) = bodies[0].interleave(bodies[1]);
	let (high_xy, high_z) = bodies[2].interleave(bodies[3]);
	WideVector {
		x: simd_swizzle!(low_xy, high_xy, [0, 1, 4, 5]),
		y: simd_swizzle!(low_xy, high_xy, [2, 3, 6, 7]),
		z: simd_swizzle!(low_z, high_z, [0, 1, 4, 5]),
	}
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

impl Velocities {
	/// Takes the velocities of every body in `bodies`, with no shift.
	fn load(&mut self, bodies: &StableVec<PhysicsBody>) {
		self.bodies.resize_with(bodies.slots_len(), Default::default);
		let whole = |[x, y, z]: [f32; 3]| f32x4::from_array([x, y, z, 0.0]);
		for (slot, body) in bodies.indexed_iter() {
			*self.bodies[slot].0.get_mut() = [
				whole(body.linear_velocity.to_array()),
				whole(body.angular_velocity.to_array()),
				f32x4::splat(0.0),
			];
		}
	}

	/// Gives every body its solved velocities, and shifts it out of overlap.
	fn store(&mut self, bodies: &mut StableVec<PhysicsBody>) {
		for (slot, body) in bodies.indexed_iter_mut() {
			// A body that contacts cannot move is left as it is.
			if body.inv_mass == 0.0 {
				continue;
			}
			let [linear, angular, shift] = self.bodies[slot].0.get_mut().map(|vector| {
				let [x, y, z, _] = vector.to_array();
				Vector::new(x, y, z)
			});
			body.position += shift;
			body.linear_velocity = linear;
			// A contact far from the center of mass can spin a body faster than the simulation can follow, so the
			// spin is capped.
			let speed_squared = angular.length_squared();
			body.angular_velocity = if speed_squared > MAX_ANGULAR_SPEED * MAX_ANGULAR_SPEED {
				angular * (MAX_ANGULAR_SPEED / speed_squared.sqrt())
			} else {
				angular
			};
		}
	}

	/// Returns the linear velocity, angular velocity, and shift of the body in `slot`.
	///
	/// # Safety
	///
	/// No other thread may write the motion of the body in `slot` during the call.
	#[allow(unsafe_code, reason = "The caller keeps other threads from writing this body's motion.")]
	#[inline]
	unsafe fn read(&self, slot: usize) -> [f32x4; 3] {
		// SAFETY: The caller keeps every other thread from writing this body's motion, so nothing changes it while
		// it is copied out.
		unsafe { *self.bodies[slot].0.get() }
	}

	/// Replaces the linear velocity, angular velocity, and shift of the body in `slot`.
	///
	/// # Safety
	///
	/// No other thread may read or write the motion of the body in `slot` during the call.
	#[allow(unsafe_code, reason = "The caller keeps other threads away from this body's motion.")]
	#[inline]
	unsafe fn write(&self, slot: usize, motion: [f32x4; 3]) {
		// SAFETY: The caller keeps every other thread from reading or writing this body's motion, so this is the
		// only access to it.
		unsafe { *self.bodies[slot].0.get() = motion }
	}

	/// Reads the motion of the two bodies of each contact of `contact`.
	///
	/// # Safety
	///
	/// No other thread may write the motion of any body of `contact` during the call.
	#[allow(
		unsafe_code,
		reason = "The caller keeps other threads from writing the motion of the contact's bodies."
	)]
	#[inline]
	unsafe fn gather(&self, contact: &WideContact) -> [WideMotion; 2] {
		contact.bodies.map(|slots| {
			// SAFETY: The caller keeps every other thread from writing the motion of these bodies.
			let bodies = slots.map(|slot| unsafe { self.read(slot) });
			WideMotion {
				linear: coordinates(bodies.map(|motion| motion[0])),
				angular: coordinates(bodies.map(|motion| motion[1])),
				shift: coordinates(bodies.map(|motion| motion[2])),
			}
		})
	}

	/// Writes `motions` to the bodies of `contact` that contacts can move.
	///
	/// # Safety
	///
	/// No other thread may read or write the motion of any movable body of `contact` during the call.
	#[allow(
		unsafe_code,
		reason = "The caller keeps other threads away from the motion of the contact's movable bodies."
	)]
	#[inline]
	unsafe fn scatter(&self, contact: &WideContact, motions: [WideMotion; 2]) {
		for side in 0..2 {
			let motion = motions[side];
			let (linear, angular, shift) = (per_body(motion.linear), per_body(motion.angular), per_body(motion.shift));
			let mut lanes = contact.movable[side];
			while lanes != 0 {
				let lane = lanes.trailing_zeros() as usize;
				lanes &= lanes - 1;
				// SAFETY: The lane's body is movable, and the caller keeps every other thread away from the motion
				// of the contact's movable bodies.
				unsafe { self.write(contact.bodies[side][lane], [linear[lane], angular[lane], shift[lane]]) };
			}
		}
	}
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
	/// The slots of the two bodies of each contact.
	bodies: [[usize; WIDTH]; 2],
	/// Which of the contacts between its two bodies each contact is.
	corners: [u8; WIDTH],
	/// The lanes that hold a contact, one bit per lane.
	in_use: u64,
	/// The lanes whose body takes new velocities, one bit per lane, for each of the two bodies.
	movable: [u64; 2],
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
			([self.bodies[0][lane], self.bodies[1][lane]], self.corners[lane], push)
		})
	}
}

/// Works out what [`solve`] needs for the contacts `group` names in `contacts`, from the bodies' current state.
///
/// `group` holds at most [`WIDTH`] contacts, and no movable body may be in two of them. `dt` is the length of the
/// step in seconds. `last_push` returns the
/// impulse that held two bodies apart in the last step, or zero: resting bodies need about the same again, so it
/// is applied to `velocities` right away and [`solve`] only has to adjust it. Pass the result to [`solve`] once
/// per pass.
///
/// # Safety
///
/// During the call no other thread may write the motion, in `velocities`, of any body of the group's contacts, or
/// read that of a movable one.
#[allow(
	unsafe_code,
	reason = "The caller keeps other threads away from the motion of the group's bodies."
)]
unsafe fn prepare(
	bodies: &StableVec<PhysicsBody>,
	velocities: &Velocities,
	contacts: &Contacts,
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
	let [a, b] = sides.map(|sides| WideSide::gather(bodies, sides));
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
	// a corner of the patch they touch over. They may close that gap in this step, and no more.
	let gap = (zero - depth).simd_max(zero);
	let allowance = touching.select(
		gap.simd_gt(zero).select(gap / f32x4::splat(dt), zero),
		closing_speed.simd_max(zero) * time / f32x4::splat(dt),
	);
	let overlapping = touching & movable;
	let overlap =
		((depth - f32x4::splat(OVERLAP_SLACK)).simd_max(zero) * f32x4::splat(OVERLAP_SHARE)).simd_min(f32x4::splat(MAX_SHIFT));

	let bodies = sides.map(|sides| sides.map(|side| side.object));
	let corners = contacts.map(|contact| contact.corner);
	let pushes = f32x4::from_array(std::array::from_fn(|lane| {
		if lane < group.len() {
			last_push([bodies[0][lane], bodies[1][lane]], corners[lane])
		} else {
			0.0
		}
	}));
	let pushes = solvable[0].select(pushes.simd_max(zero), zero);
	let contact = WideContact {
		bodies,
		corners,
		in_use: in_use.to_bitmask(),
		movable: [
			(in_use & a.inverse_mass.simd_ne(zero)).to_bitmask(),
			(in_use & b.inverse_mass.simd_ne(zero)).to_bitmask(),
		],
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

	// Start the bodies off with the push that held them apart in the last step.
	// SAFETY: The caller keeps every other thread from writing the motion of the group's bodies.
	let mut motions = unsafe { velocities.gather(&contact) };
	push(&contact, &mut motions, 0, pushes);
	// SAFETY: The caller keeps every other thread away from the motion of the group's movable bodies.
	unsafe { velocities.scatter(&contact, motions) };
	contact
}

/// Passes once over the contacts of `contact`, changing the motion of their bodies in `velocities`.
///
/// Along the normal, the bodies are pushed until they close no faster than their contact allows: not at all when
/// they touch, apart from a bounce, and for bodies that only meet later in the step, just fast enough to meet as it
/// ends. The push only
/// ever holds bodies apart: when they are already parting, a later pass takes back what an earlier one applied
/// instead of pulling them together. Across the normal, friction slows their sliding by at most the push times
/// their friction. Bodies that overlap are also shifted apart, which moves them without changing their
/// velocities, so closing an overlap does not throw them.
///
/// # Safety
///
/// During the call no other thread may write the motion, in `velocities`, of any body of the contacts, or read that
/// of a movable one.
#[allow(
	unsafe_code,
	reason = "The caller keeps other threads away from the motion of the contacts' bodies."
)]
unsafe fn solve(contact: &WideContact, velocities: &Velocities) {
	let zero = f32x4::splat(0.0);
	// SAFETY: The caller keeps every other thread from writing the motion of the contacts' bodies.
	let mut motions = unsafe { velocities.gather(contact) };
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
	// SAFETY: The caller keeps every other thread away from the motion of the contacts' movable bodies.
	unsafe { velocities.scatter(contact, motions) };
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

/// The `ContactSolver` struct takes a physics step's contacts from detection to the bodies' new velocities.
///
/// Call [`Self::begin`] with the step's contacts. Then get every batch ready, in order: either all of them with
/// [`Self::prepare_rest`], or the first ones with [`Self::prepare_batch`], whose groups lanes can share, and the
/// others with [`Self::prepare_rest`]. Pass over the groups [`ITERATIONS`] times with [`Self::solve`], batch after
/// batch in the same order, and call [`Self::finish`].
#[derive(Default)]
pub(crate) struct ContactSolver {
	/// The step's contacts, sorted into batches that can each be solved in any order.
	batches: ContactBatches,
	/// What [`Self::prepare_rest`] worked out, in groups: the groups of each of its batches in turn, then one group
	/// for each contact that fit in no batch. They share bodies, so one lane passes over them in order.
	rest: Vec<WideContact>,
	/// The velocities of the bodies while the step's contacts are solved.
	velocities: Velocities,
	/// The impulse that held each pair of bodies apart at each of their contacts in the last step, by [`pair_key`]. Resting bodies need about
	/// the same impulse again, so the next step's solve starts from it.
	pushes: HashMap<u64, f32>,
	/// The slots whose body was replaced since the last step. Their pairs' impulses belonged to other bodies.
	replaced: Vec<usize>,
	/// Whether the step has contacts.
	active: bool,
}

impl ContactSolver {
	/// Forgets the impulses of the pairs that include the body in `slot`. Call it when another body takes the slot,
	/// so that the new body does not start from impulses the old one needed.
	pub(crate) fn forget(&mut self, slot: usize) {
		self.replaced.push(slot);
	}

	/// Sorts a step's `contacts` into batches and takes the bodies' velocities, ready for the batches to be prepared.
	///
	/// The contacts of a step are solved together, so no order among them is more correct than another. Batching
	/// picks one that lets a batch's contacts be solved at the same time.
	pub(crate) fn begin(&mut self, bodies: &StableVec<PhysicsBody>, contacts: &Contacts) {
		self.rest.clear();
		if !self.replaced.is_empty() {
			self.replaced.sort_unstable();
			let replaced = |slot: u64| self.replaced.binary_search(&(slot as usize)).is_ok();
			self.pushes
				.retain(|pair, _| pair_slots(*pair).into_iter().all(|slot| !replaced(slot)));
			self.replaced.clear();
		}
		self.active = contacts.len() != 0;
		if !self.active {
			// Without contacts there are no batches either.
			self.batches.rebuild(0, std::iter::empty());
			return;
		}
		// A body without mass is never changed by a contact, so any number of a batch's contacts can touch it.
		let movable = |slot: usize| bodies.get_slot(slot).is_some_and(|body| body.inv_mass != 0.0).then_some(slot);
		self.batches.rebuild(
			bodies.slots_len(),
			contacts
				.iter()
				.map(|contact| [movable(contact.a.object), movable(contact.b.object)]),
		);
		// Preparing a group starts its bodies off with the last step's impulses, so the velocities come first.
		self.velocities.load(bodies);
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

	/// Returns the number of groups the contacts of `batch` are prepared and solved in.
	pub(crate) fn batch_groups(&self, batch: usize) -> usize {
		self.batch_contacts(batch).div_ceil(WIDTH)
	}

	/// Returns whether any contact is left for [`Self::prepare_rest`] once the first `batches` batches are prepared.
	pub(crate) fn has_rest(&self, batches: usize) -> bool {
		batches < self.batches.len() || !self.batches.leftover().is_empty()
	}

	/// Works out what solving `groups` of the groups of `batch` needs, and adds it to `prepared` in order. `dt` is
	/// the length of the step in seconds.
	///
	/// The groups of one batch share no movable body, so lanes can each prepare some of them at the same time.
	/// Batches have to follow each other in order, because preparing a group starts its bodies off with the last
	/// step's impulses.
	///
	/// # Safety
	///
	/// While this runs, no other thread may prepare or solve a group that shares a movable body with one of
	/// `groups`. Other groups of the same batch never do.
	#[allow(
		unsafe_code,
		reason = "The caller keeps other threads from groups that share a movable body with these."
	)]
	pub(crate) unsafe fn prepare_batch(
		&self,
		bodies: &StableVec<PhysicsBody>,
		contacts: &Contacts,
		dt: f32,
		batch: usize,
		groups: Range<usize>,
		prepared: &mut Vec<WideContact>,
	) {
		let batch = self.batches.batch(batch);
		prepared.reserve(groups.len());
		for group in groups {
			let group = &batch[group * WIDTH..((group + 1) * WIDTH).min(batch.len())];
			// SAFETY: The caller keeps every other thread from the groups that share a movable body with this one.
			prepared.push(unsafe { self.prepare_group(bodies, contacts, dt, group) });
		}
	}

	/// Prepares the batches from `first_batch` on, and then the contacts that fit in no batch, for one lane to
	/// solve with [`Self::solve_rest`].
	#[allow(
		unsafe_code,
		reason = "Exclusive access to the solver keeps other threads from every group."
	)]
	pub(crate) fn prepare_rest(&mut self, bodies: &StableVec<PhysicsBody>, contacts: &Contacts, dt: f32, first_batch: usize) {
		let mut rest = std::mem::take(&mut self.rest);
		for batch in first_batch..self.batches.len() {
			let groups = 0..self.batch_groups(batch);
			// SAFETY: This call has the only reference to the solver, so no other thread prepares or solves a group.
			unsafe { self.prepare_batch(bodies, contacts, dt, batch, groups, &mut rest) };
		}
		// Contacts that fit in no batch share bodies, so each one is a group of its own and they are solved in turn.
		for contact in self.batches.leftover() {
			// SAFETY: This call has the only reference to the solver, so no other thread prepares or solves a group.
			rest.push(unsafe { self.prepare_group(bodies, contacts, dt, std::slice::from_ref(contact)) });
		}
		self.rest = rest;
	}

	/// Works out what solving the contacts `group` names needs, starting from the impulse that held each pair of
	/// bodies apart in the last step.
	///
	/// # Safety
	///
	/// While this runs, no other thread may prepare or solve a group that shares a movable body with this one.
	#[allow(
		unsafe_code,
		reason = "The caller keeps other threads from groups that share a movable body with this one."
	)]
	unsafe fn prepare_group(
		&self,
		bodies: &StableVec<PhysicsBody>,
		contacts: &Contacts,
		dt: f32,
		group: &[u32],
	) -> WideContact {
		let last_push = |pair, corner| self.pushes.get(&pair_key(pair, corner)).copied().unwrap_or(0.0);
		// SAFETY: Preparing and solving a group only reaches the motion of its bodies, and only writes that of its
		// movable ones. The caller keeps every other thread from the groups that have one of those.
		unsafe { prepare(bodies, &self.velocities, contacts, group, dt, last_push) }
	}

	/// Passes once over `groups`, which [`Self::prepare_batch`] prepared. The groups of one batch can be passed
	/// over in any order and by several lanes at once; batches, and [`Self::solve_rest`], have to follow each other.
	///
	/// # Safety
	///
	/// While this runs, no other thread may prepare or solve a group that shares a movable body with one of
	/// `groups`. Other groups of the same batch never do.
	#[allow(
		unsafe_code,
		reason = "The caller keeps other threads from groups that share a movable body with these."
	)]
	pub(crate) unsafe fn solve(&self, groups: &[WideContact]) {
		for group in groups {
			// SAFETY: Preparing and solving a group only reaches the motion of its bodies, and only writes that of
			// its movable ones. The caller keeps every other thread from the groups that have one of those.
			unsafe { solve(group, &self.velocities) };
		}
	}

	/// Passes once over the groups [`Self::prepare_rest`] prepared.
	#[allow(
		unsafe_code,
		reason = "Exclusive access to the solver keeps other threads from every group."
	)]
	pub(crate) fn solve_rest(&mut self) {
		// SAFETY: This call has the only reference to the solver, so no other thread prepares or solves a group.
		unsafe { self.solve(&self.rest) };
	}

	/// Gives the bodies the velocities the passes left them with, and keeps each contact's impulse for the next
	/// step to start from. `shared` are the groups [`Self::prepare_batch`] prepared.
	pub(crate) fn finish<'a>(&mut self, bodies: &mut StableVec<PhysicsBody>, shared: impl Iterator<Item = &'a WideContact>) {
		// Only pairs still in contact keep an impulse.
		self.pushes.clear();
		if self.active {
			self.velocities.store(bodies);
			let pushes = &mut self.pushes;
			let mut remember = |group: &WideContact| {
				pushes.extend(group.pushes().map(|(pair, corner, push)| (pair_key(pair, corner), push)));
			};
			shared.for_each(&mut remember);
			self.rest.iter().for_each(&mut remember);
		}
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
