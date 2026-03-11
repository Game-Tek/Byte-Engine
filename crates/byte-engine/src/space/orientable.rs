use math::Quaternion;

pub trait Orientable {
	fn orientation(&self) -> Quaternion;
	fn set_orientation(&mut self, orientation: Quaternion);
}
