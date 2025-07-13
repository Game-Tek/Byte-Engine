use crate::{cube::Cube, magnitude_squared, plane::Plane, sphere::Sphere, Vector3, normalize};

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

pub struct Intersection {
	pub normal: Vector3,
	pub depth: f32,
	pub point_on_a: Vector3,
	pub point_on_b: Vector3,
}

pub fn sphere_vs_sphere(
	sphere_a: &Sphere,
	sphere_b: &Sphere,
) -> Option<Intersection> {
	let ab = sphere_b.center - sphere_a.center;
	let m2 = magnitude_squared(ab);

	if m2 < (sphere_a.radius + sphere_b.radius).powf(2f32) {
		let ab_mag = m2.sqrt();
		let normal = ab / ab_mag;

		let depth = sphere_a.radius + sphere_b.radius - ab_mag;

		let point_on_a = sphere_a.center + normal * sphere_a.radius;
		let point_on_b = sphere_b.center - normal * sphere_b.radius;

		Some(Intersection{ normal, depth, point_on_a, point_on_b })
	} else {
		None
	}
}

pub fn cube_vs_cube(
	a: &Cube,
	b: &Cube,
) -> Option<Intersection> {
	let sa = a.half_size;
	let sb = b.half_size;

	let ab = a.center - b.center;
	let abs_ab = Vector3::new(ab.x.abs(), ab.y.abs(), ab.z.abs());
	let overlap = sa + sb - abs_ab;

	if overlap.x <= 0.0 || overlap.y <= 0.0 || overlap.z <= 0.0 {
		return None;
	}

	let mut min_depth = overlap.x;

	let axis = if overlap.y < min_depth {
		min_depth = overlap.y;
		1
	} else  if overlap.z < min_depth {
		min_depth = overlap.z;
		2
	} else {
		0
	};

	let depth = min_depth;

	let sign = match axis {
		0 => ab.x.signum(),
		1 => ab.y.signum(),
		2 => ab.z.signum(),
		_ => unreachable!()
	};

	let normal = match axis {
		0 => Vector3::new(sign, 0.0, 0.0),
		1 => Vector3::new(0.0, sign, 0.0),
		2 => Vector3::new(0.0, 0.0, sign),
		_ => unreachable!()
	};

	let a_min = a.center - sa;
	let a_max = a.center + sa;
	let b_min = b.center - sb;
	let b_max = b.center + sb;

	let overlap_min = Vector3::new(
		a_min.x.max(b_min.x),
		a_min.y.max(b_min.y),
		a_min.z.max(b_min.z),
	);
	let overlap_max = Vector3::new(
		a_max.x.min(b_max.x),
		a_max.y.min(b_max.y),
		a_max.z.min(b_max.z),
	);

	let ox = (overlap_min.x + overlap_max.x) / 2f32;
	let oy = (overlap_min.y + overlap_max.y) / 2f32;
	let oz = (overlap_min.z + overlap_max.z) / 2f32;

	let (contact_a, contact_b) = match axis {
		0 => {
			let (ax, bx) = if sign > 0f32 { (a_max.x, b_min.x) } else { (a_min.x, b_max.x) };

			(Vector3::new(ax, oy, oz), Vector3::new(bx, oy, oz))
		}
		1 => {
			let (ay, by) = if sign > 0f32 { (a_max.y, b_min.y) } else { (a_min.y, b_max.y) };

			(Vector3::new(ox, ay, oz), Vector3::new(ox, by, oz))
		}
		2 => {
			let (az, bz) = if sign > 0f32 { (a_max.z, b_min.z) } else { (a_min.z, b_max.z) };

			(Vector3::new(ox, oy, az), Vector3::new(ox, oy, bz))
		}
		_ => unreachable!(),
	};

	Some(Intersection{ normal: normalize(ab), depth, point_on_a: contact_a, point_on_b: contact_b })
}

#[cfg(test)]
mod tests {
	use crate::{normalize, Vector3};
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

	#[test]
	fn test_sphere_vs_sphere() {
		let sphere_a = Sphere {
			center: Vector3::new(0.0, 0.0, 0.0),
			radius: 1.0,
		};

		let sphere_b = Sphere {
			center: Vector3::new(1.98, 0.0, 0.0),
			radius: 1.0,
		};

		assert!(sphere_vs_sphere(&sphere_a, &sphere_b).is_some());
	}

	#[test]
	fn test_cube_vs_cube() {
		let cube_a = Cube {
			center: Vector3::new(0.0, 0.0, 0.0),
			half_size: Vector3::new(1.0, 1.0, 1.0),
		};

		let cube_b = Cube {
			center: Vector3::new(1.0, 0.0, 0.0),
			half_size: Vector3::new(1.0, 1.0, 1.0),
		};

		assert!(cube_vs_cube(&cube_a, &cube_b).is_some());
	}
}
