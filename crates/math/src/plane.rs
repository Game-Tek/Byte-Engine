use maths_rs::{cross, dot, length, normalize, Vec3f};

use crate::sphere::Sphere;

/// Represents a plane in 3D space, defined by the equation N.x*x + N.y*y + N.z*z + D = 0.
/// `normal` is (N.x, N.y, N.z) and `distance` is D.
/// It is assumed that `normal` is a unit vector for distance calculations to be metric.
#[derive(Clone, Copy, Debug)]
pub struct Plane {
	pub normal: Vec3f,
	pub distance: f32,
}

impl Plane {
	/// Creates a new plane from a normal vector and a distance D.
	pub fn new(normal: Vec3f, distance: f32) -> Self {
		let length = length(normal);
		let normal = normal / length; // Normalize the normal vector
		let distance = distance / length; // Adjust distance accordingly
		Self { normal, distance }
	}

	/// Creates a new plane from three points in 3D space.
	/// The points should not be collinear.
	/// The resulting plane's normal will be a unit vector, and the distance will be calculated
	/// such that the plane equation is satisfied for the first point.
	pub fn from_points(p1: Vec3f, p2: Vec3f, p3: Vec3f) -> Self {
		let v1 = p2 - p1;
		let v2 = p3 - p1;
		let normal = normalize(cross(v1, v2));
		let distance = -dot(normal, p1);
		Self { normal, distance }
	}

	/// Calculates the signed distance from a point to the plane.
	/// If the plane's normal is a unit vector, this is the true metric distance.
	/// A positive value means the point is on the side of the plane the normal points to.
	/// A negative value means the point is on the opposite side.
	/// A zero value means the point is on the plane.
	pub fn signed_distance_to_point(&self, point: Vec3f) -> f32 {
		dot(self.normal, point) + self.distance
	}

	/// Check if the sphere is at least partially in the half-space defined by the plane.
	/// The signed distance to the sphere's center must be greater than or equal to -radius.
	pub fn is_sphere_in_half_space(&self, sphere: &Sphere) -> bool {
		let dist_to_center = self.signed_distance_to_point(sphere.center);
		dist_to_center >= -sphere.radius
	}
}

#[cfg(test)]
mod tests {
	use crate::{assert_float_eq, assert_vec3f_near};

	use super::*;

	#[test]
	fn test_plane_new_and_signed_distance() {
		let normal = normalize(Vec3f::new(1.0, 2.0, -2.0)); // Normal vector (1/3, 2/3, -2/3)
		let distance_d = -5.0;
		let plane = Plane::new(normal, distance_d);

		// Point on the plane: P such that N.P + D = 0 => N.P = -D.
		// A simple choice for P is -D * N (if N is unit vector)
		let point_on_plane = normal * (-distance_d);
		assert_float_eq!(
			plane.signed_distance_to_point(point_on_plane), 0.0, "Point on plane should have zero distance"
		);

		let point_positive_side = point_on_plane + normal * 3.0; // 3 units along normal
		assert_float_eq!(
			plane.signed_distance_to_point(point_positive_side), 3.0, "Point on positive side distance check"
		);

		let point_negative_side = point_on_plane - normal * 4.0; // 4 units against normal
		assert_float_eq!(
			plane.signed_distance_to_point(point_negative_side), -4.0, "Point on negative side distance check"
		);
	}

	#[test]
	fn test_from_points_origin_xy_plane() {
		let p1 = Vec3f::new(0.0, 0.0, 0.0);
		let p2 = Vec3f::new(2.0, 0.0, 0.0);
		let p3 = Vec3f::new(0.0, 3.0, 0.0);
		let plane = Plane::from_points(p1, p2, p3);

		// v1 = p2-p1 = (2,0,0), v2 = p3-p1 = (0,3,0)
		// cross(v1,v2) = (0,0,6). normalize -> (0,0,1)
		let expected_normal = Vec3f::new(0.0, 0.0, 1.0);
		// distance = -dot(normal, p1) = -dot((0,0,1), (0,0,0)) = 0
		let expected_distance_d = 0.0;

		assert_vec3f_near!(plane.normal, expected_normal, "Normal for XY plane");
		assert_float_eq!(plane.distance, expected_distance_d, "Distance D for XY plane");

		assert_float_eq!(plane.signed_distance_to_point(p1), 0.0, "p1 on XY plane");
		assert_float_eq!(plane.signed_distance_to_point(p2), 0.0, "p2 on XY plane");
		assert_float_eq!(plane.signed_distance_to_point(p3), 0.0, "p3 on XY plane");

		let test_point_on_plane = Vec3f::new(5.0, -5.0, 0.0);
		assert_float_eq!(
			plane.signed_distance_to_point(test_point_on_plane), 0.0,
			"Another point on XY plane"
		);
		let test_point_off_plane_pos = Vec3f::new(0.0, 0.0, 1.0);
		assert_float_eq!(
			plane.signed_distance_to_point(test_point_off_plane_pos), 1.0,
			"Point on positive side of XY plane"
		);
		let test_point_off_plane_neg = Vec3f::new(0.0, 0.0, -2.0);
		assert_float_eq!(
			plane.signed_distance_to_point(test_point_off_plane_neg), -2.0,
			"Point on negative side of XY plane"
		);
	}

