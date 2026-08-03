use crate::{Point, Sphere, UnitVector, WorldSpace};

/// The `Plane` struct represents a validated half-space for distance, frustum, and collision queries.
///
/// Its [`UnitVector`] normal keeps signed-distance results in coordinate-space units. Create a plane with [`Self::new`] when you already have a checked normal, or use [`Self::from_points`] to validate a plane from geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane<Space = WorldSpace> {
	normal: UnitVector<Space>,
	distance: f32,
}

impl<Space> Plane<Space> {
	/// Creates a plane from a checked unit normal and the equation constant `distance`.
	///
	/// The plane equation is `normal · point + distance = 0`. The [`UnitVector`] parameter prevents non-unit normals from producing incorrectly scaled distances. Use [`Self::from_point_and_normal`] when you have a point on the plane.
	pub fn new(normal: UnitVector<Space>, distance: f32) -> Self {
		Self { normal, distance }
	}

	/// Creates a plane that contains `point` and uses `normal` as its positive side.
	pub fn from_point_and_normal(point: Point<Space>, normal: UnitVector<Space>) -> Self {
		let distance = -normal.dot(point - Point::origin());
		Self::new(normal, distance)
	}

	/// Creates a plane through three non-collinear points.
	///
	/// The point winding determines the normal direction. The method returns [`crate::NormalizationError`] when the cross product has no direction, which happens when the points are coincident or collinear.
	pub fn from_points(
		first: Point<Space>,
		second: Point<Space>,
		third: Point<Space>,
	) -> Result<Self, crate::NormalizationError> {
		let normal = (second - first).cross(third - first).normalize()?;
		Ok(Self::from_point_and_normal(first, normal))
	}

	/// Returns the unit normal that defines this plane's positive half-space.
	pub fn normal(&self) -> UnitVector<Space> {
		self.normal
	}

	/// Returns the equation constant in `normal · point + distance = 0`.
	pub fn distance(&self) -> f32 {
		self.distance
	}

	/// Returns the signed world-space distance from `point` to this plane.
	pub fn signed_distance_to_point(&self, point: Point<Space>) -> f32 {
		self.normal.dot(point - Point::origin()) + self.distance
	}

	/// Returns whether any part of `sphere` is in this plane's positive half-space.
	pub fn is_sphere_in_half_space(&self, sphere: &Sphere<Space>) -> bool {
		self.signed_distance_to_point(sphere.center()) >= -sphere.radius()
	}
}

#[cfg(test)]
mod tests {
	use super::Plane;
	use crate::{assert_float_eq, assert_float_eq_with_epsilon, assert_geometry_near, Point, UnitVector, Vector, WorldSpace};

	#[test]
	fn checked_normals_keep_signed_distances_in_coordinate_space_units() {
		let normal: UnitVector<WorldSpace> = Vector::new(1.0, 2.0, -2.0).normalize().unwrap();
		let plane = Plane::new(normal, -5.0);
		let point_on_plane = Point::origin() + normal * 5.0;

		assert_float_eq!(plane.signed_distance_to_point(point_on_plane), 0.0);
		assert_float_eq!(plane.signed_distance_to_point(point_on_plane + normal * 3.0), 3.0);
		assert_float_eq!(plane.signed_distance_to_point(point_on_plane - normal * 4.0), -4.0);
	}

	#[test]
	fn points_define_a_unit_normal_with_winding() {
		let plane: Plane<WorldSpace> =
			Plane::from_points(Point::origin(), Point::new(2.0, 0.0, 0.0), Point::new(0.0, 3.0, 0.0)).unwrap();
		assert_eq!(plane.normal(), UnitVector::z_axis());
		assert_eq!(plane.signed_distance_to_point(Point::new(0.0, 0.0, 0.0)), 0.0);
	}

	#[test]
	fn offset_and_general_planes_contain_every_defining_point() {
		let offset = Plane::<WorldSpace>::from_points(
			Point::new(-2.0, 0.0, 0.0),
			Point::new(-2.0, 1.0, 0.0),
			Point::new(-2.0, 0.0, 1.0),
		)
		.unwrap();
		assert_eq!(offset.normal(), UnitVector::x_axis());
		assert_eq!(offset.distance(), 2.0);
		assert_float_eq!(offset.signed_distance_to_point(Point::new(-1.0, 0.0, 0.0)), 1.0);
		assert_float_eq!(offset.signed_distance_to_point(Point::new(-3.0, 0.0, 0.0)), -1.0);

		let points: [Point<WorldSpace>; 3] = [
			Point::new(1.0, 2.0, 3.0),
			Point::new(4.0, -1.0, 5.0),
			Point::new(-2.0, 4.0, -3.0),
		];
		let general = Plane::from_points(points[0], points[1], points[2]).unwrap();
		assert_float_eq_with_epsilon!(general.normal().into_vector().length_squared(), 1.0, 0.0001);
		for point in points {
			assert_float_eq_with_epsilon!(general.signed_distance_to_point(point), 0.0, 0.0001);
		}
	}

	#[test]
	fn reversed_winding_reverses_the_half_space() {
		let first = Point::<WorldSpace>::new(1.0, 0.0, 0.0);
		let second = Point::new(0.0, 1.0, 0.0);
		let third = Point::new(0.0, 0.0, 1.0);
		let forward = Plane::from_points(first, second, third).unwrap();
		let reversed = Plane::from_points(first, third, second).unwrap();

		assert_geometry_near!(forward.normal(), -reversed.normal());
		assert_float_eq!(forward.distance(), -reversed.distance());
		for point in [first, second, third] {
			assert_float_eq!(forward.signed_distance_to_point(point), 0.0);
			assert_float_eq!(reversed.signed_distance_to_point(point), 0.0);
		}
		let test_point = Point::new(5.0, 5.0, 5.0);
		assert_float_eq!(
			forward.signed_distance_to_point(test_point),
			-reversed.signed_distance_to_point(test_point)
		);
	}

	#[test]
	fn collinear_points_are_rejected() {
		assert_eq!(
			Plane::<WorldSpace>::from_points(Point::origin(), Point::new(1.0, 0.0, 0.0), Point::new(2.0, 0.0, 0.0)),
			Err(crate::NormalizationError::ZeroLength)
		);
	}
}
