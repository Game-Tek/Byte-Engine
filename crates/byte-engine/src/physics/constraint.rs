use math::Point;

use crate::{core::factory::Handle, physics::LocalSpace};

/// The `Constraint` struct describes a limit on how two physics bodies may move relative to each other, such as a rod
/// between them.
///
/// Make one with [`Self::distance`] and send it through a constraint factory, as a [`crate::physics::Body`] is sent
/// through a body factory. The physics world holds the two bodies to it in every step that both are in the world, so
/// the constraint and its bodies can be created in any order. Delete the handle it was created with to let the bodies
/// go.
#[derive(Debug, Clone)]
pub struct Constraint {
	bodies: [Handle; 2],
	anchors: [Point<LocalSpace>; 2],
	length: f32,
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
		Self {
			bodies: [first, second],
			anchors: [Point::origin(); 2],
			length,
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

	/// Returns how far apart the constraint keeps the two anchors, in meters.
	pub fn length(&self) -> f32 {
		self.length
	}
}
