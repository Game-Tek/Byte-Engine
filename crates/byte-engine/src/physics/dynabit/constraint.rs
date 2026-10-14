//! Constraints for Dynabit: which pairs of bodies a world holds to a limit, and the velocity changes that hold them.
//!
//! [`Constraints`] keeps a world's constraints by handle and takes them through a step in three parts: it gets them
//! ready, solves them one after the other on one lane, [`SOLVES`] times before each pass over the step's contacts,
//! and keeps what the next step starts from.
//!
//! Getting a constraint ready turns it into [`Row`]s, one for each direction it limits its bodies along or about: one
//! for a rod, and up to [`ROWS`] for a joint. [`Joined`] works them out, and [`Row::solve`] changes the motion of the
//! two bodies so that they keep to one.

use std::collections::hash_map::Entry;

use math::{Orientation, Point, Radians, UnitVector, Vector};
use utils::hash::HashMap;

use crate::{
	core::factory::Handle,
	physics::{
		LocalSpace,
		constraint::{Constraint, Kind, Resist},
		dynabit::{
			body::PhysicsBody,
			solver::{BodyMotion, BodyState, ContactSolver},
		},
	},
};

/// The most rows one constraint has, which a hinge with a limit and resistance does: three that pin its anchors
/// together, two that keep its bodies turned about its pin only, one that stops them at the limit, and one that
/// holds them back. Other joints have other rows after the three that pin them.
const ROWS: usize = 7;

/// The range of impulses of a row that both pulls and pushes, as far as it takes.
const BOTH_WAYS: [f32; 2] = [f32::NEG_INFINITY, f32::INFINITY];

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

/// The share of what its bodies are off where a constraint holds them that the constraint takes up in one step.
///
/// The step before leaves them a little off, because bodies move straight on for a step where a rod would swing
/// them. Taking up more of it at once holds a single rod closer to its length, but makes the links of a
/// chain that carries a heavy body throw each other about: with a share of 0.2, twelve links that hold a body of a
/// hundred times their mass came apart by meters.
const ERROR_SHARE: f32 = 0.05;

/// How far off each of its rows a constraint leaves its bodies, so that it does not keep nudging bodies that are as
/// good as where they should be. In meters for anchors, and in radians for how bodies are turned.
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
	/// The rows of the constraints that act in the current step, from [`Self::pre_solve`] to [`Self::post_solve`],
	/// constraint after constraint. Empty between steps, and kept so that getting ready does not allocate.
	rows: Vec<Row>,
}

/// The `Held` struct keeps one constraint with what the steps work out for it.
struct Held {
	handle: Handle,
	constraint: Constraint,
	/// How the second body was turned as the first one saw it, in the first step that had both. A joint keeps them
	/// turned so, apart from the turns it allows. None until that step, and for a rod.
	rest: Option<Orientation>,
	/// The impulse the last step left each of the constraint's rows at, which the next one starts from.
	kept: [f32; ROWS],
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
			rest: None,
			kept: [0.0; ROWS],
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

