use maths_rs::Vec3f;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cube {
	pub center: Vec3f,
	pub half_size: Vec3f,
}

impl Cube {
	pub fn new(center: Vec3f, half_size: Vec3f) -> Self {
		Self { center, half_size }
	}
}
