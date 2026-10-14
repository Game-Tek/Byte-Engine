//! Constraints for Dynabit: which pairs of bodies a world holds to a limit, and the velocity changes that hold them.
//!
//! [`Constraints`] keeps a world's constraints by handle and takes them through a step in three parts: it gets them
//! ready, solves them one after the other on one lane, [`SOLVES`] times before each pass over the step's contacts,
//! and keeps what the next step starts from. [`Row::new`] works out what a step needs of one constraint, and
//! [`Row::solve`] changes the motion of its two bodies so that they keep to it.

use std::collections::hash_map::Entry;

use math::{Point, Vector};
use utils::hash::HashMap;

use crate::{
	core::factory::Handle,
	physics::{
		constraint::Constraint,
		dynabit::{
			body::PhysicsBody,
			solver::{BodyMotion, BodyState, ContactSolver},
		},
	},
};

/// The number of times the constraints are solved before each pass over a step's contacts.
///
/// One solve lets each constraint react to its own two bodies. Constraints that share a body, as the links of a
/// chain do, only carry each other's load once they have reacted to each other, which takes more solves the longer
/// the chain and the heavier what hangs from it. Twelve links that hold a body of ten times their mass came to be
/// 42 cm off their lengths together with one solve, 7 cm with this many, and 2 cm with twelve.
///
/// These few are enough because every step starts from the impulses of the one before, which the links of a chain
/// that hangs or swings need about the same of again. Starting from nothing, the same chain was 96 cm off with this
/// many solves, and 32 cm off with twelve.
const SOLVES: usize = 4;

/// The largest impulse, in newton seconds, that a constraint keeps for the next step to start from.
///
/// An impulse beyond it is left over from something that has gone wrong, and starting from it would throw the
/// bodies.
const MAX_KEPT_IMPULSE: f32 = 1e5;

/// The share of what the anchors are off their length that a constraint takes up in one step.
///
/// The step before leaves the anchors a little off, because bodies move straight on for a step where a rod would
/// swing them. Taking up more of it at once holds a single rod closer to its length, but makes the links of a
/// chain that carries a heavy body throw each other about: with a share of 0.2, twelve links that hold a body of a
/// hundred times their mass came apart by meters.
const ERROR_SHARE: f32 = 0.05;

/// How far off their length, in meters, a constraint leaves its anchors, so that it does not keep nudging bodies
/// that are as good as where they should be.
const ERROR_SLACK: f32 = 0.005;

/// The `Constraints` struct keeps the constraints of a physics world, and holds its bodies to them during a step.
///
/// [`Self::insert`] a constraint when it is created, and [`Self::remove`] it when its handle is deleted. In every
/// step that has any, tell [`ContactSolver::begin`] so and call [`Self::pre_solve`] once it has begun. Call
/// [`Self::solve`] before each pass over the step's contacts, so that the contacts have the last word on where
/// bodies may be, and [`Self::post_solve`] after the last pass.
#[derive(Default)]
pub(crate) struct Constraints {
	/// The constraints, in the order they are solved in.
	held: Vec<Held>,
	/// Where each constraint is in `held`, by the handle it was created with.
	places: HashMap<Handle, usize>,
}

/// The `Held` struct keeps one constraint with what the steps work out for it.
struct Held {
	handle: Handle,
	constraint: Constraint,
	/// What solving the constraint needs, from [`Constraints::pre_solve`] to [`Constraints::post_solve`]. None
	/// between steps, and for a constraint that does not act in the current one.
	row: Option<Row>,
	/// The impulse the last step left the constraint at, which the next one starts from.
	kept: f32,
}

impl Constraints {
	/// Returns whether the world has no constraints, so that a step has none to solve.
	pub(crate) fn is_empty(&self) -> bool {
		self.held.is_empty()
	}

