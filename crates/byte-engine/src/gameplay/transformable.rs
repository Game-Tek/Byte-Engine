use crate::core::Entity;

use super::{Positionable, Transform};

// [`Transformable`] represents an object that can be transformed in the game world.
pub trait Transformable: Positionable + Entity {
	fn get_transform(&self) -> &Transform;
	fn get_transform_mut(&mut self) -> &mut Transform;
}