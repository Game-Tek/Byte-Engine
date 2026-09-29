use math::{AABB, Point, Vector};
use maths_rs::Vec3f;

use crate::physics::LocalSpace;

/// The `Shapes` enum selects the local-space geometry used for collision detection.
#[derive(Debug, Clone)]
pub enum Shapes {
	/// A spherical collider centered on the local origin.
	Sphere { radius: f32 },
	/// An axis-aligned box represented by local half-extents.
	Cube { size: Vector<LocalSpace> },
}

impl Shapes {
	/// Creates a spherical collider with `radius`.
	pub fn sphere(radius: f32) -> Self {
		Self::Sphere { radius }
	}

	/// Creates a box collider with local half-extents.
	pub fn cube(size: Vector<LocalSpace>) -> Self {
		Self::Cube { size }
	}

	/// Returns the unit-mass moments of inertia about the collider's local axes.
	///
	/// Every supported shape is symmetric about its local axes, so these moments are the diagonal of its local
	/// inertia tensor and every other entry is zero. Bodies invert the tensor by taking reciprocals of these values.
	pub fn principal_inertia(&self) -> Vec3f {
		let half_extents = match self {
			Self::Sphere { radius } => {
				let inertia = 0.4 * radius * radius;
				return Vec3f::new(inertia, inertia, inertia);
			}
			Self::Cube { size } => *size,
		};
		let x = 2.0 * half_extents.x().abs();
		let y = 2.0 * half_extents.y().abs();
		let z = 2.0 * half_extents.z().abs();
		Vec3f::new((y * y + z * z) / 12.0, (x * x + z * z) / 12.0, (x * x + y * y) / 12.0)
	}

	/// Returns local axis-aligned bounds for this shape.
	pub fn bounds(&self) -> AABB<LocalSpace> {
		match self {
			Self::Sphere { radius } => {
				AABB::from_center_and_half_extents(Point::origin(), Vector::new(*radius, *radius, *radius))
			}
			Self::Cube { size } => AABB::from_center_and_half_extents(Point::origin(), *size),
		}
	}
}
