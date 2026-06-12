use math::Vector3;

/// The [`Scalable`] trait exposes non-uniform scale without requiring consumers to
/// depend on the complete gameplay transform.
///
/// Types backed by [`crate::gameplay::transform::Transform`] should implement
/// [`crate::space::Transformable`] and use its blanket implementation.
pub trait Scalable {
	fn scale(&self) -> Vector3;

	fn set_scale(&mut self, scale: Vector3);
}
