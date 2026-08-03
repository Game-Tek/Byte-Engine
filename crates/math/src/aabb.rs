use crate::{Point, Vector, WorldSpace};

/// The `AABB` struct represents an axis-aligned volume in one coordinate space for broad-phase and contact queries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AABB<Space = WorldSpace> {
	min: Point<Space>,
	max: Point<Space>,
}

impl<Space> AABB<Space> {
	/// Creates an axis-aligned box from two opposite corners.
	///
	/// The constructor orders each coordinate, so callers can pass the corners in either order.
	pub fn new(first: Point<Space>, second: Point<Space>) -> Self {
		let min = Point::new(
			first.x().min(second.x()),
			first.y().min(second.y()),
			first.z().min(second.z()),
		);
		let max = Point::new(
			first.x().max(second.x()),
			first.y().max(second.y()),
			first.z().max(second.z()),
		);
		Self { min, max }
	}

	/// Creates an axis-aligned box from its center and non-negative half extents.
	pub fn from_center_and_half_extents(center: Point<Space>, half_extents: Vector<Space>) -> Self {
		Self::new(center - half_extents, center + half_extents)
	}

	/// Returns the smallest corner.
	pub fn min(&self) -> Point<Space> {
		self.min
	}

	/// Returns the largest corner.
	pub fn max(&self) -> Point<Space> {
		self.max
	}

	/// Returns the box center.
	pub fn center(&self) -> Point<Space> {
		self.min + (self.max - self.min) * 0.5
	}

	/// Returns the distance from the center to each face.
	pub fn half_extents(&self) -> Vector<Space> {
		(self.max - self.min) * 0.5
	}

	/// Returns whether `point` is inside this box or on its boundary.
	pub fn contains_point(&self, point: Point<Space>) -> bool {
		point.x() >= self.min.x()
			&& point.x() <= self.max.x()
			&& point.y() >= self.min.y()
			&& point.y() <= self.max.y()
			&& point.z() >= self.min.z()
			&& point.z() <= self.max.z()
	}
}

#[cfg(test)]
mod tests {
	use super::AABB;
	use crate::{Point, Vector, WorldSpace};

	#[test]
	fn constructor_orders_corners_and_preserves_extents() {
		let aabb: AABB<WorldSpace> = AABB::new(Point::new(3.0, -2.0, 4.0), Point::new(-1.0, 6.0, 2.0));

		assert_eq!(aabb.min(), Point::new(-1.0, -2.0, 2.0));
		assert_eq!(aabb.max(), Point::new(3.0, 6.0, 4.0));
		assert_eq!(aabb.half_extents(), Vector::new(2.0, 4.0, 1.0));
	}
}
