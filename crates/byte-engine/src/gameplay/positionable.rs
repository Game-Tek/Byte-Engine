use core::Entity;

use crate::Vector3;

/// A trait for objects that have a position in 3D space.
pub trait Positionable: Entity {
	/// Get the position of the object.
	fn get_position(&self) -> Vector3;

	/// Set the position of the object.
	fn set_position(&mut self, position: Vector3);
}