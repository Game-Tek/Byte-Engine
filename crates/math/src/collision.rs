use maths_rs::dot;

use crate::{cube::Cube, magnitude_squared, normalize, plane::Plane, ray::Ray, sphere::Sphere, Vector3};

/// Calculates the intersection point of a ray and an axis-aligned bounding box (AABB).
pub fn ray_aabb_intersection(start: Vector3, direction: Vector3, min: Vector3, max: Vector3) -> Option<f32> {
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
pub fn sphere_in_frustum(sphere: &Sphere, frustum_planes: &[Plane; 6]) -> bool {
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

#[derive(Debug, Clone, PartialEq)]
pub struct Intersection {
	pub normal: Vector3,
	pub depth: f32,
	/// Contact on A in world space coordinates.
	pub point_on_a: Vector3,
	/// Contact on B in world space coordinates.
	pub point_on_b: Vector3,
}

impl Intersection {
	/// Swap the intersection points.
	pub fn swap(self) -> Intersection {
		Intersection {
			normal: self.normal,
			depth: self.depth,
			point_on_a: self.point_on_b,
			point_on_b: self.point_on_a,
		}
	}
}

fn sphere_sphere_intersection_at_time(
	sphere_a: &Sphere,
	sphere_b: &Sphere,
	a_velocity: Vector3,
	b_velocity: Vector3,
	toi: f32,
) -> DynamicIntersection {
	let new_pos_a = sphere_a.center + a_velocity * toi;
	let new_pos_b = sphere_b.center + b_velocity * toi;
	let ab = new_pos_b - new_pos_a;
	let distance_squared = magnitude_squared(ab);

	// Coincident centers need a stable fallback normal so penetration correction does
	// not produce NaNs while the solver separates the pair.
	let (normal, distance) = if distance_squared > 1e-12 {
		let distance = distance_squared.sqrt();
		(ab / distance, distance)
	} else {
		let relative_velocity = a_velocity - b_velocity;
		if magnitude_squared(relative_velocity) > 1e-12 {
			(normalize(relative_velocity), 0.0)
		} else {
			(Vector3::new(1.0, 0.0, 0.0), 0.0)
		}
	};

	let depth = (sphere_a.radius + sphere_b.radius - distance).max(0.0);

	DynamicIntersection {
		toi,
		normal,
		depth,
		point_on_a: new_pos_a + normal * sphere_a.radius,
		point_on_b: new_pos_b - normal * sphere_b.radius,
	}
}

#[derive(Debug, Clone, PartialEq)]
pub struct DynamicIntersection {
	pub toi: f32,
	pub normal: Vector3,
	pub depth: f32,
	/// Contact on A in world space coordinates.
	pub point_on_a: Vector3,
	/// Contact on B in world space coordinates.
	pub point_on_b: Vector3,
}

impl From<DynamicIntersection> for Intersection {
	fn from(intersection: DynamicIntersection) -> Self {
		Intersection {
			normal: intersection.normal,
			depth: intersection.depth,
			point_on_a: intersection.point_on_a,
			point_on_b: intersection.point_on_b,
		}
	}
}

impl PartialOrd for DynamicIntersection {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		self.toi.partial_cmp(&other.toi)
	}
}

pub fn sphere_vs_sphere(sphere_a: &Sphere, sphere_b: &Sphere) -> Option<Intersection> {
	let ab = sphere_b.center - sphere_a.center;
	let m2 = magnitude_squared(ab);

	if m2 < (sphere_a.radius + sphere_b.radius).powf(2f32) {
		let ab_mag = m2.sqrt();
		let normal = ab / ab_mag;

		let depth = sphere_a.radius + sphere_b.radius - ab_mag;

		let point_on_a = sphere_a.center + normal * sphere_a.radius;
		let point_on_b = sphere_b.center - normal * sphere_b.radius;

		Some(Intersection {
			normal,
			depth,
			point_on_a,
			point_on_b,
		})
	} else {
		None
	}
}

pub fn cube_vs_cube(a: &Cube, b: &Cube) -> Option<Intersection> {
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
	} else if overlap.z < min_depth {
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
		_ => unreachable!(),
	};

	let a_min = a.center - sa;
	let a_max = a.center + sa;
	let b_min = b.center - sb;
	let b_max = b.center + sb;

	let overlap_min = Vector3::new(a_min.x.max(b_min.x), a_min.y.max(b_min.y), a_min.z.max(b_min.z));
	let overlap_max = Vector3::new(a_max.x.min(b_max.x), a_max.y.min(b_max.y), a_max.z.min(b_max.z));

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

	Some(Intersection {
		normal: normalize(ab),
		depth,
		point_on_a: contact_a,
		point_on_b: contact_b,
	})
}

