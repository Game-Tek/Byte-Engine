use math::{Point, Vector};

use crate::{
	physics::{collider::Collider, LocalSpace},
	space::Transformable,
};

/// The `Body` trait provides the simulation properties required by a physics world.
///
/// Implement it on transformable gameplay entities and submit their
/// [`crate::core::EntityHandle`] through the default world's body factory.
pub trait Body: Collider + Transformable {
	/// Returns how this body responds to simulation.
	fn body_type(&self) -> BodyTypes;

	/// Returns the body's world-space linear velocity.
	fn velocity(&self) -> Vector;

	/// Returns the mass of the body in kilograms.
	fn mass(&self) -> f32 {
		1.0
	}

	/// Returns the center of mass relative to the collider origin.
	fn center_of_mass(&self) -> Point<LocalSpace> {
		Point::origin()
	}
}

/// The `BodyTypes` enum selects how a physics body responds to forces and collisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyTypes {
	/// Ignores forces and collision resolution.
	Static,
	/// Ignores forces but participates in collision resolution.
	Kinematic,
	/// Responds to forces and collision resolution.
	Dynamic,
}
