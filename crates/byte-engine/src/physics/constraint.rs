use std::{f32::consts::PI, ops::RangeInclusive};

use math::{Point, Radians, UnitVector};

use crate::{core::factory::Handle, physics::LocalSpace};

/// The `Constraint` struct describes a limit on how two physics bodies may move relative to each other, such as a rod
/// between them or a joint that connects them.
///
/// Make one with [`Self::distance`], [`Self::hinge`], [`Self::constant_velocity`] or [`Self::ball`] and send it through a constraint factory, as a
/// [`crate::physics::Body`] is sent through a body factory. The physics world holds the two bodies to it in every
/// step that both are in the world, so the constraint and its bodies can be created in any order. Delete the handle
/// it was created with to let the bodies go.
///
/// Bodies that a constraint holds still collide with each other. Shape and place them so that they do not touch in
/// any pose the constraint allows.
#[derive(Debug, Clone)]
pub struct Constraint {
	bodies: [Handle; 2],
	anchors: [Point<LocalSpace>; 2],
	kind: Kind,
}

/// The `Kind` enum tells what a [`Constraint`] holds its two bodies to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
	/// The anchors stay `length` meters apart. See [`Constraint::distance`].
	Distance { length: f32 },
	/// The anchors stay at the same place, and the bodies only turn relative to each other about `axis`, a direction
	/// of the first body, and no further than from the first to the second angle of `limit`. See
	/// [`Constraint::hinge`].
	Hinge {
		axis: UnitVector<LocalSpace>,
		limit: Option<[Radians; 2]>,
	},
	/// The anchors stay at the same place, and the second body may lean away from the first any way, by no more
	/// than `limit`, but not turn relative to it about `axis`, a direction of the first body. See
	/// [`Constraint::constant_velocity`].
	ConstantVelocity {
		axis: UnitVector<LocalSpace>,
		limit: Option<Radians>,
	},
	/// The anchors stay at the same place, and the second body turns any way about them. `axis`, a direction of
	/// the first body, is what the limits count from: the second body may lean no further than `lean` away from
	/// it, and turn about it no further than from the first to the second angle of `twist`. See
	/// [`Constraint::ball`].
	Ball {
		axis: UnitVector<LocalSpace>,
		lean: Option<Radians>,
		twist: Option<[Radians; 2]>,
	},
}

impl Constraint {
	/// Creates a constraint that keeps the anchors of two bodies `length` meters apart, as a stiff rod between them
	/// would. The bodies stay free to turn about their anchors.
	///
	/// `first` and `second` are the handles the bodies were created with. Each anchor starts at its body's origin;
	/// move them with [`Self::with_anchors`].
	///
	/// The rod only acts along its own line. With a `length` of zero it has no line until the anchors have come
	/// apart, so it follows them about as far behind as they move in a step, and does not pin them to each other.
	pub fn distance(first: Handle, second: Handle, length: f32) -> Self {
		debug_assert!(
			length.is_finite() && length >= 0.0,
			"Constraint length is invalid. The most likely cause is a length that is negative or was divided by zero."
		);
		Self::new(first, second, Kind::Distance { length })
	}

	/// Creates a constraint that joins two bodies as a hinge does: their anchors stay at the same place, and the
	/// second body only turns relative to the first about `axis`, as a door does on its frame or a forearm at the
	/// elbow.
	///
	/// `first` and `second` are the handles the bodies were created with. `axis` is the direction of the hinge's pin
	/// as the first body has it, turned as that body is. Each anchor starts at its body's origin; move them with
	/// [`Self::with_anchors`] to where the pin goes through each body.
	///
	/// The hinge keeps the bodies turned to each other as they are in the first step that the world has both, apart
	/// from the turn about its pin. So create the bodies in a pose the hinge allows, with their anchors at the same
	/// place.
	///
	/// `limit` is how far the second body may turn about the pin from that pose: no further back than the angle the
	/// range starts at, and no further on than the one it ends at. Angles count counterclockwise for someone the
	/// axis points at, and are between a half turn back and a half turn on. A hinge without a limit turns freely.
	pub fn hinge(first: Handle, second: Handle, axis: UnitVector<LocalSpace>, limit: Option<RangeInclusive<Radians>>) -> Self {
		let limit = limit.map(turn_limit);
		Self::new(first, second, Kind::Hinge { axis, limit })
	}