pub fn sphere_vs_cube(sphere_a: &Sphere, cube_b: &Cube) -> Option<Intersection> {
	let delta = sphere_a.center - cube_b.center;

	let clamped_delta = Vector3::new(
		delta.x.clamp(-cube_b.half_size.x, cube_b.half_size.x),
		delta.y.clamp(-cube_b.half_size.y, cube_b.half_size.y),
		delta.z.clamp(-cube_b.half_size.z, cube_b.half_size.z),
	);

	let closest_point_on_cube = cube_b.center + clamped_delta;

	let to_center = sphere_a.center - closest_point_on_cube;
	let distance_squared = dot(to_center, to_center);

	if distance_squared > sphere_a.radius * sphere_a.radius {
		return None;
	}

	let (normal, depth, point_on_b) = if distance_squared > 1e-12 {
		let distance = distance_squared.sqrt();
		(normalize(to_center), sphere_a.radius - distance, closest_point_on_cube)
	} else {
		let distances_to_faces = cube_b.half_size - Vector3::new(delta.x.abs(), delta.y.abs(), delta.z.abs());

		// When the sphere center is inside the AABB, the closest clamped point is the
		// center itself. Use the nearest face so correction pushes the sphere outside.
		let axis = if distances_to_faces.y < distances_to_faces.x && distances_to_faces.y <= distances_to_faces.z {
			1
		} else if distances_to_faces.z < distances_to_faces.x && distances_to_faces.z < distances_to_faces.y {
			2
		} else {
			0
		};

		let sign = match axis {
			0 => delta.x.signum(),
			1 => delta.y.signum(),
			2 => delta.z.signum(),
			_ => unreachable!(),
		};
		let sign = if sign == 0.0 { 1.0 } else { sign };

		let normal = match axis {
			0 => Vector3::new(sign, 0.0, 0.0),
			1 => Vector3::new(0.0, sign, 0.0),
			2 => Vector3::new(0.0, 0.0, sign),
			_ => unreachable!(),
		};

		let point_on_b = match axis {
			0 => Vector3::new(
				cube_b.center.x + sign * cube_b.half_size.x,
				sphere_a.center.y,
				sphere_a.center.z,
			),
			1 => Vector3::new(
				sphere_a.center.x,
				cube_b.center.y + sign * cube_b.half_size.y,
				sphere_a.center.z,
			),
			2 => Vector3::new(
				sphere_a.center.x,
				sphere_a.center.y,
				cube_b.center.z + sign * cube_b.half_size.z,
			),
			_ => unreachable!(),
		};

		let depth = sphere_a.radius
			+ match axis {
				0 => distances_to_faces.x,
				1 => distances_to_faces.y,
				2 => distances_to_faces.z,
				_ => unreachable!(),
			};

		(normal, depth, point_on_b)
	};

	let point_on_a = sphere_a.center - normal * sphere_a.radius;

	Some(Intersection {
		normal,
		depth,
		point_on_a,
		point_on_b,
	})
}

/// Returns the intersection times of a ray and a sphere, if one exists.
pub fn ray_vs_sphere(ray_a: &Ray, sphere_b: &Sphere) -> Option<(f32, f32)> {
	let m = sphere_b.center - ray_a.origin;
	let a = dot(ray_a.direction, ray_a.direction);
	let b = dot(m, ray_a.direction);
	let c = dot(m, m) - sphere_b.radius * sphere_b.radius;

	let delta = b * b - a * c;

	if delta < 0.0 {
		return None;
	}

	let inv_a = 1.0 / a;

	let delta_root = delta.sqrt();
	let t1 = inv_a * (b - delta_root);
	let t2 = inv_a * (b + delta_root);

	Some((t1, t2))
}

pub fn sphere_vs_sphere_dynamic(
	sphere_a: &Sphere,
	sphere_b: &Sphere,
	a_velocity: Vector3,
	b_velocity: Vector3,
	dt: f32,
) -> Option<DynamicIntersection> {
	let relative_velocity = a_velocity - b_velocity;
	let ray_dir = relative_velocity * dt;
	let expanded_radius = sphere_a.radius + sphere_b.radius;

	// With no relative motion, this is just a static overlap at the start of the step.
	if magnitude_squared(ray_dir) < 0.00001 {
		let ab = sphere_b.center - sphere_a.center;
		let radius = expanded_radius + 0.00001;

		return (magnitude_squared(ab) <= radius * radius)
			.then(|| sphere_sphere_intersection_at_time(sphere_a, sphere_b, a_velocity, b_velocity, 0.0));
	}

	let Some((t0, t1)) = ray_vs_sphere(
		&Ray::new(sphere_a.center, ray_dir),
		&Sphere::new(sphere_b.center, expanded_radius),
	) else {
		return None;
	};

	if t1 < 0.0 {
		return None;
	}

	let toi_fraction = 0f32.max(t0);

	if toi_fraction > 1.0 {
		return None;
	}

	Some(sphere_sphere_intersection_at_time(
		sphere_a,
		sphere_b,
		a_velocity,
		b_velocity,
		toi_fraction * dt,
	))
}

