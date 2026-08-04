use math::Orientation;

/// The `Orientable` trait provides an orientation to cameras, lights, renderables, and physics systems.
///
/// Types backed by [`crate::gameplay::Transform`] should implement
/// [`crate::space::Transformable`] and use its blanket implementation.
pub trait Orientable {
	/// Returns the object's orientation.
	fn orientation(&self) -> Orientation;

	/// Sets the object's orientation.
	fn set_orientation(&mut self, orientation: Orientation);
}
