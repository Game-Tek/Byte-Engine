use crate::{Point, UnitVector, WorldSpace};

/// The `Ray` struct represents a world-space query line with a checked direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray<Space = WorldSpace> {
	origin: Point<Space>,
	direction: UnitVector<Space>,
}

impl<Space> Ray<Space> {
	/// Creates a ray from an origin and a checked direction.
	///
	/// Use [`Vector::normalize`] to turn an unnormalized displacement into `direction`.
	pub fn new(origin: Point<Space>, direction: UnitVector<Space>) -> Self {
		Self { origin, direction }
	}

	/// Returns the origin of this ray.
	pub fn origin(&self) -> Point<Space> {
		self.origin
	}

	/// Returns the unit direction of this ray.
	pub fn direction(&self) -> UnitVector<Space> {
		self.direction
	}

	/// Returns the point reached after travelling `distance` along this ray.
	pub fn point_at(&self, distance: f32) -> Point<Space> {
		self.origin + self.direction * distance
	}
}

#[cfg(test)]
mod tests {
	use super::Ray;
	use crate::{Point, UnitVector, WorldSpace};

	#[test]
	fn point_at_uses_the_unit_direction() {
		let ray: Ray<WorldSpace> = Ray::new(Point::origin(), UnitVector::z_axis());
		assert_eq!(ray.point_at(2.5), Point::new(0.0, 0.0, 2.5));
	}
}
