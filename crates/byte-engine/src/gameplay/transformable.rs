use math::Vector3;

use crate::core::Entity;

use super::{Positionable, transform::Transform};

// [`Transformable`] represents an object that can be transformed in the game world.
pub trait Transformable: Positionable + Entity {
	fn transform(&self) -> &Transform;
	fn transform_mut(&mut self) -> &mut Transform;
}

// Automatically implement [`Positionable`] for any type that implements [`Transformable`].
impl <T: Transformable> Positionable for T {
	fn position(&self) -> Vector3 {
		self.transform().get_position()
	}

	fn set_position(&mut self, position: Vector3) {
		self.transform_mut().set_position(position);
	}
}
