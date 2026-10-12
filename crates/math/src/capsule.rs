use crate::{AABB, Point, Vector, WorldSpace};

/// The `Capsule` struct represents every point within a radius of a line segment in one coordinate space, for
/// collision queries against rounded, elongated volumes such as a standing character.
///
/// Pass it to [`crate::collision::capsule_vs_sphere`], [`crate::collision::capsule_vs_capsule`], or
/// [`crate::collision::capsule_vs_aabb`] to find where it touches another shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Capsule<Space = WorldSpace> {
	start: Point<Space>,
	end: Point<Space>,
	radius: f32,
}

impl<Space> Capsule<Space> {
	/// Creates a capsule around the segment from `start` to `end`.
	///
	/// `start` and `end` are the centers of the two round caps, so the capsule reaches `radius` past each of them.
	/// Equal points give a sphere.
	pub fn new(start: Point<Space>, end: Point<Space>, radius: f32) -> Self {
		Self { start, end, radius }
	}

	/// Returns the center of the first cap.
	pub fn start(&self) -> Point<Space> {
		self.start
	}

	/// Returns the center of the second cap.
	pub fn end(&self) -> Point<Space> {
		self.end
	}

	/// Returns the distance from the segment to the surface.
	pub fn radius(&self) -> f32 {
		self.radius
	}

	/// Returns the smallest axis-aligned box that holds this capsule.
	pub fn bounds(&self) -> AABB<Space> {
		let reach = Vector::new(self.radius, self.radius, self.radius);
		let segment = AABB::new(self.start, self.end);
		AABB::new(segment.min() - reach, segment.max() + reach)
	}
}

#[cfg(test)]
mod tests {
	use super::Capsule;
	use crate::{Point, WorldSpace};

	#[test]
	fn bounds_reach_a_radius_past_both_caps_in_either_point_order() {
		let capsule: Capsule<WorldSpace> = Capsule::new(Point::new(1.0, 3.0, -2.0), Point::new(-1.0, 0.0, -2.0), 0.5);

		assert_eq!(capsule.bounds().min(), Point::new(-1.5, -0.5, -2.5));
		assert_eq!(capsule.bounds().max(), Point::new(1.5, 3.5, -1.5));
	}
}
