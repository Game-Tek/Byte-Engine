use maths_rs::Vec3f;

/// The `Scalable` trait provides non-spatial scale values without requiring a complete gameplay transform.
///
/// Types backed by [`crate::gameplay::Transform`] should implement
/// [`crate::space::Transformable`] and use its blanket implementation.
pub trait Scalable {
	/// Returns the object's non-uniform scale factors.
	fn scale(&self) -> Vec3f;

	/// Sets the object's non-uniform scale factors.
	fn set_scale(&mut self, scale: Vec3f);
}