	/// Gets every constraint ready for the step: works out the rows that solving it needs, and starts its two bodies
	/// off with the impulses that held them in the last step. They need about the same again, so that
	/// [`Self::solve`] only has to adjust them.
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
		self.rows.clear();
		for (place, held) in self.held.iter_mut().enumerate() {
			// A row that does not act in this step starts the next one it acts in from nothing.
			let kept = std::mem::take(&mut held.kept);
			let [Some((first_slot, first)), Some((second_slot, second))] = held.constraint.bodies().map(&body) else {
				continue;
			};
			// A body keeps to itself whatever it is held to.
			if first_slot == second_slot {
				continue;
			}
			let slots = [first_slot, second_slot];
			let [first_anchor, second_anchor] = held.constraint.anchors();
			let joined = Joined {
				place,
				slots,
				bodies: slots.map(|slot| solver.state(slot)),
				anchors: [first.to_world(first_anchor), second.to_world(second_anchor)],
				dt,
			};

			let first_row = self.rows.len();
			let turned = [first.orientation, second.orientation];
			let mut rest = || *held.rest.get_or_insert_with(|| turned[0].inverse().compose(turned[1]));
			let mut pins = |axis| Pins::new(turned, rest(), axis);
			// Every joint pins its anchors together with rows 0 to 2. What it allows of how its bodies turn is told
			// by its other rows.
			match held.constraint.kind() {
				Kind::Distance { length } => self.rows.extend(joined.rod(length)),
				Kind::Hinge { axis, limit } => {
					let pins = pins(axis);
					let stopped = limit.and_then(|limit| joined.twist_stop(5, &pins, limit));
					self.rows.extend(joined.pin().chain(joined.aligned(&pins)).chain(stopped));
				}
				Kind::ConstantVelocity { axis, limit } => {
					let pins = pins(axis);
					let stopped = limit.and_then(|limit| joined.lean_stop(4, &pins, limit));
					self.rows
						.extend(joined.pin().chain(joined.untwisted(3, &pins)).chain(stopped));
				}
				Kind::Ball { axis, lean, twist } => {
					let pins = pins(axis);
					let twist = twist.and_then(|limit| joined.twist_stop(3, &pins, limit));
					let lean = lean.and_then(|limit| joined.lean_stop(4, &pins, limit));
					self.rows.extend(joined.pin().chain(twist).chain(lean));
				}
			}
			if let Some(resistance) = held.constraint.resistance() {
				self.rows.extend(joined.resisted(6, turned, rest(), resistance));
			}
			for row in &mut self.rows[first_row..] {
				// A row that only pulls or only pushes may have changed which since it left the impulse.
				row.pull(solver.pair_mut(slots), kept[row.number].clamp(row.range[0], row.range[1]));
			}
		}
	}

	/// Solves the rows that [`Self::pre_solve`] found to act in this step, [`SOLVES`] times over, changing the
	/// motion that `solver` keeps of their bodies. Only a stage that one lane runs can call it.
	///
	/// Rows that share a body depend on each other, as those of one hinge and of the links of a chain do, so they
	/// are taken in turn.
	pub(crate) fn solve(&mut self, solver: &mut ContactSolver) {
		for _ in 0..SOLVES {
			for row in &mut self.rows {
				row.solve(solver.pair_mut(row.slots));
			}
		}
	}

	/// Ends the step for every constraint: keeps the impulse each of its rows was left at, within
	/// [`MAX_KEPT_IMPULSE`], for the next step to start from. Call it once the step's last pass is done.
	pub(crate) fn post_solve(&mut self) {
		for row in self.rows.drain(..) {
			// An impulse that is not a number would start the bodies of every later step off with one, long after
			// whatever caused it is gone.
			self.held[row.place].kept[row.number] = if row.applied.is_finite() {
				row.applied.clamp(-MAX_KEPT_IMPULSE, MAX_KEPT_IMPULSE)
			} else {
				0.0
			};
		}
	}
}

/// The `Joined` struct holds the two bodies of one constraint as a step finds them, which the constraint's rows are
/// worked out from.
struct Joined {
	/// Where the constraint is among the world's.
	place: usize,
	/// The slots of the two bodies.
	slots: [usize; 2],
	/// What the solver took of the two bodies.
	bodies: [BodyState; 2],
	/// Where each body's anchor is in the world.
	anchors: [Point; 2],
	/// The length of the step in seconds.
	dt: f32,
}

impl Joined {
	/// Returns the row of a rod that keeps the anchors `length` meters apart, which limits them along the line
	/// between them: they may neither close nor part along it.
	///
	/// Returns None for a rod that pins two anchors that are already at the same place, which have no line to hold
	/// them along.
	fn rod(&self, length: f32) -> Option<Row> {
		let apart = self.anchors[1] - self.anchors[0];
		let distance = apart.length();
		let direction = if distance > f32::EPSILON {
			apart / distance
		} else if length > 0.0 {
			// Anchors at the same place are as far from their length along every direction, so any one parts them.
			Vector::new(1.0, 0.0, 0.0)
		} else {
			return None;
		};
		self.along(0, direction, distance - length)
	}

	/// Returns the rows that keep the anchors at the same place: one along each axis of the world.
	///
	/// The axes stay as they are from step to step, where the line between two anchors that are as good as together
	/// points anywhere. So the impulses that a step leaves these rows at mean the same to the next step.
	fn pin(&self) -> impl Iterator<Item = Row> {
		let off = (self.anchors[1] - self.anchors[0]).to_array();
		let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
		(0..3).filter_map(move |axis| self.along(axis, Vector::from_array(axes[axis]), off[axis]))
	}

