use math::Vector3;

use crate::core::Entity;

/// The [`Positionable`] trait exposes world-space translation to systems that do
/// not require a complete transform.
///
/// Implement [`crate::space::Transformable`] instead when the type owns a
/// [`crate::gameplay::transform::Transform`]; the blanket implementation will
/// provide this trait.
pub trait Positionable {
	/// Get the position of the object.
	fn position(&self) -> Vector3;

	/// Set the position of the object.
	fn set_position(&mut self, position: Vector3);
}
