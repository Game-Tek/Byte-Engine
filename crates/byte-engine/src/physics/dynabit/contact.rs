use math::{Point, UnitVector, collision::DynamicIntersection, collision::Intersection};

/// The `Contact` struct stores the world-space data required to resolve one collision.
#[derive(Debug)]
pub struct Contact {
	pub(crate) a: Side,
	pub(crate) b: Side,
	pub(crate) normal: UnitVector,
	pub(crate) depth: f32,
	/// How far into the step the bodies touch, in seconds. Zero for bodies that touch where they are. For bodies
	/// that touch later, the contact holds them to the speed that brings them together as the step ends.
	pub(crate) toi: f32,
}

impl Contact {
	/// Converts a static math intersection into a physics contact between bodies that touch where they are.
	pub(crate) fn from_static(a: usize, b: usize, intersection: Intersection) -> Self {
		Self {
			a: Side {
				object: a,
				point: intersection.point_on_a(),
			},
			b: Side {
				object: b,
				point: intersection.point_on_b(),
			},
			normal: intersection.normal(),
			depth: intersection.depth(),
			toi: 0.0,
		}
	}

	/// Converts a dynamic math intersection into a physics contact at the time the bodies touch.
	pub(crate) fn from_dynamic(a: usize, b: usize, intersection: DynamicIntersection) -> Self {
		let toi = intersection.toi();
		Self {
			toi,
			..Self::from_static(a, b, intersection.into_contact())
		}
	}
}

/// The `Side` struct identifies one body and its world-space contact point.
#[derive(Debug)]
pub struct Side {
	pub(crate) object: usize,
	pub(crate) point: Point,
}
