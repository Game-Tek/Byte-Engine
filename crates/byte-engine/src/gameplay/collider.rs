use math::Vector3;

use crate::{core::{Entity, EntityHandle}, gameplay::{Positionable, Transformable}, physics::{body::{Body, BodyTypes}, collider::{Collider, Shapes}}};

use crate::physics;

pub struct Sphere {
	radius: f32,
}

pub struct Cube {
	/// The half-size of the cube
	size: Vector3,
}

impl Sphere {
	pub fn new(radius: f32) -> Self {
		Self {
			radius,
		}
	}
}

impl Cube {
	pub fn new(size: Vector3) -> Self {
		Self {
			size,
		}
	}
}

impl Entity for Sphere {}

impl Positionable for Sphere {
	fn position(&self) -> Vector3 {
		todo!()
	}

	fn set_position(&mut self, position: Vector3) {
		todo!()
	}
}

impl Collider for Sphere {
	fn shape(&self) -> Shapes { Shapes::Sphere { radius: self.radius } }
}

impl Entity for Cube {
}

impl Collider for Cube {
	fn shape(&self) -> Shapes { Shapes::Cube { size: self.size } }
}

impl Positionable for Cube {
	fn position(&self) -> Vector3 {
		todo!()
	}

	fn set_position(&mut self, position: Vector3) {
		todo!()
	}
}
