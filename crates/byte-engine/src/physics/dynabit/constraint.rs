//! Constraints for Dynabit: which pairs of bodies a world holds to a limit, and the velocity changes that hold them.
//!
//! [`Constraints`] keeps a world's constraints by handle and takes them through a step in three parts: it gets them
//! ready, solves them one after the other on one lane before each pass over the step's contacts, and keeps what the
//! next step starts from. [`Row::new`] works out what a step needs of one constraint, and [`Row::solve`] changes the
//! motion of its two bodies so that they keep to it.

use std::collections::hash_map::Entry;

use math::{Point, Vector};
use utils::hash::HashMap;

use crate::{
	core::factory::Handle,
	physics::{
		constraint::Constraint,
		dynabit::{
			body::PhysicsBody,
			solver::{BodyMotion, BodyState, ContactSolver, MAX_SHIFT},
		},
	},
};

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
	/// moves it. A constraint without both of its bodies does not act in this step. `solver` has begun the step.
	pub(crate) fn pre_solve<'a>(
		&mut self,
		body: impl Fn(Handle) -> Option<(usize, &'a PhysicsBody)>,
		solver: &mut ContactSolver,
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
			held.row = Row::new(slots, bodies, anchors, held.constraint.length());
			if let Some(row) = &mut held.row {
				row.apply(solver.pair_mut(slots), held.kept, 0.0);
			}
		}
	}

	/// Solves once the constraints that [`Self::pre_solve`] found to act in this step, changing the motion that
	/// `solver` keeps of their bodies. Only a stage that one lane runs can call it.
	///
	/// Constraints that share a body depend on each other, as the links of a chain do, so they are taken in turn.
	pub(crate) fn solve(&mut self, solver: &mut ContactSolver) {
		for row in self.held.iter_mut().filter_map(|held| held.row.as_mut()) {
			row.solve(solver.pair_mut(row.slots));
		}
	}

	/// Ends the step for every constraint: keeps the impulse it was left at, for the next step to start from. Call
	/// it once the step's last pass is done.
	pub(crate) fn post_solve(&mut self) {
		for held in &mut self.held {
			let pull = held.row.take().map_or(0.0, |row| row.pull);
			// An impulse that is not a number would start the bodies of every later step off with one, long after
			// whatever caused it is gone. A constraint that did not act starts again from nothing.
			held.kept = if pull.is_finite() { pull } else { 0.0 };
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
	/// How much closer the anchors should be shifted in this step, in meters. Negative for anchors that are closer
	/// than the constraint's length.
	closer: f32,
	/// The share of a shift each body takes, by how easily it moves.
	shares: [f32; 2],
	/// The impulse applied so far that pulls the anchors together. Negative when it pushes them apart.
	pull: f32,
}

impl Row {
	/// Works out the row of a constraint that keeps `anchors`, one point of each body in world space, `length` meters
	/// apart. `bodies` is what the solver took of the two bodies, which are in `slots`.
	///
	/// Returns None for a constraint that cannot act in this step: one whose bodies no impulse moves, or that pins
	/// two anchors that are already at the same place, which have no direction to hold them along.
	fn new(slots: [usize; 2], bodies: [BodyState; 2], anchors: [Point; 2], length: f32) -> Option<Self> {
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
		let inverse_mass_sum = inverse_mass[0] + inverse_mass[1];
		let resistance = inverse_mass_sum + arms[0].dot(turns[0]) + arms[1].dot(turns[1]);
		// Two bodies that nothing moves, or mass or inertia data that is not valid.
		if !resistance.is_finite() || resistance <= f32::EPSILON || inverse_mass_sum <= 0.0 {
			return None;
		}

		Some(Self {
			slots,
			direction,
			arms,
			turns,
			inverse_mass,
			mass: 1.0 / resistance,
			closer: (distance - length).clamp(-MAX_SHIFT, MAX_SHIFT),
			shares: inverse_mass.map(|inverse_mass| inverse_mass / inverse_mass_sum),
			pull: 0.0,
		})
	}

	/// Solves the constraint once, changing the `motions` of its two bodies.
	///
	/// The bodies get the impulse along the line between the anchors that stops the anchors from closing or parting
	/// along it, which keeps their distance as it is. The step before left the anchors a little off the constraint's
	/// length, because bodies move straight on for a step while a rod would swing them. So the bodies are also
	/// shifted until the anchors are that length apart, which moves them without changing their velocities.
	fn solve(&mut self, motions: [&mut BodyMotion; 2]) {
		let [
			[first_linear, first_angular, first_shift],
			[second_linear, second_angular, second_shift],
		] = [motions[0].parts(), motions[1].parts()];
		let parting_speed = self.direction.dot(second_linear - first_linear) + second_angular.dot(self.arms[1])
			- first_angular.dot(self.arms[0]);
		// The bodies' shifts so far, by this constraint or by anything else, may have closed part of the distance.
		let closed = self.direction.dot(first_shift - second_shift);
		self.apply(motions, parting_speed * self.mass, self.closer - closed);
	}

	/// Pulls the anchors together with `impulse`, or pushes them apart with a negative one, and shifts the bodies so
	/// that the anchors are `closer` meters closer.
	fn apply(&mut self, motions: [&mut BodyMotion; 2], impulse: f32, closer: f32) {
		// The first body goes along the direction, toward the second one, and the second body against it.
		for (side, (motion, toward)) in motions.into_iter().zip([1.0, -1.0]).enumerate() {
			motion.add([
				self.direction * (toward * impulse * self.inverse_mass[side]),
				self.turns[side] * (toward * impulse),
				self.direction * (toward * closer * self.shares[side]),
			]);
		}
		self.pull += impulse;
	}
}
