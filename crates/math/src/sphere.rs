use crate::{Point, WorldSpace};

/// The `Sphere` struct represents a spherical volume in one coordinate space for containment and collision queries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sphere<Space = WorldSpace> {
	center: Point<Space>,
	radius: f32,
}

impl<Space> Sphere<Space> {
	/// Creates a sphere with `center` and `radius`.
	pub fn new(center: Point<Space>, radius: f32) -> Self {
		Self { center, radius }
	}

	/// Returns the center point.
	pub fn center(&self) -> Point<Space> {
		self.center
	}

	/// Returns the radius.
	pub fn radius(&self) -> f32 {
		self.radius
	}

	/// Returns whether `point` is inside this sphere or on its surface.
	pub fn contains_point(&self, point: Point<Space>) -> bool {
		(self.center - point).length_squared() <= self.radius * self.radius
	}

	/// Returns whether this sphere touches or overlaps `other`.
	pub fn intersects(&self, other: &Self) -> bool {
		let radius_sum = self.radius + other.radius;
		(self.center - other.center).length_squared() <= radius_sum * radius_sum
	}
}

#[cfg(test)]
mod tests {
	use super::Sphere;
	use crate::{Point, WorldSpace};

	#[test]
	fn containment_includes_surface_and_is_translation_invariant() {
		let sphere: Sphere<WorldSpace> = Sphere::new(Point::new(10.0, -4.0, 2.0), 3.0);
		assert!(sphere.contains_point(Point::new(10.0, -4.0, 2.0)));
		assert!(sphere.contains_point(Point::new(13.0, -4.0, 2.0)));
		assert!(!sphere.contains_point(Point::new(13.001, -4.0, 2.0)));
	}

	#[test]
	fn intersection_is_symmetric_and_includes_tangency() {
		let sphere: Sphere<WorldSpace> = Sphere::new(Point::origin(), 2.0);
		let tangent = Sphere::new(Point::new(3.0, 0.0, 0.0), 1.0);
		let separated = Sphere::new(Point::new(3.001, 0.0, 0.0), 1.0);

		assert!(sphere.intersects(&tangent));
		assert!(!sphere.intersects(&separated));
		assert_eq!(sphere.intersects(&tangent), tangent.intersects(&sphere));
	}
}
