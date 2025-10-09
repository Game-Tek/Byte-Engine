use math::Vector3;

use crate::{core::Entity, gameplay::Positionable, physics::{collider::Collider, CollisionEvent}};

/// The `Body` trait represents a physical body in the world.
pub trait Body: Collider + Positionable + Entity {
	fn on_collision(&mut self) -> Option<&mut CollisionEvent>;

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
