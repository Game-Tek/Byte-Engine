//! Sleep for Dynabit: which bodies have come to rest, and how they wake.
//!
//! A step does nothing for a body that sleeps: it does not pull it down, test it against other bodies that sleep or
//! that nothing moves, move it, or publish where it is. To the bodies that are awake, a sleeping body is as fixed
//! as a body without mass, which they can lie on or lean against. [`Sleep`] decides which bodies sleep.
//!
//! Each body sleeps on its own, so the parts of a pile that have come to rest sleep while other parts still move.
//! A body that moves wakes the sleeping bodies it comes near, before it can push into them or leave them without
//! what they lie on. Bodies that a constraint holds together, through any number of others, sleep and wake as one
//! island, because a constraint pulls on both of its bodies.
//!
//! [`Sleep`] watches the bodies that are awake for [`REST_TIME`] at a time. A body that stayed where the watch found
//! it goes to sleep when the watch ends, with its island. The bodies of a sleeping island name each other in a ring,
//! with [`Rest::Asleep`], so that waking one of them wakes them all.

use math::{Orientation, Point, Vector};
use utils::StableVec;

use crate::physics::dynabit::body::{PhysicsBody, Rest, half_turn_squared};

/// How long one watch lasts, in seconds, which is how long a body has to stay where it is before it sleeps.
///
/// Under the world's gravity, a body in the air stays within [`REST_REACH`] of one place for less than a sixth of a
/// second, so no body goes to sleep in the air.
const REST_TIME: f32 = 0.5;

/// How far, in meters, a body may move from where a watch found it and still be at rest.
///
/// A body that moves slower than this in a watch, five centimeters a second, stops, so it hardly shows. A pendulum
/// a meter long that swings less far than this stops a degree and a half from straight down at most.
///
const REST_REACH: f32 = 0.025;

/// How far, in radians, a body may turn from how a watch found it and still be at rest. A one-meter box that turns
/// this far moves the middles of its sides as far as [`REST_REACH`].
const REST_TURN: f32 = 0.05;

/// How fast, in meters a second, a point of a body that is awake has to move for the body to wake the sleeping
/// bodies it comes near. A body that has strayed from where the current watch found it wakes them however fast it
/// is.
///
/// A slower body is as good as at rest to the bodies that sleep near it: one that comes to rest on a sleeping body
/// lies on it as on the ground, and one that pushes into it slower than this is stopped by it until it strays.
/// Bodies that are at rest in a pile still move this fast now and then, for a step, as their contacts close the
/// small gaps between them, and every time one does, it wakes the bodies around it. With a thousand boxes dropped
/// ten layers deep, a fifth of this kept two thirds of the pile awake for good, half of it let the pile sleep 22
/// seconds after the drop, and this let it sleep after 10.
///
/// A body that loses what it lies on reaches this speed in its second step of falling, so what lies on it wakes in
/// the step after that.
pub(crate) const WAKE_SPEED: f32 = 0.5;

/// The `Sleep` struct keeps what a physics world needs to let the bodies that have come to rest sleep.
///
/// During a step, ask [`Self::strays`] of every body that moved, and keep the answer in the body's [`Rest`]. After
/// a step, call [`Self::watch`], and [`Self::settle`] when it says that the watch has ended. Call [`Self::wake`]
/// for a sleeping body that something is about to move.
#[derive(Default)]
pub(crate) struct Sleep {
	/// Where each body was, and how it was turned, when the current watch began, by body slot.
	places: Vec<(Point, Orientation)>,
	/// How long the current watch has lasted, in seconds.
	watched: f32,
	/// How many bodies are asleep.
	asleep: usize,
	islands: Islands,
	/// The slots of the bodies that [`Self::settle`] is putting to sleep. Empty between calls, and kept so that
	/// they do not allocate.
	settled: Vec<u32>,
}

impl Sleep {
	/// Returns how many bodies are asleep. While none are, nothing needs to be woken.
	pub(crate) fn asleep(&self) -> usize {
		self.asleep
	}

	/// Returns whether `body`, the one in `slot`, has left where the current watch found it: moved further than
	/// [`REST_REACH`] from there, or turned further than [`REST_TURN`]. A body that the watch did not see has.
	///
	/// A body that only shakes where it lies stays within both, where its speed alone would not tell it from one
	/// that travels.
	#[inline]
	pub(crate) fn strays(&self, slot: usize, body: &PhysicsBody) -> bool {
		let Some((position, orientation)) = self.places.get(slot) else {
			return true;
		};
		(body.position - *position).length_squared() > REST_REACH * REST_REACH
			|| half_turn_squared(body.orientation, *orientation) > (REST_TURN * 0.5) * (REST_TURN * 0.5)
	}

	/// Counts a step of `seconds` seconds toward the current watch. Returns whether the watch ends with that step,
	/// which is when to call [`Self::settle`].
	pub(crate) fn watch(&mut self, seconds: f32) -> bool {
		self.watched += seconds;
		let ended = self.watched >= REST_TIME;
		if ended {
			self.watched = 0.0;
		}
		ended
	}