	/// Returns the two rows that keep the second body's pin along the first one's, as the pin of a hinge is, so that
	/// the bodies only turn relative to each other about it.
	fn aligned(&self, pins: &Pins) -> impl Iterator<Item = Row> {
		// The second body's pin lies along the first one's when it has nothing along two directions across that
		// one. What it has along one of them changes with how fast the bodies turn relative to each other about the
		// direction that is across both it and the second pin.
		let second_pin = pins.world[1];
		let across = across(pins.axis).map(|across| to_world(pins.first, across));
		(3..).zip(across).filter_map(move |(number, across)| {
			self.about(
				number,
				second_pin.cross(across),
				self.take_up(across.dot(second_pin)),
				BOTH_WAYS,
			)
		})
	}

	/// Returns the row that keeps the second body from turning about its pin, as the two halves of a drive shaft
	/// are kept at a constant velocity joint. None for a joint that is folded back on itself.
	fn untwisted(&self, number: usize, pins: &Pins) -> Option<Row> {
		self.about(number, pins.twists_about?, self.take_up(pins.twist), BOTH_WAYS)
	}

	/// Returns the row that stops the second body's turn about its pin at the end of `limit` that it is nearer to.
	/// None for a joint that is folded back on itself.
	fn twist_stop(&self, number: usize, pins: &Pins, limit: [Radians; 2]) -> Option<Row> {
		let [back, on] = limit.map(Radians::value);
		// A step only brings the turn to the end of the limit that it is nearer to. A row stops it there by turning
		// the bodies one way only: back from the end.
		let (end, range) = if pins.twist - back < on - pins.twist {
			(back, [f32::NEG_INFINITY, 0.0])
		} else {
			(on, [0.0, f32::INFINITY])
		};
		let closing = self.stop(pins.twist - end, (back..=on).contains(&pins.twist));
		self.about(number, pins.twists_about?, closing, range)
	}

	/// Returns the row that stops the second body's pin from leaning further than `limit` away from the first
	/// one's. None for pins that point the same way, which are a step away from any limit worth setting.
	fn lean_stop(&self, number: usize, pins: &Pins, limit: Radians) -> Option<Row> {
		// The second pin leans away from the first by turning about the direction across both.
		let [first_pin, second_pin] = pins.world;
		let leans_about = first_pin.cross(second_pin);
		let sine = leans_about.length();
		if sine <= f32::EPSILON {
			return None;
		}
		let past = sine.atan2(first_pin.dot(second_pin)) - limit.value();
		self.about(number, leans_about / sine, self.stop(past, past <= 0.0), [0.0, f32::INFINITY])
	}

	/// Returns the row that holds the second body back from turning further away from how it was turned to the
	/// first when the constraint first held them. `resistance` returns the torque that does, in newton meters, at
	/// the angle the body is turned by. None for bodies that are turned as they were then.
	///
	/// `turned` is how the two bodies are turned, and `rest` how the second was as the first saw it then.
	fn resisted(&self, number: usize, turned: [Orientation; 2], rest: Orientation, resistance: &Resist) -> Option<Row> {
		// However the second body has turned since, one turn about one line takes it there. The angle of that turn
		// grows as fast as the bodies turn relative to each other about that line.
		let turn = turned[0].inverse().compose(turned[1]).compose(rest.inverse());
		let turn = turn.to_rotation_vector::<LocalSpace>();
		let angle = turn.length();
		if angle <= f32::EPSILON {
			return None;
		}
		// The most impulse the torque gives in this step. A row that may pull no harder slows the turn by what
		// that does, and stops a turn that takes less. It never turns the bodies back, as it pulls one way only.
		let most = resistance(Radians::new(angle)) * self.dt;
		// A torque that is not a number, or that would push the body on, holds nothing back.
		if most.is_nan() || most <= 0.0 {
			return None;
		}
		self.about(number, to_world(turned[0], turn / angle), 0.0, [0.0, most])
	}

	/// Returns the row that limits how the anchors move relative to each other along `direction`, a unit vector.
	/// `off` is how far along it the second anchor is from where the constraint holds it.
	fn along(&self, number: usize, direction: Vector, off: f32) -> Option<Row> {
		// A body's spin moves its anchor along the direction by how the anchor's place from the center of mass and
		// the direction cross.
		let arms = [0, 1].map(|side| (self.anchors[side] - self.bodies[side].center()).cross(direction));
		self.row(number, direction, arms, self.take_up(off), BOTH_WAYS)
	}

