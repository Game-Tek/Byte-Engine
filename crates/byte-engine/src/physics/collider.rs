use math::Vector3;

use crate::gameplay::Positionable;

/// The `Collider` trait allows an entity to present itself as a collision shape.
pub trait Collider: Positionable {
	/// Returns the shape of the collider.
	fn shape(&self) -> CollisionShapes;
}

/// The `CollisionShapes` enum represents the different shapes that a collider can have.
#[derive(Debug, Clone)]
pub enum CollisionShapes {
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

impl CollisionShapes {
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