	/// Keeps `constraint` under `handle`, in place of the constraint that handle had before, if any.
	pub(crate) fn insert(&mut self, handle: Handle, constraint: Constraint) {
		let held = Held {
			handle,
			constraint,
			row: None,
			kept: 0.0,
		};
		match self.places.entry(handle) {
			Entry::Occupied(place) => self.held[*place.get()] = held,
			Entry::Vacant(place) => {
				place.insert(self.held.len());
				self.held.push(held);
			}
		}
	}

	/// Forgets the constraint that was kept under `handle`, if any.
	pub(crate) fn remove(&mut self, handle: Handle) {
		let Some(place) = self.places.remove(&handle) else {
			return;
		};
		self.held.swap_remove(place);
		// The last constraint took the place of the removed one.
		if let Some(moved) = self.held.get(place) {
			self.places.insert(moved.handle, place);
		}
	}

	/// Gets every constraint ready for the step: works out what solving it needs, and starts its two bodies off with
	/// the impulse that held them in the last step. They need about the same again, so that [`Self::solve`] only has
	/// to adjust it.
	///
	/// `body` returns the slot of the body that was created with a handle, and that body as it is before the step
	/// moves it. A constraint without both of its bodies does not act in this step. `solver` has begun the step,
	/// which is `dt` seconds long.
	pub(crate) fn pre_solve<'a>(
		&mut self,
		body: impl Fn(Handle) -> Option<(usize, &'a PhysicsBody)>,
		solver: &mut ContactSolver,
		dt: f32,
	) {
		for held in &mut self.held {
			held.row = None;
			let [Some((first_slot, first)), Some((second_slot, second))] = held.constraint.bodies().map(&body) else {
				continue;
			};
			// A body keeps every distance to itself.
			if first_slot == second_slot {
				continue;
			}
			let slots = [first_slot, second_slot];
			let [first_anchor, second_anchor] = held.constraint.anchors();
			let anchors = [first.to_world(first_anchor), second.to_world(second_anchor)];
			let bodies = slots.map(|slot| solver.state(slot));
			held.row = Row::new(slots, bodies, anchors, held.constraint.length(), dt);
			if let Some(row) = &mut held.row {
				row.pull(solver.pair_mut(slots), held.kept);
			}
		}
	}

	/// Solves the constraints that [`Self::pre_solve`] found to act in this step, [`SOLVES`] times over, changing
	/// the motion that `solver` keeps of their bodies. Only a stage that one lane runs can call it.
	///
	/// Constraints that share a body depend on each other, as the links of a chain do, so they are taken in turn.
	pub(crate) fn solve(&mut self, solver: &mut ContactSolver) {
		for _ in 0..SOLVES {
			for row in self.held.iter_mut().filter_map(|held| held.row.as_mut()) {
				row.solve(solver.pair_mut(row.slots));
			}
		}
	}

	/// Ends the step for every constraint: keeps the impulse it was left at, within [`MAX_KEPT_IMPULSE`], for the
	/// next step to start from. Call it once the step's last pass is done.
	pub(crate) fn post_solve(&mut self) {
		for held in &mut self.held {
			let pull = held.row.take().map_or(0.0, |row| row.applied);
			// An impulse that is not a number would start the bodies of every later step off with one, long after
			// whatever caused it is gone. A constraint that did not act starts again from nothing.
			held.kept = if pull.is_finite() {
				pull.clamp(-MAX_KEPT_IMPULSE, MAX_KEPT_IMPULSE)
			} else {
				0.0
			};
		}
	}
}

