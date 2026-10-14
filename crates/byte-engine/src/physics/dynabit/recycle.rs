//! Contact recycling for Dynabit: reusing the contacts that were found between two bodies in an earlier step for as
//! long as the bodies stay put relative to each other.
//!
//! Finding the contacts of a pair of bodies whose shapes are turned the way their bodies are, or that has a hull,
//! takes a search over both shapes. Most such pairs in a pile hardly move from one step to the next, so their
//! contacts are almost the ones they had. [`Manifold`] keeps what such a search found, and [`Manifold::recycle`] gives
//! those contacts again, moved with the bodies, while that is close enough.
//!
//! This follows Box2D's contact recycling (<https://box2d.org/posts/2026/10/contact-recycling/>). A recycled contact
//! keeps its normal and its points' offsets from their bodies as they are in the world, so its points move with the
//! bodies but do not turn with them. How deep the bodies are in each other there follows from how far each body
//! moved since the contacts were found, with the offsets turned the way the bodies turned since then.

use math::{Orientation, Point, UnitVector, Vector};
use utils::hash::HashMap;

use crate::{
	core::factory::Handle,
	physics::dynabit::{
		body::{PhysicsBody, half_turn_squared},
		contact::{Contact, Side},
		solver::pair_key,
	},
};

/// How far, in meters, two bodies may move relative to each other after their contacts were found for the contacts
/// to be recycled. It counts how far one moves as the other sees it, and how far the furthest point of either moves
/// as it turns relative to the other.
const RECYCLE_REACH: f32 = 0.02;

/// How far, in radians, either body of a pair may turn after its contacts were found for them to be recycled. A
/// recycled contact's normal does not turn with the bodies, so this keeps it from leaning too far from where the
/// bodies now face each other.
const RECYCLE_TURN: f32 = 0.05;

/// The `Manifold` struct keeps the contacts that were last found fresh between two bodies, with where the bodies
/// were then, so that the steps after it can recycle those contacts while the bodies stay put relative to each other.
///
/// Make one with [`Self::of`] from the contacts a search found, keep it in a [`ManifoldPart`], and call
/// [`Self::recycle`] in later steps instead of searching again.
#[derive(Clone, Copy)]
pub(crate) struct Manifold {
	/// The handles of the two bodies, in the order the contacts name them. A slot that another body has taken since
	/// does not get these contacts.
	handles: [Handle; 2],
	/// Where each body was, and how it was turned, when the contacts were found.
	places: [(Point, Orientation); 2],
	/// Where the second body was then as the first one saw it, and how it was turned to it.
	seen: (Vector, Orientation),
	/// How far from its position the shape of the larger of the bodies that have mass reaches.
	reach: f32,
	corners: [Corner; 4],
	/// How many of `corners` the bodies touched at.
	count: u8,
}

/// The `Corner` struct keeps one contact of a [`Manifold`].
#[derive(Clone, Copy)]
struct Corner {
	/// Where the contact's point is on each body, from the body's position, as it was in the world when the contact
	/// was found.
	offsets: [Vector; 2],
	/// The normal, from the first body toward the second.
	normal: UnitVector,
	/// How deep the bodies were in each other there when the contact was found.
	depth: f32,
	/// Which of the pair's contacts this is.
	number: u8,
}

impl Manifold {
	/// Returns the manifold of `contacts`, which were just found between `a` and `b` where they are, and name `a`
	/// first. Bodies touch at four places at most.
	pub(crate) fn of(a: &PhysicsBody, b: &PhysicsBody, contacts: &[Contact]) -> Self {
		let reach = [a, b]
			.iter()
			.filter(|body| body.inv_mass != 0.0)
			.map(|body| body.reach_squared([0.0; 3]).sqrt())
			.fold(0.0, f32::max);
		debug_assert!(
			(1..=4).contains(&contacts.len()),
			"Manifold contact count is invalid. The most likely cause is a pair that a search found no contacts or more than four contacts for."
		);
		let corner = |contact: &Contact| Corner {
			offsets: [contact.a.point - a.position, contact.b.point - b.position],
			normal: contact.normal,
			depth: contact.depth,
			number: contact.corner,
		};
		Self {
			handles: [a.handle, b.handle],
			places: [(a.position, a.orientation), (b.position, b.orientation)],
			seen: seen(a, b),
			reach,
			corners: std::array::from_fn(|index| corner(&contacts[index.min(contacts.len() - 1)])),
			count: contacts.len() as u8,
		}
	}

