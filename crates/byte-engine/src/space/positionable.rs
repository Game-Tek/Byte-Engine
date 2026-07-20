use math::Vector3;

use crate::core::Entity;

/// The [`Positionable`] trait exposes world-space translation to systems that do
/// not require a complete transform.
///
/// Implement [`crate::space::Transformable`] instead when the type owns a
/// [`crate::gameplay::transform::Transform`]; the blanket implementation will
/// provide this trait.
pub trait Positionable {
	/// Returns the object's world-space position.
	fn position(&self) -> Vector3;

	/// Sets the object's world-space position.
	fn set_position(&mut self, position: Vector3);
}
