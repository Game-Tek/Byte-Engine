use math::Vector3;

pub trait Scalable {
	fn scale(&self) -> Vector3;

	fn set_scale(&mut self, scale: Vector3);
}
