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