	/// Ends a watch: puts every body to sleep that stayed at rest through it, with the other bodies of its island,
	/// and calls `slept` with the slot of each body that it put to sleep and that body. Then begins the next watch
	/// of the bodies that are still awake. Does nothing unless `allowed`.
	///
	/// `held` returns the slots of the two bodies of every constraint, which make islands of the bodies that are
	/// awake and have mass. It is not asked for when no body is at rest.
	pub(crate) fn settle(
		&mut self,
		bodies: &mut StableVec<PhysicsBody>,
		allowed: bool,
		held: impl Iterator<Item = [usize; 2]>,
		mut slept: impl FnMut(usize, &PhysicsBody),
	) {
		// While bodies may not sleep, they stay strayed, so there is no watch to begin.
		if !allowed {
			return;
		}
		let rested = |body: &PhysicsBody| {
			body.inv_mass != 0.0
				&& matches!(
					body.rest,
					Rest::Awake {
						strayed: false,
						woke: false,
						..
					}
				)
		};
		// Most watches of a world in motion end with no body at rest, and so with no island to find.
		if bodies.iter().any(rested) {
			let active = |slot: usize| bodies.get_slot(slot).is_some_and(|body| body.active_since() == 0);
			let mut held = held.filter(|slots| slots.iter().all(|slot| active(*slot))).peekable();
			// Without constraints between bodies that are awake, every body is an island of its own.
			let islands = held.peek().is_some();
			if islands {
				self.islands.reset(bodies.slots_len());
				for [first, second] in held {
					self.islands.join(first, second);
				}
				for (slot, body) in bodies.indexed_iter() {
					if body.active_since() != PhysicsBody::NEVER_ACTIVE && !rested(body) {
						self.islands.disturb(slot);
					}
				}
			}

			for (slot, body) in bodies.indexed_iter_mut() {
				if rested(body) && (!islands || self.islands.rests(slot)) {
					// Each body starts as an island of its own, and the next pass joins it to the others. What is
					// left of its creeping would carry it off when it wakes.
					body.rest = Rest::Asleep { next: slot as u32 };
					(body.linear_velocity, body.angular_velocity) = (Vector::zero(), Vector::zero());
					slept(slot, body);
					self.settled.push(slot as u32);
				}
			}
			self.asleep += self.settled.len();
			for slot in self.settled.drain(..) {
				// The root of an island is one of its bodies. Every other one goes into the ring right after it.
				let root = if islands {
					self.islands.root(slot as usize) as u32
				} else {
					slot
				};
				if root == slot {
					continue;
				}
				let after = bodies
					.get_slot_mut(root as usize)
					.map(|body| std::mem::replace(&mut body.rest, Rest::Asleep { next: slot }));
				if let (Some(after), Some(body)) = (after, bodies.get_slot_mut(slot as usize)) {
					body.rest = after;
				}
			}
		}

		self.places
			.resize(bodies.slots_len(), (Point::origin(), Orientation::identity()));
		for (slot, body) in bodies.indexed_iter_mut() {
			if let Rest::Awake { strayed, woke, .. } = &mut body.rest {
				(*strayed, *woke) = (false, false);
				self.places[slot] = (body.position, body.orientation);
			}
		}
	}

	/// Wakes the body in `slot` and every body of the island it sleeps with, and calls `woken` with the slot of
	/// each body that it woke and that body. Does nothing for a body that is awake.
	///
	/// `since` is what [`PhysicsBody::active_since`] returns for those bodies from now on: zero between steps, and
	/// during a step the round after the one that found what wakes them.
	pub(crate) fn wake(
		&mut self,
		bodies: &mut StableVec<PhysicsBody>,
		mut slot: usize,
		since: u32,
		mut woken: impl FnMut(usize, &mut PhysicsBody),
	) {
		// The ring leads back to the first body, which is awake by then.
		while let Some(body) = bodies.get_slot_mut(slot) {
			let Rest::Asleep { next } = body.rest else {
				break;
			};
			// A body that was woken has a whole watch ahead of it before it sleeps again, and strays from where it
			// woke, which is where its author may just have put it.
			body.rest = Rest::Awake {
				strayed: false,
				woke: true,
				since,
				stirs: false,
			};
			if let Some(place) = self.places.get_mut(slot) {
				*place = (body.position, body.orientation);
			}
			self.asleep -= 1;
			woken(slot, body);
			slot = next as usize;
		}
	}
}

/// The `Islands` struct sorts bodies into islands, the groups that [`Sleep`] puts to sleep together, and tells
/// which islands are at rest.
///
/// Call [`Self::reset`], then [`Self::join`] for every two bodies that belong together, then [`Self::disturb`] for
/// every body that is not at rest. After that, [`Self::rests`] tells whether a body's island is.
#[derive(Default)]
struct Islands {
	/// For each body slot, the slot of another body of its island that is nearer the island's root, or the slot
	/// itself for the root.
	parents: Vec<u32>,
	/// For each root's slot, whether a body of its island is not at rest.
	restless: Vec<bool>,
}

impl Islands {
	/// Makes every one of `slots` body slots an island of its own, at rest.
	fn reset(&mut self, slots: usize) {
		self.parents.clear();
		self.parents.extend(0..slots as u32);
		self.restless.clear();
		self.restless.resize(slots, false);
	}

	/// Returns the root of the island of the body in `slot`, which is the same slot for every body of an island.
	fn root(&mut self, mut slot: usize) -> usize {
		loop {
			let parent = self.parents[slot] as usize;
			if parent == slot {
				return slot;
			}
			// Each look halves the way to the root for the next one.
			self.parents[slot] = self.parents[parent];
			slot = self.parents[slot] as usize;
		}
	}

	/// Makes one island of the islands of the bodies in the slots `first` and `second`.
	fn join(&mut self, first: usize, second: usize) {
		let (first, second) = (self.root(first), self.root(second));
		self.parents[first] = second as u32;
	}

	/// Marks the island of the body in `slot` as not at rest. Call it once every island is joined.
	fn disturb(&mut self, slot: usize) {
		let root = self.root(slot);
		self.restless[root] = true;
	}

	/// Returns whether every body of the island of the body in `slot` is at rest.
	fn rests(&mut self, slot: usize) -> bool {
		let root = self.root(slot);
		!self.restless[root]
	}
}
