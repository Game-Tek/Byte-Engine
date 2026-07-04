use math::{magnitude, magnitude_squared, mat::MatNew3 as _, Base, Matrix3, Vector3};

use crate::{
	core::Entity,
	physics::collider::Collider,
	space::{Positionable, Transformable},
};

/// The [`Body`] trait exposes the simulation properties required by a physics
/// world.
///
/// Implement it on transformable gameplay entities and submit their
/// [`crate::core::EntityHandle`] through the default world's body factory.
pub trait Body: Collider + Transformable {
	fn body_type(&self) -> BodyTypes;

	fn velocity(&self) -> Vector3;

	/// Returns the mass of the body in kilograms.
	/// Default implementation returns 1 kilogram.
	fn mass(&self) -> f32 {
		1f32
	}

	/// Returns the center of mass of the body in body space.
	/// Default implementation returns the origin.
	fn center_of_mass(&self) -> Vector3 {
		Vector3::new(0.0, 0.0, 0.0)
	}
}

/// The type of body that an entity is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyTypes {
	/// Static bodies are not affected by forces or collisions.
	Static,
	/// Kinematic bodies are not affected by forces, but are affected by collisions.
	Kinematic,
	/// Dynamic bodies are affected by forces and collisions.
	Dynamic,
}
