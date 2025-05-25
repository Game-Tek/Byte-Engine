use maths_rs::Vec3f;

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

#[cfg(test)]
mod tests {
	use maths_rs::normalize;

	use super::*;

	#[test]
	fn test_intersection() {
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
}
