use crate::Vector3;

pub struct Ray {
	pub(crate) origin: Vector3,
	pub(crate) direction: Vector3,
}

impl Ray {
	pub fn new(origin: Vector3, direction: Vector3) -> Self {
		Self { origin, direction }
	}
}
