use math::{Orientation, Point, Scale};

use crate::{
	gameplay::Transform,
	space::{Orientable, Positionable, Scalable},
};

/// The `Transformable` trait connects a type's complete gameplay transform to spatial consumers.
///
/// Implement this trait on renderable or physical entities that store a [`Transform`].
/// Position, orientation, and scale traits are then supplied automatically.
pub trait Transformable: Positionable + Orientable + Scalable {
	/// Returns the entity's complete transform.
	fn transform(&self) -> &Transform;

	/// Returns mutable access to the entity's complete transform.
	fn transform_mut(&mut self) -> &mut Transform;
}

impl<T: Transformable> Positionable for T {
	fn position(&self) -> Point {
		self.transform().get_position()
	}

	fn set_position(&mut self, position: Point) {
		self.transform_mut().set_position(position);
	}
}

impl<T: Transformable> Orientable for T {
	fn orientation(&self) -> Orientation {
		self.transform().get_orientation()
	}

	fn set_orientation(&mut self, orientation: Orientation) {
		self.transform_mut().set_orientation(orientation);
	}
}

impl<T: Transformable> Scalable for T {
	fn scale(&self) -> Scale {
		self.transform().scale()
	}

	fn set_scale(&mut self, scale: Scale) {
		self.transform_mut().set_scale(scale);
	}
}
