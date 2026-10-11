use math::{
	AABB, Orientation, Point, Scale, Sphere, Vector,
	collision::{aabb_vs_aabb, sphere_vs_aabb, sphere_vs_sphere_dynamic},
};
use maths_rs::mat::MatTranspose as _;

use crate::{
	core::factory::Handle,
	physics::{LocalSpace, body::BodyTypes, collider::Shapes, dynabit::contact::Contact},
	time::MediaTime,
};

/// The fastest a contact may leave a body spinning, in radians per second.
pub(crate) const MAX_ANGULAR_SPEED: f32 = 30.0;

/// The `PhysicsBody` struct stores the mutable world-space state used by Dynabit.
#[derive(Clone)]
pub struct PhysicsBody {
	pub(crate) body_type: BodyTypes,
	pub(crate) collision_shape: Shapes,
	pub(crate) position: Point,
	pub(crate) orientation: Orientation,
	pub(crate) scale: Scale,
	pub(crate) linear_velocity: Vector,
	pub(crate) angular_velocity: Vector,
	/// Reciprocal mass in kilograms.
	pub(crate) inv_mass: f32,
	/// Center of mass relative to the collider origin.
	pub(crate) center_of_mass: Point<LocalSpace>,
	pub(crate) elasticity: f32,
	pub(crate) friction: f32,
	pub(crate) handle: Handle,
}

impl PhysicsBody {
	/// Applies a world-space linear impulse.
	pub fn apply_linear_impulse(&mut self, impulse: Vector) {
		if self.inv_mass != 0.0 {
			self.linear_velocity += impulse * self.inv_mass;
		}
	}

	/// Returns the mass in kilograms.
	pub fn mass(&self) -> f32 {
		1.0 / self.inv_mass
	}

	/// Returns the center of mass in world coordinates.
	pub fn world_space_center_of_mass(&self) -> Point {
		let local_offset = self.center_of_mass - Point::origin();
		// Rotating a local offset into the world frame crosses a spatial math boundary.
		self.position + Vector::from_maths(self.orientation.into_maths() * local_offset.into_maths())
	}

	/// Advances the body by `dt` using its current linear and angular velocities.
	pub fn update(&mut self, dt: MediaTime) {
		let motion = self.advanced(dt);
		self.apply(motion);
	}

	/// Returns where the body is, and how it spins, after moving for `dt` at its current velocities.
	///
	/// This reads the body without changing it, so lanes can advance bodies side by side. Write the result back
	/// with [`Self::apply`].
	pub(crate) fn advanced(&self, dt: MediaTime) -> Motion {
		let seconds = dt.as_seconds_f32();
		let travel = self.linear_velocity * seconds;
		let position = self.position + travel;
		let center_of_mass = self.world_space_center_of_mass() + travel;
		let center_offset = position - center_of_mass;

		// The spin changes when the body's moments of inertia differ. That change is worked out in the body's own
		// frame, where the inertia tensor is the diagonal of those moments, and then rotated into the world.
		let rotation = self.orientation.into_maths().get_matrix();
		let moments = self.collision_shape.principal_inertia();
		let local_velocity = rotation.transpose() * self.angular_velocity.into_maths();
		let local_acceleration = maths_rs::cross(local_velocity, local_velocity * moments) / moments;
		let angular_velocity = self.angular_velocity + Vector::from_maths(rotation * local_acceleration) * seconds;

		let angular_step = angular_velocity * seconds;
		// Check the axis and measure its rotation in one operation.
		let delta_orientation = angular_step
			.normalize_with_length()
			.map(|(axis, angle)| {
				Orientation::try_from_axis_angle(axis, math::Radians::new(angle))
					.expect("a finite unit axis and finite angle form an orientation")
			})
			.unwrap_or_else(|_| Orientation::identity());
		Motion {
			position: center_of_mass + delta_orientation.rotate_vector(center_offset),
			orientation: delta_orientation.compose(self.orientation),
			angular_velocity,
		}
	}

	/// Moves the body to where [`Self::advanced`] found it to be.
	pub(crate) fn apply(&mut self, motion: Motion) {
		self.position = motion.position;
		self.orientation = motion.orientation;
		self.angular_velocity = motion.angular_velocity;
	}

	/// Returns the current world-space axis-aligned bounds.
	pub fn bounds(&self) -> AABB {
		let local = self.collision_shape.bounds();
		// Crossing from authored local geometry to world simulation requires an explicit raw boundary conversion.
		AABB::new(
			self.position + Vector::from_maths(local.min().into_maths()),
			self.position + Vector::from_maths(local.max().into_maths()),
		)
	}

