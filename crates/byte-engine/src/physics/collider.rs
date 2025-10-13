use math::Vector3;

use crate::gameplay::Positionable;

/// The `Collider` trait allows an entity to present itself as a collision shape.
pub trait Collider: Positionable {
	/// Returns the shape of the collider.
	fn shape(&self) -> Shapes;

	/// Returns the elasticity of the body.
	fn elasticity(&self) -> f32 {
		0.1f32
	}

	/// Returns the friction of the body.
	fn friction(&self) -> f32 {
		0.1f32
	}
}

/// The `CollisionShapes` enum represents the different shapes that a collider can have.
#[derive(Debug, Clone, Copy)]
pub enum Shapes {
	/// A sphere shaped collider.
	Sphere {
		/// The radius of the sphere.
		radius: f32,
	},
	/// A cube shaped collider.
	Cube {
		/// The half-size of the cube
		size: Vector3,
	},
}

impl Shapes {
	/// Creates a new sphere shaped collider.
	/// The radius parameter is the radius of the sphere.
	pub fn sphere(radius: f32) -> Self {
		Self::Sphere { radius }
	}

	/// Creates a new cube shaped collider.
	/// The size parameter is the half-size of the cube.
	pub fn cube(size: Vector3) -> Self {
		Self::Cube { size }
	}
}