	#[test]
	fn test_from_points_offset_plane_x_equals_constant() {
		// Plane x = -2
		let p1 = Vec3f::new(-2.0, 0.0, 0.0);
		let p2 = Vec3f::new(-2.0, 1.0, 0.0); // p2-p1 = (0,1,0)
		let p3 = Vec3f::new(-2.0, 0.0, 1.0); // p3-p1 = (0,0,1)
		let plane = Plane::from_points(p1, p2, p3);

		// cross((0,1,0), (0,0,1)) = (1,0,0). normalize -> (1,0,0)
		let expected_normal = Vec3f::new(1.0, 0.0, 0.0);
		// distance = -dot((1,0,0), (-2,0,0)) = -(-2) = 2.0
		let expected_distance_d = 2.0;

		assert_vec3f_near!(plane.normal, expected_normal, "Normal for x=-2 plane");
		assert_float_eq!(plane.distance, expected_distance_d, "Distance D for x=-2 plane");

		assert_float_eq!(plane.signed_distance_to_point(p1), 0.0, "p1 on x=-2 plane");
		assert_float_eq!(plane.signed_distance_to_point(p2), 0.0, "p2 on x=-2 plane");
		assert_float_eq!(plane.signed_distance_to_point(p3), 0.0, "p3 on x=-2 plane");

		let test_point_on_plane = Vec3f::new(-2.0, 5.0, 5.0);
		assert_float_eq!(
			plane.signed_distance_to_point(test_point_on_plane), 0.0, "Another point on x=-2 plane"
		);
		// N.P + D = (1,0,0).(-1,0,0) + 2 = -1 + 2 = 1
		let test_point_off_plane_pos = Vec3f::new(-1.0, 0.0, 0.0);
		assert_float_eq!(
			plane.signed_distance_to_point(test_point_off_plane_pos), 1.0, "Point on positive side of x=-2 plane (x=-1)"
		);
		// N.P + D = (1,0,0).(-3,0,0) + 2 = -3 + 2 = -1
		let test_point_off_plane_neg = Vec3f::new(-3.0, 0.0, 0.0);
		assert_float_eq!(
			plane.signed_distance_to_point(test_point_off_plane_neg), -1.0, "Point on negative side of x=-2 plane (x=-3)"
		);
	}

	#[test]
	fn test_from_points_general_case_properties() {
		let p1 = Vec3f::new(1.0, 2.0, 3.0);
		let p2 = Vec3f::new(4.0, -1.0, 5.0);
		let p3 = Vec3f::new(-2.0, 4.0, -3.0); // Ensure non-collinear
		let plane = Plane::from_points(p1, p2, p3);

		// Check if the normal is a unit vector
		assert_float_eq!(
			dot(plane.normal, plane.normal), 1.0, "Normal vector should be unit length"
		);

		// Check if all three defining points lie on the generated plane
		assert_float_eq!(
			plane.signed_distance_to_point(p1), 0.0, "p1 should be on the generated plane"
		);
		assert_float_eq!(
			plane.signed_distance_to_point(p2), 0.0, "p2 should be on the generated plane"
		);
		assert_float_eq!(
			plane.signed_distance_to_point(p3), 0.0, "p3 should be on the generated plane"
		);
	}

	#[test]
	fn test_from_points_winding_order() {
		let p1 = Vec3f::new(1.0, 0.0, 0.0);
		let p2 = Vec3f::new(0.0, 1.0, 0.0);
		let p3 = Vec3f::new(0.0, 0.0, 1.0);

		let plane1 = Plane::from_points(p1, p2, p3); // (p2-p1) x (p3-p1)
		let plane2 = Plane::from_points(p1, p3, p2); // (p3-p1) x (p2-p1)

		// Normals should be opposite
		assert_vec3f_near!(plane1.normal, plane2.normal * -1.0, "Normals should be opposite");
		// Distances D should be opposite (since D = -N.p1)
		assert_float_eq!(plane1.distance, plane2.distance * -1.0, "Distances D should be opposite");

		// Both planes must still contain all three points (distance = 0)
		for p_idx in [p1, p2, p3].iter().enumerate() {
			assert_float_eq!(
				plane1.signed_distance_to_point(*p_idx.1), 0.0,
				"Point p{} on plane1", p_idx.0 + 1
			);
			assert_float_eq!(
				plane2.signed_distance_to_point(*p_idx.1), 0.0,
				"Point p{} on plane2", p_idx.0 + 1
			);
		}

		// For any other point, signed distances should be opposite
		let test_point = Vec3f::new(5.0, 5.0, 5.0);
		let dist1 = plane1.signed_distance_to_point(test_point);
		let dist2 = plane2.signed_distance_to_point(test_point);
		assert_float_eq!(
			dist1, -dist2,
			"Signed distances to a test point should be opposite for reversed winding"
		);
	}
}