	/// Adds the manifold's contacts between `a`, in slot `i`, and `b`, in slot `j`, to `contacts`, moved with the
	/// bodies, and returns `true`. Returns `false` and adds nothing when the bodies are not the manifold's, in its
	/// order, or have moved or turned too far since its contacts were found, which then have to be found again.
	pub(crate) fn recycle(
		&self,
		(a, i): (&PhysicsBody, usize),
		(b, j): (&PhysicsBody, usize),
		contacts: &mut Vec<Contact>,
	) -> bool {
		if self.handles != [a.handle, b.handle] {
			return false;
		}
		let [(a_place, a_turn), (b_place, b_turn)] = self.places;
		let most = (RECYCLE_TURN * 0.5) * (RECYCLE_TURN * 0.5);
		if half_turn_squared(a.orientation, a_turn) > most || half_turn_squared(b.orientation, b_turn) > most {
			return false;
		}
		let ((now_offset, now_turn), (then_offset, then_turn)) = (seen(a, b), self.seen);
		// The sine of half a small angle is as good as half of it.
		let turned = 2.0 * half_turn_squared(now_turn, then_turn).sqrt();
		if (now_offset - then_offset).length() + turned * self.reach > RECYCLE_REACH {
			return false;
		}

		// How each body turned and moved since the contacts were found.
		let turned = [
			a.orientation.compose(a_turn.inverse()),
			b.orientation.compose(b_turn.inverse()),
		];
		let moved = [a.position - a_place, b.position - b_place];
		for corner in &self.corners[..self.count as usize] {
			// A point of each body moves as the body does, and as its offset turns with it.
			let shift = |side: usize| moved[side] + turned[side].rotate_vector(corner.offsets[side]) - corner.offsets[side];
			contacts.push(Contact {
				a: Side {
					object: i,
					point: a.position + corner.offsets[0],
				},
				b: Side {
					object: j,
					point: b.position + corner.offsets[1],
				},
				normal: corner.normal,
				depth: corner.depth - (shift(1) - shift(0)).dot(corner.normal.into()),
				toi: 0.0,
				corner: corner.number,
			});
		}
		true
	}
}

/// Returns where `b` is as `a` sees it, and how it is turned to it.
fn seen(a: &PhysicsBody, b: &PhysicsBody) -> (Vector, Orientation) {
	let inverse = a.orientation.inverse();
	(inverse.rotate_vector(b.position - a.position), inverse.compose(b.orientation))
}

/// Returns whether `a` and `b` move apart from each other so fast that they leave what a manifold of theirs allows
/// within one step of `dt` seconds. Their contacts cannot be recycled in this step or the next, so they are not
/// worth keeping.
pub(crate) fn outpaces(a: &PhysicsBody, b: &PhysicsBody, dt: f32) -> bool {
	(a.linear_velocity - b.linear_velocity).length_squared() * dt * dt > RECYCLE_REACH * RECYCLE_REACH
}

/// The `ManifoldPart` struct keeps the manifolds that one lane made or recycled in a step, so that the next step can
/// look them up by pair.
///
/// A lane fills its own part while it searches for contacts, with [`Self::keep`]. In the next step, every lane
/// reads every part with [`find`].
#[derive(Default)]
pub(crate) struct ManifoldPart {
	manifolds: Vec<Manifold>,
	/// Where each pair's manifold is in `manifolds`, by the [`pair_key`] of its bodies' slots. A map of the
	/// manifolds themselves made steps of a large pile 5% slower, as its entries are many times larger.
	places: HashMap<u64, u32>,
}

impl ManifoldPart {
	/// Keeps `manifold` of the pair of bodies in `slots` for the next step.
	pub(crate) fn keep(&mut self, slots: [usize; 2], manifold: Manifold) {
		self.places.insert(pair_key(slots, 0), self.manifolds.len() as u32);
		self.manifolds.push(manifold);
	}

	/// Empties the part, ready for the manifolds of another step. It keeps its storage.
	pub(crate) fn clear(&mut self) {
		self.manifolds.clear();
		self.places.clear();
	}

	fn get(&self, key: u64) -> Option<&Manifold> {
		Some(&self.manifolds[*self.places.get(&key)? as usize])
	}
}

/// Returns the manifold that one of `parts` keeps for the pair of bodies in `slots`. The part at `lane`, which
/// belongs to the lane that searches for the pair's contacts now, most likely has it, and is looked at first.
///
/// A pair's manifold is found wherever it was kept, since which lane finds a pair depends on how long lanes took in
/// the last step, and the contacts a pair gets must not. On an Apple M4, looking only through the parts of the
/// lanes next to `lane` made steps of a large pile that was still falling 2% faster, but let a pair's contacts
/// depend on that. Telling from marks on the bodies which parts to look through made them no faster.
pub(crate) fn find(parts: &[ManifoldPart], lane: usize, slots: [usize; 2]) -> Option<&Manifold> {
	let key = pair_key(slots, 0);
	let (before, after) = parts.split_at(lane.min(parts.len()));
	after.iter().chain(before).find_map(|part| part.get(key))
}

#[cfg(test)]
mod tests {
	use math::{Scale, WorldSpace};

	use super::*;
	use crate::{
		core::factory::Factory,
		physics::{BodyTypes, collider::Shapes, dynabit::body::intersect, epa::ContactScratch},
	};

