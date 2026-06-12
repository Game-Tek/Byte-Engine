use math::Vector3;

use crate::{
	core::Entity,
	gameplay::transform::Transform,
	space::{Orientable, Positionable, Scalable},
};

/// The [`Transformable`] trait connects a type's complete gameplay transform to
/// spatial consumers.
///
/// Implement this trait on renderable or physical entities that store a
/// [`Transform`]. Position, orientation, and scale traits are supplied
/// automatically.
pub trait Transformable: Positionable + Orientable + Scalable {
	fn transform(&self) -> &Transform;
	fn transform_mut(&mut self) -> &mut Transform;
}

// Automatically implement [`Positionable`] for any type that implements [`Transformable`].
impl<T: Transformable> Positionable for T {
	fn position(&self) -> Vector3 {
		self.transform().get_position()
	}

	fn set_position(&mut self, position: Vector3) {
		self.transform_mut().set_position(position);
	}
}

impl<T: Transformable> Orientable for T {
	fn orientation(&self) -> math::Quaternion {
		self.transform().get_orientation()
	}

	fn set_orientation(&mut self, orientation: math::Quaternion) {
		self.transform_mut().set_orientation(orientation);
	}
}

impl<T: Transformable> Scalable for T {
	fn scale(&self) -> Vector3 {
		self.transform().scale()
	}

	fn set_scale(&mut self, scale: Vector3) {
		self.transform_mut().set_scale(scale);
	}
}
