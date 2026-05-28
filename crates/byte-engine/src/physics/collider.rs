/// The `Collider` trait allows an entity to present itself as a collision shape.
pub trait Collider: Positionable {
	/// Returns the shape of the collider.
	fn shape(&self) -> Shapes;

	/// Returns the elasticity of the body.
	///
	/// Implement this method to provide a custom elasticity value for the collider.
	fn elasticity(&self) -> f32 {
		0.1f32
	}

	/// Returns the friction of the body.
	///
	/// Implement this method to provide a custom friction value for the collider.
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

	pub fn inertia_tensor(&self) -> Matrix3 {
		match self {
			Self::Sphere { radius } => {
				let inertia = (2.0 / 5.0) * radius * radius;
				Matrix3::new(inertia, 0.0, 0.0, 0.0, inertia, 0.0, 0.0, 0.0, inertia)
			}
			Self::Cube { size: half_size } => {
				let max = half_size;
				let min = -*half_size;

				let dx = max.x - min.x;
				let dy = max.y - min.y;
				let dz = max.z - min.z;

				let dx2 = dx * dx;
				let dy2 = dy * dy;
				let dz2 = dz * dz;

				let tensor = Matrix3::new(
					(dy2 + dz2) / 12f32,
					0.0,
					0.0,
					0.0,
					(dx2 + dz2) / 12f32,
					0.0,
					0.0,
					0.0,
					(dx2 + dy2) / 12f32,
				);

				let cm = Vector3::new((max.x + min.x) * 0.5, (max.y + min.y) * 0.5, (max.z + min.z) * 0.5);

				let r = Vector3::zero() - cm;
				let r2 = magnitude_squared(r);

				let rx2 = r.x * r.x;
				let ry2 = r.y * r.y;
				let rz2 = r.z * r.z;

				let pat_tensor = Matrix3::new(
					r2 - rx2,
					r.x * r.y,
					r.x * r.x,
					r.y * r.x,
					r2 - r.y * r.y,
					r.y * r.z,
					r.z * r.x,
					r.z * r.y,
					r2 - rz2,
				);

				let inertia = Matrix3::new(
					tensor[0] + pat_tensor[0],
					tensor[1] + pat_tensor[1],
					tensor[2] + pat_tensor[2],
					pat_tensor[3],
					tensor[4] + pat_tensor[4],
					pat_tensor[5],
					pat_tensor[6],
					pat_tensor[7],
					tensor[8] + pat_tensor[8],
				);

				inertia
			}
		}
	}
}

use math::{magnitude_squared, mat::MatNew3 as _, Base as _, Matrix3, Vector3};

use crate::space::Positionable;
