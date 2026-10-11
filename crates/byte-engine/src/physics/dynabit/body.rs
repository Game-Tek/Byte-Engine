use math::{
	AABB, Orientation, Point, Scale, Sphere, Vector,
	collision::{aabb_vs_aabb, sphere_vs_aabb, sphere_vs_sphere, sphere_vs_sphere_dynamic},
};

use crate::{
	core::factory::Handle,
	physics::{LocalSpace, body::BodyTypes, collider::Shapes, dynabit::contact::Contact},
	time::MediaTime,
};

/// The `PhysicsBody` struct stores the mutable world-space state used by Dynabit.
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

	/// Returns how easily contacts turn the body about each of its local axes: its inverse mass over each moment of
	/// inertia, or zero for a body that contacts do not turn.
	///
	/// A box collides as an axis-aligned box whatever its orientation. If contacts turned it, friction would spin
	/// it until it rolled along on its faces like a wheel, so contacts only move it.
	pub(crate) fn inverse_moments(&self) -> [f32; 3] {
		match self.collision_shape {
			Shapes::Cube { .. } => [0.0; 3],
			Shapes::Sphere { .. } => {
				let moments = self.collision_shape.principal_inertia();
				[
					self.inv_mass / moments.x,
					self.inv_mass / moments.y,
					self.inv_mass / moments.z,
				]
			}
		}
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
		// frame, where the inertia tensor is the diagonal of those moments, and then rotated into the world. With no
		// torque acting, angular momentum stays as it is, so the spin changes by the inverse inertia times the cross
		// product of the momentum with the spin. A body that is not spinning, as boxes never are, has none to change.
		let angular_velocity = if self.angular_velocity.to_array() == [0.0; 3] {
			self.angular_velocity
		} else {
			let moments = self.collision_shape.principal_inertia();
			let local_velocity = self.orientation.inverse().rotate_vector(self.angular_velocity).into_maths();
			let local_acceleration = maths_rs::cross(local_velocity * moments, local_velocity) / moments;
			self.angular_velocity + self.orientation.rotate_vector(Vector::from_maths(local_acceleration)) * seconds
		};

		// A spin that is not finite turns the body by nothing.
		let delta_orientation =
			Orientation::try_from_rotation_vector(angular_velocity * seconds).unwrap_or_else(|_| Orientation::identity());
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

	/// Returns whether the body travels far enough in the next `dt` seconds that [`intersect`] tests it along its
	/// path instead of where it is now.
	///
	/// Only a sphere is tested along its path, and only once a step carries it farther than half its radius. A
	/// slower sphere cannot pass through another body between two steps: whatever it reaches, it still overlaps
	/// when the step ends. Testing it in place lets its contacts be found and resolved together with the others.
	pub fn outruns_its_shape(&self, dt: f32) -> bool {
		match self.collision_shape {
			Shapes::Sphere { radius } => {
				let reach = radius * 0.5;
				self.linear_velocity.length_squared() * dt * dt > reach * reach
			}
			Shapes::Cube { .. } => false,
		}
	}

	/// Returns world-space bounds around everywhere [`intersect`] can find this body in contact during the next
	/// `dt` seconds.
	///
	/// A body that [`Self::outruns_its_shape`] is tested along its path, so its bounds cover the path at its
	/// current linear velocity, slightly enlarged so bodies that are about to touch still count as overlapping.
	/// Any other body is only tested where it is now, so its bounds are exact and no pair is tested that cannot be
	/// in contact. Pass the bounds to [`crate::physics::SweepBounds::rebuild`] to find the bodies this one may
	/// collide with.
	pub fn swept_bounds(&self, dt: f32) -> AABB {
		debug_assert!(
			dt.is_finite() && dt >= 0.0,
			"Swept bounds interval is invalid. The most likely cause is passing a negative or non-finite frame interval."
		);
		let current = self.bounds();
		if !self.outruns_its_shape(dt) {
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
	let touching = match (&a.collision_shape, &b.collision_shape) {
		(Shapes::Sphere { radius: a_radius }, Shapes::Sphere { radius: b_radius }) => {
			let (a_sphere, b_sphere) = (Sphere::new(a.position, *a_radius), Sphere::new(b.position, *b_radius));
			if a.outruns_its_shape(dt) || b.outruns_its_shape(dt) {
				// A fast sphere could pass through the other one between two steps, so find where their paths meet.
				let impact = sphere_vs_sphere_dynamic(&a_sphere, &b_sphere, a.linear_velocity, b.linear_velocity, dt)?;
				let mut contact = Contact::from_dynamic(i, j, impact);
				// The points are where the spheres will touch. Contacts are solved from where the bodies are now, so
				// the points move back to where they are on the spheres now.
				contact.a.point = contact.a.point - a.linear_velocity * contact.toi;
				contact.b.point = contact.b.point - b.linear_velocity * contact.toi;
				return Some(contact);
			}
			sphere_vs_sphere(&a_sphere, &b_sphere)?
		}
		(Shapes::Cube { .. }, Shapes::Cube { .. }) => aabb_vs_aabb(&a.bounds(), &b.bounds())?,
		(Shapes::Sphere { radius }, Shapes::Cube { .. }) => sphere_vs_aabb(&Sphere::new(a.position, *radius), &b.bounds())?,
		(Shapes::Cube { .. }, Shapes::Sphere { radius }) => {
			sphere_vs_aabb(&Sphere::new(b.position, *radius), &a.bounds())?.swap()
		}
	};
	Some(Contact::from_static(i, j, touching))
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
	fn swept_bounds_of_a_slow_sphere_are_its_bounds() {
		let mut body = body();
		body.position = Point::new(2.0, 0.0, 0.0);
		// The sphere has a radius of one and moves less than half of it in the step.
		body.linear_velocity = Vector::new(0.3, 0.0, 0.0);

		let swept = body.swept_bounds(1.0);

		assert_eq!(swept.min(), body.bounds().min());
		assert_eq!(swept.max(), body.bounds().max());
	}

	#[test]
	fn slow_spheres_touch_only_where_they_are_now() {
		let mut first = body();
		first.linear_velocity = Vector::new(0.3, 0.0, 0.0);
		let mut apart = body();
		apart.position = Point::new(2.2, 0.0, 0.0);
		let mut overlapping = body();
		overlapping.position = Point::new(1.5, 0.0, 0.0);

		// The first sphere reaches the second during the step, but it is tested where it starts.
		assert!(intersect((&first, 0), (&apart, 1), 1.0).is_none());
		let contact = intersect((&first, 0), (&overlapping, 1), 1.0).expect("overlapping spheres are in contact");
		assert_eq!(contact.toi, 0.0);
		assert!((contact.depth - 0.5).abs() <= 1e-6);
	}

	#[test]
	fn a_fast_sphere_touches_where_its_path_meets_another() {
		let mut first = body();
		first.linear_velocity = Vector::new(4.0, 0.0, 0.0);
		let mut apart = body();
		apart.position = Point::new(4.0, 0.0, 0.0);

		// The spheres have a radius of one, so they touch after the first has moved two units.
		let contact = intersect((&first, 0), (&apart, 1), 1.0).expect("the path reaches the other sphere");
		assert!((contact.toi - 0.5).abs() <= 1e-6);
		// The contact points are the facing points of the spheres where they are now.
		assert!((contact.a.point - Point::new(1.0, 0.0, 0.0)).length() <= 1e-5);
		assert!((contact.b.point - Point::new(3.0, 0.0, 0.0)).length() <= 1e-5);
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
	fn a_spinning_body_with_unequal_moments_keeps_its_angular_momentum() {
		let mut body = body();
		// A box of 1 by 2 by 3 units has unit-mass moments of 13/12, 10/12, and 5/12.
		body.collision_shape = Shapes::cube(Vector::new(0.5, 1.0, 1.5));
		body.angular_velocity = Vector::new(1.0, 1.0, 0.0);

		body.update(MediaTime::from_seconds_f32(0.01));

		// With no torque, the spin about the third axis changes at (13/12 - 10/12) / (5/12) = 0.6 per second.
		assert!((body.angular_velocity.z() - 0.006).abs() <= 1e-5);
	}

	#[test]
	fn bounds_are_world_space_even_when_shape_data_is_local() {
		let mut body = body();
		body.position = Point::new(3.0, 0.0, 0.0);

		assert_eq!(body.bounds().min(), Point::new(2.0, -1.0, -1.0));
	}
}
