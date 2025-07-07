use math::Vector3;

use crate::{core::Entity, gameplay::Positionable, physics::{collider::Collider, CollisionEvent}};

pub trait Body: Collider + Positionable + Entity {
	fn on_collision(&mut self) -> Option<&mut CollisionEvent>;

	fn get_body_type(&self) -> BodyTypes;

	fn get_velocity(&self) -> Vector3;

	fn get_mass(&self) -> f32;
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
