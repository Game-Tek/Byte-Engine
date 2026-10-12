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
	/// An upright capsule: a cylinder along the local y-axis with a half sphere on each end, which suits a standing
	/// character.
	///
	/// Like a box, a capsule does not follow the body's orientation, so a character can face any way and stay
	/// upright, and contacts never tip it over.
	Capsule {
		/// The radius of the cylinder and of both caps.
		radius: f32,
		/// The distance from the bottom of the lower cap to the top of the upper one. A height under twice the
		/// radius gives a sphere.
		height: f32,
		/// The middle of the capsule, relative to the collider origin.
		center: Point<LocalSpace>,
	},
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

	/// Creates an upright capsule collider that is `height` tall, caps included, around `center`.
	///
	/// A character's origin is usually at its feet. Pass a `center` half the height above the origin to stand such
	/// a character's capsule on its origin, then give the shape to [`crate::physics::Body::new`].
	pub fn capsule(radius: f32, height: f32, center: Point<LocalSpace>) -> Self {
		Self::Capsule { radius, height, center }
	}

	/// Returns the unit-mass moments of inertia about the collider's local axes.
	///
	/// Every supported shape is symmetric about the local axes through its middle, so these moments are the diagonal
	/// of its local inertia tensor and every other entry is zero. Bodies invert the tensor by taking reciprocals of
	/// these values.
	pub fn principal_inertia(&self) -> Vec3f {
		let half_extents = match self {
			Self::Sphere { radius } => {
				let inertia = sphere_moment(*radius);
				return Vec3f::new(inertia, inertia, inertia);
			}
			Self::Cube { size } => *size,
			Self::Capsule { radius, height, .. } => return capsule_principal_inertia(*radius, *height),
		};
		let x = 2.0 * half_extents.x().abs();
		let y = 2.0 * half_extents.y().abs();
		let z = 2.0 * half_extents.z().abs();
		Vec3f::new((y * y + z * z) / 12.0, (x * x + z * z) / 12.0, (x * x + y * y) / 12.0)
	}

	/// Returns local axis-aligned bounds for this shape.
	// Every step reads the bounds of every body, so this has to stay small enough for the compiler to inline. When
	// one arm called a helper it no longer was, and steps of piles were 5% to 7% slower on an Apple M4.
	pub fn bounds(&self) -> AABB<LocalSpace> {
		let (center, half_extents) = match self {
			Self::Sphere { radius } => (Point::origin(), Vector::new(*radius, *radius, *radius)),
			Self::Cube { size } => (Point::origin(), *size),
			Self::Capsule { radius, height, center } => (*center, Vector::new(*radius, (height * 0.5).max(*radius), *radius)),
		};
		AABB::from_center_and_half_extents(center, half_extents)
	}
}

/// Returns the unit-mass moment of inertia of a solid sphere about any axis through its middle.
pub(crate) fn sphere_moment(radius: f32) -> f32 {
	0.4 * radius * radius
}

/// Returns how far the center of each cap of a capsule is from its middle, which is half the length of the cylinder
/// between the caps. A capsule no taller than it is wide has no cylinder, so its caps share one center.
pub(crate) fn capsule_reach(radius: f32, height: f32) -> f32 {
	(height * 0.5 - radius).max(0.0)
}

/// Returns the unit-mass moments of inertia of a solid capsule about the local axes through its middle.
fn capsule_principal_inertia(radius: f32, height: f32) -> Vec3f {
	// A solid capsule is a cylinder between two half spheres. Each has the share of the mass that it has of the
	// volume, which is as `1.5 * length` to `2 * radius`.
	let length = 2.0 * capsule_reach(radius, height);
	let cylinder = 1.5 * length / (1.5 * length + 2.0 * radius);
	let caps = 1.0 - cylinder;
	let around = (0.5 * cylinder + 0.4 * caps) * radius * radius;
	// The caps sit half the cylinder's length from the middle, which adds to how much they resist turning end over
	// end.
	let across = cylinder * (length * length / 12.0 + radius * radius / 4.0)
		+ caps * (0.4 * radius * radius + length * length / 4.0 + 0.375 * length * radius);
	Vec3f::new(across, around, across)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::physics::{Body, BodyTypes};

	#[test]
	fn capsule_bounds_reach_its_height_and_radius_around_its_center() {
		// A character 1.8 tall whose origin is at its feet.
		let bounds = Shapes::capsule(0.3, 1.8, Point::new(0.0, 0.9, 0.0)).bounds();

		assert!((bounds.min() - Point::new(-0.3, 0.0, -0.3)).length() <= 1e-6);
		assert!((bounds.max() - Point::new(0.3, 1.8, 0.3)).length() <= 1e-6);
	}

	#[test]
	fn a_capsule_no_taller_than_it_is_wide_is_a_sphere() {
		let capsule = Shapes::capsule(0.5, 0.4, Point::origin());
		let sphere = Shapes::sphere(0.5);

		assert_eq!(capsule.bounds(), sphere.bounds());
		assert_eq!(capsule.principal_inertia(), sphere.principal_inertia());
	}

	#[test]
	fn a_tall_capsule_is_harder_to_turn_end_over_end_than_around_its_length() {
		let moments = Shapes::capsule(0.3, 1.8, Point::origin()).principal_inertia();

		assert!(moments.x > moments.y);
		assert_eq!(moments.x, moments.z);
		// It is slimmer than a sphere as tall as it and no slimmer than a rod as tall as it, whose moment about its
		// middle is a twelfth of its length squared.
		assert!(moments.x < 0.4 * 0.9 * 0.9);
		assert!(moments.x > 1.8 * 1.8 / 12.0 * 0.5);
	}

	#[test]
	fn a_body_starts_with_its_center_of_mass_in_the_middle_of_its_capsule() {
		let center = Point::new(0.0, 0.9, 0.0);
		let body = Body::new(BodyTypes::Dynamic, Shapes::capsule(0.3, 1.8, center));

		assert!((body.center_of_mass() - center).length() <= 1e-6);
		assert_eq!(
			Body::new(BodyTypes::Dynamic, Shapes::sphere(1.0)).center_of_mass(),
			Point::origin()
		);
	}
}