	/// Creates a constraint that joins two bodies as a constant velocity joint does the two halves of a drive
	/// shaft: their anchors stay at the same place, and the second body may lean away from the first any way, but
	/// not twist relative to it. So when one body turns about the joint's axis, the other turns as fast about its
	/// own, however the joint is bent. For a joint that lets its bodies twist as well, see [`Self::ball`].
	///
	/// `first` and `second` are the handles the bodies were created with. `axis` is the direction of the shaft as
	/// the first body has it, turned as that body is, which is the direction that the bodies may not twist about.
	/// Each anchor starts at its body's origin; move them with [`Self::with_anchors`] to where the joint is in each
	/// body.
	///
	/// The joint counts the bodies as straight and without twist as they are in the first step that the world has
	/// both. So create the bodies in that pose, with their anchors at the same place.
	///
	/// `limit` is the angle, of no more than a half turn, that the second body may lean away from that pose, the
	/// same in every direction. A joint without a limit bends freely.
	pub fn constant_velocity(first: Handle, second: Handle, axis: UnitVector<LocalSpace>, limit: Option<Radians>) -> Self {
		let limit = limit.map(lean_limit);
		Self::new(first, second, Kind::ConstantVelocity { axis, limit })
	}

	/// Creates a constraint that joins two bodies as a ball in a socket does: their anchors stay at the same
	/// place, and the second body turns any way about them, as far as the limits allow. A shoulder or a hip is such
	/// a joint: an arm swings any way and turns some about its own length.
	///
	/// `first` and `second` are the handles the bodies were created with. Each anchor starts at its body's origin;
	/// move them with [`Self::with_anchors`] to where the ball is in each body.
	///
	/// `axis` is the direction that the limits count from, as the first body has it and turned as that body is.
	/// Point it along the length of the limb that the second body is. A joint without limits does not use it.
	///
	/// The joint counts the second body as not leaning and not turned as the bodies are in the first step that the
	/// world has both. So create the bodies in that pose, with their anchors at the same place.
	///
	/// `lean` is the angle, of no more than a half turn, that the second body may lean away from that pose, the
	/// same in every direction. `twist` is how far it may turn about the axis as it has it: no further back than
	/// the angle the range starts at, and no further on than the one it ends at. Angles count counterclockwise for
	/// someone the axis points at, and are between a half turn back and a half turn on. Without either, the joint
	/// turns freely that way. A joint that is folded all the way back on itself has no twist to tell, so set `lean`
	/// when you set `twist`.
	pub fn ball(
		first: Handle,
		second: Handle,
		axis: UnitVector<LocalSpace>,
		lean: Option<Radians>,
		twist: Option<RangeInclusive<Radians>>,
	) -> Self {
		let (lean, twist) = (lean.map(lean_limit), twist.map(turn_limit));
		Self::new(first, second, Kind::Ball { axis, lean, twist })
	}

	fn new(first: Handle, second: Handle, kind: Kind) -> Self {
		Self {
			bodies: [first, second],
			anchors: [Point::origin(); 2],
			kind,
		}
	}

	/// Returns this constraint with replacement anchors: the point of each body that the constraint holds, relative
	/// to the body's origin and turned as the body is.
	pub fn with_anchors(self, first: Point<LocalSpace>, second: Point<LocalSpace>) -> Self {
		Self {
			anchors: [first, second],
			..self
		}
	}

	/// Returns the handles of the two bodies the constraint holds.
	pub fn bodies(&self) -> [Handle; 2] {
		self.bodies
	}

	/// Returns the point of each body that the constraint holds, relative to the body's origin.
	pub fn anchors(&self) -> [Point<LocalSpace>; 2] {
		self.anchors
	}

	/// Returns what the constraint holds its two bodies to.
	pub fn kind(&self) -> Kind {
		self.kind
	}
}

/// Returns the angles that a limit on a turn about an axis starts and ends at.
fn turn_limit(limit: RangeInclusive<Radians>) -> [Radians; 2] {
	let [back, on] = [*limit.start(), *limit.end()];
	debug_assert!(
		-PI <= back.value() && back <= on && on.value() <= PI,
		"Turn limit is invalid. The most likely cause is a range that ends before it starts, or an angle given in degrees."
	);
	[back, on]
}

/// Returns `limit`, the angle that a body may lean away from an axis, once it is checked.
fn lean_limit(limit: Radians) -> Radians {
	debug_assert!(
		(0.0..=PI).contains(&limit.value()),
		"Lean limit is invalid. The most likely cause is a negative angle, or an angle given in degrees."
	);
	limit
}