	/// Returns world-space bounds around everywhere [`intersect`] can find this body in contact during the next
	/// `dt` seconds.
	///
	/// A sphere is tested along its path, so its bounds cover the path at its current linear velocity, slightly
	/// enlarged so spheres that are about to touch still count as overlapping. A box is only tested where it is
	/// now, so its bounds are exact and no pair is tested that cannot be in contact. Pass the bounds to
	/// [`crate::physics::SweepBounds::rebuild`] to find the bodies this one may collide with.
	pub fn swept_bounds(&self, dt: f32) -> AABB {
		debug_assert!(
			dt.is_finite() && dt >= 0.0,
			"Swept bounds interval is invalid. The most likely cause is passing a negative or non-finite frame interval."
		);
		let current = self.bounds();
		if matches!(self.collision_shape, Shapes::Cube { .. }) {
			return current;
		}
		const MARGIN: f32 = 0.01;
		let margin = Vector::new(MARGIN, MARGIN, MARGIN);
		let travel = self.linear_velocity * dt;
		let min = maths_rs::min(current.min().into_maths(), (current.min() + travel).into_maths());
		let max = maths_rs::max(current.max().into_maths(), (current.max() + travel).into_maths());
		AABB::new(Point::from_maths(min) - margin, Point::from_maths(max) + margin)
	}
}

/// The `Motion` struct holds the state a body has after moving for a while, between working it out and writing it
/// to the body.
pub(crate) struct Motion {
	position: Point,
	orientation: Orientation,
	angular_velocity: Vector,
}

/// Finds a collision contact for the two world-space bodies.
pub fn intersect((a, i): (&PhysicsBody, usize), (b, j): (&PhysicsBody, usize), dt: f32) -> Option<Contact> {
	let contact = match (&a.collision_shape, &b.collision_shape) {
		(Shapes::Sphere { radius: a_radius }, Shapes::Sphere { radius: b_radius }) => {
			let contact = sphere_vs_sphere_dynamic(
				&Sphere::new(a.position, *a_radius),
				&Sphere::new(b.position, *b_radius),
				a.linear_velocity,
				b.linear_velocity,
				dt,
			)?;
			Contact::from_dynamic(i, j, contact)
		}
		(Shapes::Cube { .. }, Shapes::Cube { .. }) => Contact::from_static(i, j, aabb_vs_aabb(&a.bounds(), &b.bounds())?, 0.0),
		(Shapes::Sphere { radius }, Shapes::Cube { .. }) => {
			Contact::from_static(i, j, sphere_vs_aabb(&Sphere::new(a.position, *radius), &b.bounds())?, 0.0)
		}
		(Shapes::Cube { .. }, Shapes::Sphere { radius }) => Contact::from_static(
			i,
			j,
			sphere_vs_aabb(&Sphere::new(b.position, *radius), &a.bounds())?.swap(),
			0.0,
		),
	};
	Some(contact)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::physics::Shapes;

	fn body() -> PhysicsBody {
		PhysicsBody {
			body_type: BodyTypes::Dynamic,
			collision_shape: Shapes::Sphere { radius: 1.0 },
			position: Point::origin(),
			orientation: Orientation::identity(),
			scale: Scale::identity(),
			linear_velocity: Vector::zero(),
			angular_velocity: Vector::new(0.0, 1.0, 0.0),
			inv_mass: 1.0,
			center_of_mass: Point::origin(),
			elasticity: 0.0,
			friction: 1.0,
			handle: crate::core::factory::Factory::<()>::new().create(()),
		}
	}

	#[test]
	fn swept_bounds_reach_where_a_fast_body_ends_the_step() {
		let mut body = body();
		body.position = Point::new(-5.0, -5.0, -5.0);
		body.linear_velocity = Vector::new(10.0, 10.0, 10.0);

		let swept = body.swept_bounds(1.0);

		// The body has a radius of one and ends the step centered five units past the origin.
		assert!(swept.min().x() <= -6.0 && swept.max().x() >= 6.0);
		assert!(swept.min().y() <= -6.0 && swept.max().y() >= 6.0);
		assert!(swept.min().z() <= -6.0 && swept.max().z() >= 6.0);
	}

	#[test]
	fn swept_bounds_of_a_box_are_its_bounds() {
		let mut body = body();
		body.collision_shape = Shapes::cube(Vector::new(0.5, 1.0, 1.5));
		body.position = Point::new(2.0, 0.0, 0.0);
		body.linear_velocity = Vector::new(10.0, 10.0, 10.0);

		let swept = body.swept_bounds(1.0);

		assert_eq!(swept.min(), body.bounds().min());
		assert_eq!(swept.max(), body.bounds().max());
	}

	#[test]
	fn bounds_are_world_space_even_when_shape_data_is_local() {
		let mut body = body();
		body.position = Point::new(3.0, 0.0, 0.0);

		assert_eq!(body.bounds().min(), Point::new(2.0, -1.0, -1.0));
	}
}