/// The `Row` struct holds what solving one distance constraint needs, worked out once a step so that solving it
/// only reads and writes the motion of its two bodies.
///
/// A distance constraint limits its bodies along one direction, the line between its anchors: the anchors may neither
/// close nor part along it. That is one row of the equations all of a step's constraints make together, which are
/// solved one row at a time, several times over.
struct Row {
	/// The slots of the two bodies.
	slots: [usize; 2],
	/// The direction from the first body's anchor toward the second's.
	direction: Vector,
	/// For each body, how its spin moves its anchor along the direction.
	arms: [Vector; 2],
	/// For each body, how a unit impulse along the direction changes its spin.
	turns: [Vector; 2],
	inverse_mass: [f32; 2],
	/// The impulse that changes the speed the anchors part at by one.
	mass: f32,
	/// The speed the anchors should close at, which takes up a share of what they are off their length. Negative
	/// for anchors that are closer than that length.
	closing: f32,
	/// The impulse applied so far that pulls the anchors together. Negative when it pushes them apart.
	applied: f32,
}

impl Row {
	/// Works out the row of a constraint that keeps `anchors`, one point of each body in world space, `length` meters
	/// apart, for a step of `dt` seconds. `bodies` is what the solver took of the two bodies, which are in `slots`.
	///
	/// Returns None for a constraint that cannot act in this step: one whose bodies no impulse moves, or that pins
	/// two anchors that are already at the same place, which have no direction to hold them along.
	fn new(slots: [usize; 2], bodies: [BodyState; 2], anchors: [Point; 2], length: f32, dt: f32) -> Option<Self> {
		let apart = anchors[1] - anchors[0];
		let distance = apart.length();
		let direction = if distance > f32::EPSILON {
			apart / distance
		} else if length > 0.0 {
			// Anchors at the same place are as far from their length along every direction, so any one parts them.
			Vector::new(1.0, 0.0, 0.0)
		} else {
			return None;
		};

		// An impulse along the direction changes how fast the anchors part by the bodies' inverse masses, and by how
		// easily each turns about its center of mass under an impulse at its anchor.
		let arms = [0, 1].map(|side| (anchors[side] - bodies[side].center()).cross(direction));
		let turns = [0, 1].map(|side| bodies[side].turn(arms[side]));
		let inverse_mass = [0, 1].map(|side| bodies[side].inverse_mass());
		let resistance = inverse_mass[0] + inverse_mass[1] + arms[0].dot(turns[0]) + arms[1].dot(turns[1]);
		// Two bodies that nothing moves, or mass or inertia data that is not valid.
		if !resistance.is_finite() || resistance <= f32::EPSILON {
			return None;
		}

		// What the anchors are off their length beyond the slack, with its sign. A step without length, as a paused
		// world's is, has no speed that takes any of it up.
		let off = distance - length;
		let off = off.signum() * (off.abs() - ERROR_SLACK).max(0.0);
		let closing = if dt > 0.0 { off * ERROR_SHARE / dt } else { 0.0 };

		Some(Self {
			slots,
			direction,
			arms,
			turns,
			inverse_mass,
			mass: 1.0 / resistance,
			closing,
			applied: 0.0,
		})
	}

	/// Solves the constraint once, changing the `motions` of its two bodies.
	///
	/// The bodies get the impulse along the line between the anchors that leaves the anchors closing at the speed
	/// that takes up a share of what they are off their length. Anchors at their length neither close nor part,
	/// which keeps their distance as it is.
	fn solve(&mut self, motions: [&mut BodyMotion; 2]) {
		let ([first_linear, first_angular, _], [second_linear, second_angular, _]) = (motions[0].parts(), motions[1].parts());
		let parting_speed = self.direction.dot(second_linear - first_linear) + second_angular.dot(self.arms[1])
			- first_angular.dot(self.arms[0]);
		self.pull(motions, (parting_speed + self.closing) * self.mass);
	}

	/// Pulls the anchors together with `impulse`, or pushes them apart with a negative one.
	fn pull(&mut self, motions: [&mut BodyMotion; 2], impulse: f32) {
		// The first body goes along the direction, toward the second one, and the second body against it.
		for (side, (motion, toward)) in motions.into_iter().zip([1.0, -1.0]).enumerate() {
			motion.push(
				self.direction * (toward * impulse * self.inverse_mass[side]),
				self.turns[side] * (toward * impulse),
			);
		}
		self.applied += impulse;
	}
}