	/// Returns a one-meter box that rotates, with its middle at `position`, turned by `turned`.
	fn turning_box(position: Point, turned: Orientation) -> PhysicsBody {
		PhysicsBody {
			body_type: BodyTypes::Dynamic,
			collision_shape: Shapes::Cube {
				size: Vector::new(0.5, 0.5, 0.5),
			},
			position,
			orientation: turned,
			scale: Scale::identity(),
			linear_velocity: Vector::zero(),
			angular_velocity: Vector::zero(),
			inv_mass: 1.0,
			center_of_mass: Point::origin(),
			elasticity: 0.0,
			friction: 0.5,
			rotates: true,
			handle: Factory::<()>::new().create(()),
			rest: Default::default(),
		}
	}

	/// Returns `body` moved to `position` and turned by `turned`.
	fn placed(body: &PhysicsBody, position: Point, turned: Orientation) -> PhysicsBody {
		PhysicsBody {
			handle: body.handle,
			..turning_box(position, turned)
		}
	}

	fn about_y(radians: f32) -> Orientation {
		Orientation::try_from_rotation_vector(Vector::<WorldSpace>::new(0.0, radians, 0.0)).unwrap()
	}

	/// Returns the contacts a search finds between `a` and `b`, which name `a` first.
	fn searched(a: &PhysicsBody, b: &PhysicsBody) -> Vec<Contact> {
		let mut contacts = Vec::new();
		intersect((a, 0), (b, 1), 1.0 / 60.0, &mut contacts, &mut ContactScratch::default());
		contacts
	}

	/// Returns the most that `contacts` reach into their bodies.
	fn deepest(contacts: &[Contact]) -> f32 {
		contacts.iter().map(|contact| contact.depth).fold(f32::MIN, f32::max)
	}

	/// Returns a box with another one lying on it a centimeter deep, turned a little the other way, and the
	/// manifold of the patch they touch over.
	fn stacked() -> (PhysicsBody, PhysicsBody, Manifold) {
		let lower = turning_box(Point::origin(), about_y(0.05));
		let upper = turning_box(Point::new(0.0, 0.99, 0.0), about_y(-0.1));
		let contacts = searched(&lower, &upper);
		assert_eq!(contacts.len(), 4, "the boxes touch over a patch with four corners");
		let manifold = Manifold::of(&lower, &upper, &contacts);
		(lower, upper, manifold)
	}

	#[test]
	fn recycled_contacts_reach_as_deep_as_contacts_searched_for_where_the_bodies_moved() {
		let (lower, mut upper, manifold) = stacked();
		// The upper box sinks two millimeters, slides a little, and tips by a hundredth of a radian.
		upper.position += Vector::new(0.004, -0.002, -0.003);
		let tipped = Orientation::try_from_rotation_vector(Vector::<WorldSpace>::new(0.01, 0.0, 0.0)).unwrap();
		upper.orientation = tipped.compose(upper.orientation);

		let mut recycled = Vec::new();
		assert!(manifold.recycle((&lower, 0), (&upper, 1), &mut recycled));
		// A search finds the tipped box on one edge, where the recycled patch still holds it at four corners, so
		// only the deepest contacts compare.
		let searched = searched(&lower, &upper);
		let (most, fresh_most) = (deepest(&recycled), deepest(&searched));
		assert!(
			(most - fresh_most).abs() <= 0.001,
			"recycled contacts reach {most} deep, and searched ones {fresh_most}"
		);
		assert_eq!(recycled.len(), 4);
		assert!(recycled.iter().all(|contact| [contact.a.object, contact.b.object] == [0, 1]));
	}

	#[test]
	fn contacts_are_not_recycled_for_bodies_that_moved_or_turned_too_far_or_other_bodies_or_order() {
		let (lower, upper, manifold) = stacked();
		let recycles = |a: &PhysicsBody, b: &PhysicsBody| manifold.recycle((a, 0), (b, 1), &mut Vec::new());

		let slid = placed(&upper, upper.position + Vector::new(0.03, 0.0, 0.0), upper.orientation);
		assert!(!recycles(&lower, &slid));
		// Both boxes turned together keep where they are to each other, but the normal would lean with them.
		let tilt = Orientation::try_from_rotation_vector(Vector::<WorldSpace>::new(0.1, 0.0, 0.0)).unwrap();
		let tilted_lower = placed(&lower, lower.position, tilt.compose(lower.orientation));
		let tilted_upper = placed(
			&upper,
			Point::origin() + tilt.rotate_vector(upper.position - Point::origin()),
			tilt.compose(upper.orientation),
		);
		assert!(!recycles(&tilted_lower, &tilted_upper));
		// A body that took the slot of the upper box is another body, and the manifold names its bodies in one
		// order.
		let other = turning_box(upper.position, upper.orientation);
		assert!(!recycles(&lower, &other));
		assert!(!recycles(&upper, &lower));
	}
}
