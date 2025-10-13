use math::{mat::MatNew3 as _, Matrix3, Vector3};

use crate::{core::Entity, gameplay::{Positionable, Transformable}, physics::{collider::Collider, CollisionEvent}};

/// The `Body` trait represents a physical body in the world.
pub trait Body: Collider + Transformable + Entity {
	fn on_collision(&mut self) -> Option<&mut CollisionEvent>;

	fn body_type(&self) -> BodyTypes;

	fn velocity(&self) -> Vector3;
	fn inertia_tensor(&self) -> Matrix3 {
		match self.shape() {
			super::collider::Shapes::Sphere { radius } => {
				let mass = self.mass();
				let inertia = (2.0 / 5.0) * mass * radius * radius;
				Matrix3::new(
					inertia, 0.0, 0.0,
					0.0, inertia, 0.0,
					0.0, 0.0, inertia
				)
			}
			super::collider::Shapes::Cube { size: half_size } => {
				let mass = self.mass();
				Matrix3::identity()
			}
		}
	}

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
