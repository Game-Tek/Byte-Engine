use maths_rs::Vec3f;

use crate::{plane::Plane, sphere::Sphere};

type Vector3 = Vec3f;

/// Calculates the intersection point of a ray and an axis-aligned bounding box (AABB).
pub fn ray_aabb_intersection(start: Vector3, direction: Vector3, min: Vector3, max: Vector3,) -> Option<f32> {
	let r = 1.0 / direction;

	let mis = min - start;
	let mas = max - start;

	let t1 = mis.x * r.x;
	let t2 = mas.x * r.x;
	let t3 = mis.y * r.y;
	let t4 = mas.y * r.y;
	let t5 = mis.z * r.z;
	let t6 = mas.z * r.z;

	let tmin = t1.min(t2).max(t3.min(t4)).max(t5.min(t6));
	let tmax = t1.max(t2).min(t3.max(t4)).min(t5.max(t6));

	if tmax >= 0.0 && tmin <= tmax {
		Some(tmin)
	} else {
		None
	}
}

/// Checks if a sphere is inside or touching a frustum defined by a set of planes.
///
/// The frustum is defined by an array of 6 planes. It is assumed that the
/// normals of these planes point inwards (towards the interior of the frustum).
///
/// # Arguments
/// * `sphere_center` - The world-space center of the sphere.
/// * `sphere_radius` - The radius of the sphere. Must be non-negative.
/// * `frustum_planes` - An array of 6 planes defining the frustum.
///
/// # Returns
/// `true` if the sphere is (at least partially) inside or intersecting the frustum,
/// `false` if the sphere is completely outside any of the frustum planes.
pub fn sphere_in_frustum(
	sphere: &Sphere,
	frustum_planes: &[Plane; 6],
) -> bool {
	// For a sphere to be visible, it must be on the "inside" or "positive" side
	// of all frustum planes (or intersecting them).
	// The "inside" is the half-space in the direction of the plane's normal.

	for plane in frustum_planes {
		if !plane.is_sphere_in_half_space(sphere) {
			// Sphere is entirely outside this plane, so it's not visible.
			return false;
		}
	}

	// Sphere is not completely outside any of the frustum planes, so it is considered visible.
	true
}

#[cfg(test)]
mod tests {
	use maths_rs::normalize;

	use super::*;

	#[test]
	fn test_ray_aabb_intersection() {
		{
			let start = Vector3::new(0.0, 2.0, 0.0);
			let direction = Vector3::new(0.0, -1.0, 0.0);
			let min = Vector3::new(-0.5, -0.5, -0.5);
			let max = Vector3::new(0.5, 0.5, 0.5);

			assert_eq!(ray_aabb_intersection(start, direction, min, max), Some(1.5));
		}

		{
			let start = Vector3::new(0.0, 0.0, -2.0);
			let direction = Vector3::new(0.0, 0.0, 1.0);
			let min = Vector3::new(-0.5, -0.5, -0.5);
			let max = Vector3::new(0.5, 0.5, 0.5);

			assert_eq!(ray_aabb_intersection(start, direction, min, max), Some(1.5));
		}

		{
			let start = Vector3::new(0.0, 1.0, -1.0);
			let direction = normalize(Vector3::new(0.0, -1.0, 1.0));
			let min = Vector3::new(-0.5, -0.5, -0.5);
			let max = Vector3::new(0.5, 0.5, 0.5);

			assert_eq!(ray_aabb_intersection(start, direction, min, max), Some(0.70710677));
		}
	}

	#[test]
	fn test_sphere_in_frustum() {
		let sphere = Sphere {
			center: Vector3::new(0.0, 0.0, 0.0),
			radius: 1.0,
		};

		let frustum_planes = [
			Plane::new(normalize(Vector3::new(1.0, 0.0, 0.0)), 1.0), // Left plane x >= -1 (normal points +X)
			Plane::new(normalize(Vector3::new(-1.0, 0.0, 0.0)), 1.0), // Right plane x <= 1 (normal points -X)
			Plane::new(normalize(Vector3::new(0.0, 1.0, 0.0)), 1.0), // Bottom plane y >= -1 (normal points +Y)
			Plane::new(normalize(Vector3::new(0.0, -1.0, 0.0)), 1.0), // Top plane y <= 1 (normal points -Y)
			Plane::new(normalize(Vector3::new(0.0, 0.0, 1.0)), 1.0), // Near plane z >= -1 (normal points +Z)
			Plane::new(normalize(Vector3::new(0.0, 0.0, -1.0)), 1.0), // Far plane z <= 1 (normal points -Z)
		];

		assert!(sphere_in_frustum(&sphere, &frustum_planes));

		let outside_sphere = Sphere {
			center: Vector3::new(5.0, 5.0, 5.0),
			radius: 1.5,
		};

		assert!(!sphere_in_frustum(&outside_sphere, &frustum_planes));

		let edge_sphere = Sphere {
			center: Vector3::new(1.0, 0.0, 0.0),
			radius: 1.0,
		};

		assert!(sphere_in_frustum(&edge_sphere, &frustum_planes));

		let edge_sphere_outside = Sphere {
			center: Vector3::new(2.0, 0.0, 0.0),
			radius: 1.0,
		};

		assert!(sphere_in_frustum(&edge_sphere_outside, &frustum_planes));
	}
}
