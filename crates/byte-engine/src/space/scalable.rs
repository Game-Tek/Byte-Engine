use math::Scale;

/// The `Scalable` trait provides scale values without requiring a complete gameplay transform.
///
/// Types backed by [`crate::gameplay::Transform`] should implement
/// [`crate::space::Transformable`] and use its blanket implementation.
pub trait Scalable {
	/// Returns the object's scale.
	fn scale(&self) -> Scale;

	/// Sets the object's scale.
	fn set_scale(&mut self, scale: Scale);
}
