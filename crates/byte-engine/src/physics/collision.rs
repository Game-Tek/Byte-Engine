use math::Vector3;

#[derive(Debug)]
pub struct Collision {
	pub(crate) pair: (usize, usize),
	pub(crate) normal: Vector3,
	pub(crate) depth: f32,
	pub(crate) impulses: (Vector3, Vector3),
}