#[cfg(test)]
mod tests {
	use std::assert_matches;

	use maths_rs::num::Base;

	use super::*;
	use crate::{normalize, Vector3};

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

		assert_matches!(sphere_vs_sphere(&sphere_a, &sphere_b), Some(_));
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

		assert_matches!(cube_vs_cube(&cube_a, &cube_b), Some(_));
	}

	#[test]
	fn test_sphere_vs_cube() {
		let sphere = Sphere {
			center: Vector3::new(0.0, 2.0, 0.0),
			radius: 1.0,
		};

		let cube = Cube {
			center: Vector3::new(0.0, 0.0, 0.0),
			half_size: Vector3::new(1.0, 1.0, 1.0),
		};

		assert_matches!(sphere_vs_cube(&sphere, &cube), Some(_));

		let sphere = Sphere {
			center: Vector3::new(0.0, 3.0, 0.0),
			radius: 1.0,
		};

		let cube = Cube {
			center: Vector3::new(0.0, 0.0, 0.0),
			half_size: Vector3::new(1.0, 1.0, 1.0),
		};

		assert_matches!(sphere_vs_cube(&sphere, &cube), None);
	}

	#[test]
	fn sphere_vs_cube_uses_closest_cube_surface_point() {
		let sphere = Sphere::new(Vector3::new(0.0, 0.9, 0.0), 0.5);
		let cube = Cube::new(Vector3::zero(), Vector3::new(0.5, 0.5, 0.5));

		let intersection = sphere_vs_cube(&sphere, &cube).unwrap();

		assert_eq!(intersection.point_on_b, Vector3::new(0.0, 0.5, 0.0));
		assert!((intersection.depth - 0.1).abs() < 1e-6);
	}

	#[test]
	fn sphere_vs_cube_pushes_out_from_nearest_face_when_center_is_inside() {
		let sphere = Sphere::new(Vector3::new(0.0, 0.49, 0.0), 0.5);
		let cube = Cube::new(Vector3::zero(), Vector3::new(0.5, 0.5, 0.5));

		let intersection = sphere_vs_cube(&sphere, &cube).unwrap();

		assert_eq!(intersection.normal, Vector3::new(0.0, 1.0, 0.0));
		assert_eq!(intersection.point_on_b, Vector3::new(0.0, 0.5, 0.0));
		assert!((intersection.depth - 0.51).abs() < 1e-6);
	}

	#[test]
	fn sphere_vs_sphere_dynamic_detects_approaching_spheres_before_overlap() {
		let sphere_a = Sphere::new(Vector3::new(0.0, 0.0, 0.0), 1.0);
		let sphere_b = Sphere::new(Vector3::new(4.0, 0.0, 0.0), 1.0);

		let intersection = sphere_vs_sphere_dynamic(
			&sphere_a,
			&sphere_b,
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(-1.0, 0.0, 0.0),
			2.0,
		)
		.unwrap();

		assert!((intersection.toi - 1.0).abs() < 1e-6);
		assert!(intersection.depth <= 1e-6);
		assert_eq!(intersection.normal, Vector3::new(1.0, 0.0, 0.0));
	}

	#[test]
	fn sphere_vs_sphere_dynamic_reports_positive_depth_for_initial_overlap() {
		let sphere_a = Sphere::new(Vector3::new(0.0, 0.0, 0.0), 1.0);
		let sphere_b = Sphere::new(Vector3::new(1.5, 0.0, 0.0), 1.0);

		let intersection = sphere_vs_sphere_dynamic(&sphere_a, &sphere_b, Vector3::zero(), Vector3::zero(), 1.0).unwrap();

		assert_eq!(intersection.toi, 0.0);
		assert!((intersection.depth - 0.5).abs() < 1e-6);
		assert_eq!(intersection.normal, Vector3::new(1.0, 0.0, 0.0));
	}

	#[test]
	fn test_ray_vs_sphere() {
		let ray = Ray::new(Vector3::zero(), Vector3::new(0.0, 0.0, 1.0));
		let sphere = Sphere::new(Vector3::new(0.0, 0.0, 10.0), 1.0);

		let result = ray_vs_sphere(&ray, &sphere);

		assert_matches!(result, Some(_));

		let ray = Ray::new(Vector3::zero(), Vector3::new(0.0, 0.0, 1.0));
		let sphere = Sphere::new(Vector3::new(0.0, 4.0, 10.0), 1.0);

		let result = ray_vs_sphere(&ray, &sphere);

		assert_matches!(result, None);
	}
}
