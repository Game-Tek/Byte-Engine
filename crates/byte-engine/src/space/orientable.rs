use maths_rs::Quatf;

/// The `Orientable` trait provides a raw rotation boundary to cameras, lights, renderables, and physics systems.
///
/// Types backed by [`crate::gameplay::Transform`] should implement
/// [`crate::space::Transformable`] and use its blanket implementation.
pub trait Orientable {
	/// Returns the object's orientation quaternion.
	fn orientation(&self) -> Quatf;

	/// Sets the object's orientation quaternion.
	fn set_orientation(&mut self, orientation: Quatf);
}
