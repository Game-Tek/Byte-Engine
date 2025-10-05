use math::Vector3;

use crate::core::Entity;

use super::{Positionable, Transform};

// [`Transformable`] represents an object that can be transformed in the game world.
pub trait Transformable: Positionable + Entity {
	fn get_transform(&self) -> &Transform;
	fn get_transform_mut(&mut self) -> &mut Transform;
}

// Automatically implement [`Positionable`] for any type that implements [`Transformable`].
impl <T: Transformable> Positionable for T {
	fn get_position(&self) -> Vector3 {
		self.get_transform().get_position()
	}

	fn set_position(&mut self, position: Vector3) {
		self.get_transform_mut().set_position(position);
	}
}
