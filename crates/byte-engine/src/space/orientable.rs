use math::Quaternion;

/// The [`Orientable`] trait exposes world-space rotation to cameras, lights,
/// renderables, and physics systems.
///
/// Types backed by [`crate::gameplay::transform::Transform`] should implement
/// [`crate::space::Transformable`] and use its blanket implementation.
pub trait Orientable {
	fn orientation(&self) -> Quaternion;
	fn set_orientation(&mut self, orientation: Quaternion);
}
