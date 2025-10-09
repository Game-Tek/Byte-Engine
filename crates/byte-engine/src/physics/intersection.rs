use math::Vector3;

pub struct Intersection {
	pub(crate) normal: Vector3,
	pub(crate) depth: f32,
	pub(crate) point_on_a: Vector3,
	pub(crate) point_on_b: Vector3,
}