	/// Returns the row that limits how the bodies turn relative to each other about `arm`, to impulses within `range`.
	/// `closing` is the speed, in radians per second, that the row leaves the first body turning ahead of the second
	/// about it.
	fn about(&self, number: usize, arm: Vector, closing: f32, range: [f32; 2]) -> Option<Row> {
		self.row(number, Vector::zero(), [arm; 2], closing, range)
	}

	/// Returns the row with these parts, or None for one that cannot act in this step: one whose bodies no impulse
	/// moves the way it limits.
	fn row(&self, number: usize, direction: Vector, arms: [Vector; 2], closing: f32, range: [f32; 2]) -> Option<Row> {
		// An impulse changes the speed the row limits by the bodies' inverse masses, where the row limits how they
		// move, and by how easily each turns about its center of mass.
		let turns = [0, 1].map(|side| self.bodies[side].turn(arms[side]));
		let inverse_mass = [0, 1].map(|side| self.bodies[side].inverse_mass());
		let resistance =
			direction.length_squared() * (inverse_mass[0] + inverse_mass[1]) + arms[0].dot(turns[0]) + arms[1].dot(turns[1]);
		// Two bodies that nothing moves or turns so, or mass or inertia data that is not valid.
		if !resistance.is_finite() || resistance <= f32::EPSILON {
			return None;
		}
		Some(Row {
			place: self.place,
			number,
			slots: self.slots,
			direction,
			arms,
			turns,
			inverse_mass,
			mass: 1.0 / resistance,
			closing,
			range,
			applied: 0.0,
		})
	}

	/// Returns the speed that a row which stops a turn at an end of a limit leaves its bodies turning at. `past` is
	/// how far past that end the turn is, in radians, and `within` whether it is still within the limit.
	fn stop(&self, past: f32, within: bool) -> f32 {
		if within {
			// The bodies may turn as far as the end in this step, and no further. That stops them at the end instead
			// of a step's turn beyond it.
			self.speed_over(past)
		} else {
			self.take_up(past)
		}
	}

	/// Returns the speed that takes up a share of `off`, how far the bodies are from where a row holds them, in this
	/// step. It leaves alone what is within [`ERROR_SLACK`].
	fn take_up(&self, off: f32) -> f32 {
		let beyond_slack = off.signum() * (off.abs() - ERROR_SLACK).max(0.0);
		self.speed_over(beyond_slack * ERROR_SHARE)
	}

	/// Returns the speed that covers `distance` in this step. A step without length, as a paused world's is, has
	/// none.
	fn speed_over(&self, distance: f32) -> f32 {
		if self.dt > 0.0 { distance / self.dt } else { 0.0 }
	}
}

/// The `Pins` struct holds how a joint finds its two bodies turned to each other.
///
/// Each body has the joint's axis as a direction of its own, its pin. The two pins pointed the same way when the
/// joint first held the bodies, and what a joint allows is told by how they have come to differ since.
struct Pins {
	/// How the first body is turned.
	first: Orientation,
	/// The direction of the pin as the first body has it.
	axis: Vector<LocalSpace>,
	/// Where each body's pin points in the world.
	world: [Vector; 2],
	/// How far, in radians, the second body has turned about its pin since the joint first held the bodies,
	/// counterclockwise for someone the pin points at.
	twist: f32,
	/// What the bodies turn relative to each other about, and how fast for a radian a second of twist, when only
	/// the twist changes: the line halfway between the two pins. Limiting the twist about it is what makes one body
	/// of a constant velocity joint turn as fast about its pin as the other does about its own. None for pins that
	/// point opposite ways, whose halfway line is every line across them.
	twists_about: Option<Vector>,
}

