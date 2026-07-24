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
	///
	/// The default implementation returns 1 kilogram.
	fn mass(&self) -> f32 {
		1f32
	}

	/// Returns the center of mass of the body in body space.
	///
	/// The default implementation returns the origin.
	fn center_of_mass(&self) -> Vector3 {
		Vector3::new(0.0, 0.0, 0.0)
	}
}

/// The `BodyTypes` enum selects how a physics body responds to forces and collisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyTypes {
	/// Ignores forces and collisions.
	Static,
	/// Ignores forces but participates in collisions.
	Kinematic,
	/// Responds to forces and collisions.
	Dynamic,
}
