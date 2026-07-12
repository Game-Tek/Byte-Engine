use maths_rs::{dot, Vec3f};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sphere {
	pub center: Vec3f,
	pub radius: f32,
}

impl Sphere {
	pub fn new(center: Vec3f, radius: f32) -> Self {
		Self { center, radius }
	}

	pub fn contains_point(&self, point: Vec3f) -> bool {
		let distance_squared = dot(self.center - point, self.center - point);
		distance_squared <= self.radius * self.radius
	}

	pub fn intersects(&self, other: &Sphere) -> bool {
		let distance_squared = dot(self.center - other.center, self.center - other.center);
		let radius_sum = self.radius + other.radius;
		distance_squared <= radius_sum * radius_sum
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::Vec3f;

	use super::Sphere;

	#[test]
	fn containment_includes_surface_and_is_translation_invariant() {
		let sphere = Sphere::new(Vec3f::new(10.0, -4.0, 2.0), 3.0);
		assert!(sphere.contains_point(Vec3f::new(10.0, -4.0, 2.0)));
		assert!(sphere.contains_point(Vec3f::new(13.0, -4.0, 2.0)));
		assert!(!sphere.contains_point(Vec3f::new(13.001, -4.0, 2.0)));

		let offset = Vec3f::new(-7.0, 5.0, 11.0);
		let translated = Sphere::new(sphere.center + offset, sphere.radius);
		for point in [
			Vec3f::new(10.0, -4.0, 2.0),
			Vec3f::new(13.0, -4.0, 2.0),
			Vec3f::new(13.001, -4.0, 2.0),
		] {
			assert_eq!(sphere.contains_point(point), translated.contains_point(point + offset));
		}
	}

	#[test]
	fn intersection_is_symmetric_and_includes_tangency() {
		let a = Sphere::new(Vec3f::new(0.0, 0.0, 0.0), 2.0);
		let overlapping = Sphere::new(Vec3f::new(2.5, 0.0, 0.0), 1.0);
		let tangent = Sphere::new(Vec3f::new(3.0, 0.0, 0.0), 1.0);
		let separated = Sphere::new(Vec3f::new(3.001, 0.0, 0.0), 1.0);

		for other in [overlapping, tangent, separated] {
			assert_eq!(a.intersects(&other), other.intersects(&a));
		}
		assert!(a.intersects(&overlapping));
		assert!(a.intersects(&tangent));
		assert!(!a.intersects(&separated));
	}

	#[test]
	fn zero_radius_contains_only_its_center() {
		let point = Vec3f::new(1.0, 2.0, 3.0);
		let sphere = Sphere::new(point, 0.0);
		assert!(sphere.contains_point(point));
		assert!(!sphere.contains_point(point + Vec3f::new(f32::EPSILON, 0.0, 0.0)));
	}
}