impl Pins {
	/// Works out the pins of a joint about `axis`, a direction of the first body. `turned` is how the two bodies
	/// are turned, and `rest` how the second was as the first saw it when the joint first held them.
	fn new(turned: [Orientation; 2], rest: Orientation, axis: UnitVector<LocalSpace>) -> Self {
		// How the first body would be turned had it turned with the second one since the joint first held them.
		// Where that differs from how it is turned is what the joint has to allow, or to take up.
		let followed = turned[1].compose(rest.inverse());
		let axis = axis.into_vector();

		// The turn that takes the first body to there is a turn about the pin, followed by one that leans the pin
		// over. A unit quaternion holds the first of them as its part along the pin and its scalar part. Either
		// sign of a quaternion is the same turn, and the one with a positive scalar part is the short way around.
		let [x, y, z, w] = turned[0].inverse().compose(followed).to_array();
		let along = Vector::<LocalSpace>::new(x, y, z).dot(axis);
		let twist = 2.0 * (along * w.signum()).atan2(w.abs());

		let world = [to_world(turned[0], axis), to_world(followed, axis)];
		let alike = 1.0 + world[0].dot(world[1]);
		Self {
			first: turned[0],
			axis,
			world,
			twist,
			twists_about: (alike > f32::EPSILON).then(|| (world[0] + world[1]) / alike),
		}
	}
}

/// Returns where `local`, a direction of a body that is turned by `orientation`, points in the world.
fn to_world(orientation: Orientation, local: Vector<LocalSpace>) -> Vector {
	// Rotating a local direction into the world frame crosses a spatial math boundary.
	Vector::from_maths(orientation.into_maths() * local.into_maths())
}

/// Returns two unit vectors that are square to `axis`, a unit vector, and to each other.
fn across(axis: Vector<LocalSpace>) -> [Vector<LocalSpace>; 2] {
	// Crossing the axis with the axis of the frame that it leans on least gives a vector that is never close to
	// zero length.
	let [x, y, z] = axis.to_array().map(f32::abs);
	let least = if x <= y && x <= z {
		[1.0, 0.0, 0.0]
	} else if y <= z {
		[0.0, 1.0, 0.0]
	} else {
		[0.0, 0.0, 1.0]
	};
	let first = axis.cross(Vector::from_array(least));
	let first = first / first.length();
	[first, axis.cross(first)]
}

/// The `Row` struct holds what solving one row of a constraint needs, worked out once a step so that solving it
/// only reads and writes the motion of its two bodies.
///
/// A row limits one speed of its bodies relative to each other: how fast two anchors part along a direction, or how
/// fast the bodies turn relative to each other about one. A constraint is as many rows as it limits such speeds. All
/// of a step's rows make equations together, which are solved one row at a time, several times over.
struct Row {
	/// Where the row's constraint is among the world's, and which of that constraint's rows this is.
	place: usize,
	number: usize,
	/// The slots of the two bodies.
	slots: [usize; 2],
	/// The direction from the first body's anchor toward the second's that the row limits them along. Zero for a
	/// row that only limits how the bodies turn.
	direction: Vector,
	/// For each body, how its spin adds to the speed the row limits.
	arms: [Vector; 2],
	/// For each body, how a unit impulse of the row changes its spin.
	turns: [Vector; 2],
	inverse_mass: [f32; 2],
	/// The impulse that changes the speed the row limits by one.
	mass: f32,
	/// The speed the row should leave: the one the anchors close at, or the first body turns ahead of the second
	/// at. It takes up a share of what the bodies are off where the row holds them, so it is negative for anchors
	/// that are too close.
	closing: f32,
	/// The least and the most impulse the row may have applied. A row that stops its bodies one way only has zero
	/// for one of them.
	range: [f32; 2],
	/// The impulse applied so far that pulls the anchors together, or turns the first body on and the second back.
	/// Negative the other way.
	applied: f32,
}

impl Row {
	/// Solves the row once, changing the `motions` of its two bodies.
	///
	/// The bodies get the impulse that leaves the speed the row limits at the one it should leave, as far as the
	/// row's range of impulses allows. For a row whose bodies are where it holds them, that speed is none.
	fn solve(&mut self, motions: [&mut BodyMotion; 2]) {
		let ([first_linear, first_angular, _], [second_linear, second_angular, _]) = (motions[0].parts(), motions[1].parts());
		let parting_speed = self.direction.dot(second_linear - first_linear) + second_angular.dot(self.arms[1])
			- first_angular.dot(self.arms[0]);
		let wanted = self.applied + (parting_speed + self.closing) * self.mass;
		self.pull(motions, wanted.clamp(self.range[0], self.range[1]) - self.applied);
	}

	/// Applies `impulse`, which pulls the anchors together and turns the first body on and the second back about
	/// the row's arms, or does the reverse when it is negative.
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
