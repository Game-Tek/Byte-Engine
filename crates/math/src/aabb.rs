use crate::serialization::serialize_as_array;
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
		let (first, second) = (first.into_maths(), second.into_maths());
		Self {
			min: Point::from_maths(maths_rs::min(first, second)),
			max: Point::from_maths(maths_rs::max(first, second)),
		}
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

	/// Returns the smallest and largest corners as the arrays this box is stored as.
	fn to_array(&self) -> [[f32; 3]; 2] {
		[self.min.to_array(), self.max.to_array()]
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
	fn constructors_order_corners_and_preserve_center_and_extents() {
		let aabb: AABB<WorldSpace> = AABB::new(Point::new(3.0, -2.0, 4.0), Point::new(-1.0, 6.0, 2.0));

		assert_eq!(aabb.min(), Point::new(-1.0, -2.0, 2.0));
		assert_eq!(aabb.max(), Point::new(3.0, 6.0, 4.0));
		assert_eq!(aabb.half_extents(), Vector::new(2.0, 4.0, 1.0));
		let center = Point::new(-3.0, 4.0, 9.0);
		let half_extents = Vector::new(2.0, 5.0, 1.5);
		let centered: AABB<WorldSpace> = AABB::from_center_and_half_extents(center, half_extents);

		assert_eq!(centered.center(), center);
		assert_eq!(centered.half_extents(), half_extents);
	}

	#[test]
	fn containment_includes_faces_and_rejects_each_outside_axis() {
		let aabb: AABB<WorldSpace> = AABB::new(Point::new(-1.0, -2.0, -3.0), Point::new(4.0, 5.0, 6.0));

		assert!(aabb.contains_point(Point::new(-1.0, 5.0, 6.0)));
		assert!(aabb.contains_point(Point::new(4.0, -2.0, -3.0)));
		assert!(!aabb.contains_point(Point::new(-1.01, 0.0, 0.0)));
		assert!(!aabb.contains_point(Point::new(0.0, 5.01, 0.0)));
		assert!(!aabb.contains_point(Point::new(0.0, 0.0, 6.01)));
	}
}

serialize_as_array!(
	AABB<Space>,
	[[f32; 3]; 2],
	to_array,
	from: |[min, max]: [[f32; 3]; 2]| AABB::new(Point::from_array(min), Point::from_array(max)),
	Space
);
