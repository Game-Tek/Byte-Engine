use math::Vector3;

use crate::physics;
use crate::{
	core::{Entity, EntityHandle},
	physics::{
		body::{Body, BodyTypes},
		collider::{Collider, Shapes},
	},
	space::{Positionable, Transformable},
};

pub struct Sphere {
	radius: f32,
	position: Vector3,
}

pub struct Cube {
	/// The half-size of the cube
	size: Vector3,
	position: Vector3,
}

impl Sphere {
	pub fn new(radius: f32) -> Self {
		Self {
			radius,
			position: Vector3::new(0.0, 0.0, 0.0),
		}
	}
}

impl Cube {
	pub fn new(size: Vector3) -> Self {
		Self {
			size,
			position: Vector3::new(0.0, 0.0, 0.0),
		}
	}
}

impl Entity for Sphere {}

impl Positionable for Sphere {
	fn position(&self) -> Vector3 {
		self.position
	}

	fn set_position(&mut self, position: Vector3) {
		self.position = position;
	}
}

impl Collider for Sphere {
	fn shape(&self) -> Shapes {
		Shapes::Sphere { radius: self.radius }
	}
}

impl Entity for Cube {}

impl Collider for Cube {
	fn shape(&self) -> Shapes {
		Shapes::Cube { size: self.size }
	}
}

impl Positionable for Cube {
	fn position(&self) -> Vector3 {
		self.position
	}

	fn set_position(&mut self, position: Vector3) {
		self.position = position;
	}
}

#[cfg(test)]
mod tests {
	use math::Vector3;

	use super::{Cube, Sphere};
	use crate::{
		physics::collider::{Collider, Shapes},
		space::Positionable,
	};

	#[test]
	fn primitive_colliders_retain_position_independently_of_shape() {
		let mut sphere = Sphere::new(2.5);
		let mut cube = Cube::new(Vector3::new(1.0, 2.0, 3.0));

		assert_eq!(sphere.position(), Vector3::new(0.0, 0.0, 0.0));
		assert_eq!(cube.position(), Vector3::new(0.0, 0.0, 0.0));
		sphere.set_position(Vector3::new(4.0, 5.0, 6.0));
		cube.set_position(Vector3::new(-1.0, -2.0, -3.0));
		assert_eq!(sphere.position(), Vector3::new(4.0, 5.0, 6.0));
		assert_eq!(cube.position(), Vector3::new(-1.0, -2.0, -3.0));

		assert!(matches!(sphere.shape(), Shapes::Sphere { radius } if radius == 2.5));
		assert!(matches!(cube.shape(), Shapes::Cube { size } if size == Vector3::new(1.0, 2.0, 3.0)));
	}

	#[test]
	fn primitive_colliders_keep_default_material_response() {
		let sphere = Sphere::new(1.0);
		assert_eq!(sphere.elasticity(), 0.1);
		assert_eq!(sphere.friction(), 0.1);
	}
}
