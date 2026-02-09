use math::{Base, Matrix3, Vector3, magnitude, magnitude_squared, mat::MatNew3 as _};

use crate::{core::Entity, gameplay::{Positionable, Transformable}, physics::{collider::Collider}};

/// The `Body` trait represents a physical body in the world.
pub trait Body: Collider + Transformable {
	fn body_type(&self) -> BodyTypes;

	fn velocity(&self) -> Vector3;
	fn inertia_tensor(&self) -> Matrix3 {
		match self.shape() {
			super::collider::Shapes::Sphere { radius } => {
				let inertia = (2.0 / 5.0) * radius * radius;
				Matrix3::new(
					inertia, 0.0, 0.0,
					0.0, inertia, 0.0,
					0.0, 0.0, inertia
				)
			}
			super::collider::Shapes::Cube { size: half_size } => {
				let max = half_size;
				let min = -half_size;

				let dx = max.x - min.x;
				let dy = max.y - min.y;
				let dz = max.z - min.z;

				let dx2 = dx * dx;
				let dy2 = dy * dy;
				let dz2 = dz * dz;

				let tensor = Matrix3::new(
					(dy2 + dz2) / 12f32, 0.0, 0.0,
					0.0, (dx2 + dz2) / 12f32, 0.0,
					0.0, 0.0, (dx2 + dy2) / 12f32
				);

				let cm = Vector3::new((max.x + min.x) * 0.5, (max.y + min.y) * 0.5, (max.z + min.z) * 0.5);

				let r = Vector3::zero() - cm;
				let r2 = magnitude_squared(r);

				let rx2 = r.x * r.x;
				let ry2 = r.y * r.y;
				let rz2 = r.z * r.z;

				let pat_tensor = Matrix3::new(
					r2 - rx2, r.x * r.y, r.x * r.x,
					r.y * r.x, r2 - r.y * r.y, r.y * r.z,
					r.z * r.x, r.z * r.y, r2 - rz2
				);

				let inertia = Matrix3::new(
					tensor[0] + pat_tensor[0], tensor[1] + pat_tensor[1], tensor[2] + pat_tensor[2],
					pat_tensor[3], tensor[4] + pat_tensor[4], pat_tensor[5],
					pat_tensor[6], pat_tensor[7], tensor[8] + pat_tensor[8]
				);

				inertia
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
