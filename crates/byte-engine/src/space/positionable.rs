use math::Point;

/// The `Positionable` trait provides a world-space location to systems that do not need a complete transform.
///
/// Implement [`crate::space::Transformable`] instead when the type owns a
/// [`crate::gameplay::Transform`]; its blanket implementation provides this trait.
pub trait Positionable {
	/// Returns the object's world-space position.
	fn position(&self) -> Point;

	/// Sets the object's world-space position.
	fn set_position(&mut self, position: Point);
}
