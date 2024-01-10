use crate::core::Entity;

pub struct Transform {
	matrix: maths_rs::Mat4f,
}

impl Transform {
	pub fn new() -> Self {
		Self {
			matrix: maths_rs::Mat4f::identity(),
		}
	}
}

impl Entity for Transform {}