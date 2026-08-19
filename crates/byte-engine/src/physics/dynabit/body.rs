use math::{
	collision::{aabb_vs_aabb, sphere_vs_aabb, sphere_vs_sphere_dynamic},
	Orientation, Point, Sphere, Vector, AABB,
};
use maths_rs::{
	mat::{MatInverse as _, MatScale as _, MatTranspose as _},
	vec::Magnitude as _,
	Mat3f, Vec3f,
};

use crate::{
	core::factory::Handle,
	physics::{body::BodyTypes, collider::Shapes, dynabit::contact::Contact, LocalSpace},
	time::MediaTime,
};

/// The `PhysicsBody` struct stores the mutable world-space state used by Dynabit.
#[derive(Clone)]
pub struct PhysicsBody {
	pub(crate) body_type: BodyTypes,
	pub(crate) collision_shape: Shapes,
	pub(crate) position: Point,
	pub(crate) orientation: Orientation,
	pub(crate) acceleration: Vector,
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
	/// Applies an impulse at a world-space contact point.
	pub fn apply_impulse(&mut self, point: Point, impulse: Vector) {
		if self.inv_mass == 0.0 {
			return;
		}
		self.apply_linear_impulse(impulse);
		self.apply_angular_impulse((point - self.world_space_center_of_mass()).cross(impulse));
	}

	/// Applies a world-space linear impulse.
	pub fn apply_linear_impulse(&mut self, impulse: Vector) {
		if self.inv_mass != 0.0 {
			self.linear_velocity += impulse * self.inv_mass;
		}
	}

	/// Applies a world-space angular impulse.
	pub fn apply_angular_impulse(&mut self, impulse: Vector) {
		if self.inv_mass == 0.0 {
			return;
		}
		self.angular_velocity += Vector::from_maths(self.inverse_world_space_inertia_tensor() * impulse.into_maths());
		const MAX_ANGULAR_SPEED: f32 = 30.0;
		if self.angular_velocity.length_squared() > MAX_ANGULAR_SPEED * MAX_ANGULAR_SPEED {
			self.angular_velocity = self.angular_velocity.normalized().expect("finite angular velocity") * MAX_ANGULAR_SPEED;
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

	/// Returns the inverse local inertia tensor as a raw matrix boundary value.
	pub fn inverse_body_space_inertia_tensor(&self) -> Mat3f {
		self.collision_shape.inertia_tensor().inverse()
			* Mat3f::from_scale(Vec3f::new(self.inv_mass, self.inv_mass, self.inv_mass))
	}

	/// Returns the inverse world inertia tensor as a raw matrix boundary value.
	pub fn inverse_world_space_inertia_tensor(&self) -> Mat3f {
		let rotation = self.orientation.into_maths().get_matrix();
		rotation * self.inverse_body_space_inertia_tensor() * rotation.transpose()
	}

	/// Advances the body by `dt` using its current linear and angular velocities.
	pub fn update(&mut self, dt: MediaTime) {
		let seconds = dt.as_seconds_f32();
		self.position += self.linear_velocity * seconds;

		let center_of_mass = self.world_space_center_of_mass();
		let center_offset = self.position - center_of_mass;
		let rotation = self.orientation.into_maths().get_matrix();
		let inertia = rotation * self.collision_shape.inertia_tensor() * rotation.transpose();
		let angular_momentum = Vector::from_maths(inertia * self.angular_velocity.into_maths());
		let angular_acceleration =
			Vector::from_maths(inertia.inverse() * self.angular_velocity.cross(angular_momentum).into_maths());
		self.angular_velocity += angular_acceleration * seconds;

		let angular_step = self.angular_velocity * seconds;
		// Check the axis and measure its rotation in one operation.
		let delta_orientation = angular_step
			.normalize_with_length()
			.map(|(axis, angle)| {
				Orientation::try_from_axis_angle(axis, math::Radians::new(angle))
					.expect("a finite unit axis and finite angle form an orientation")
			})
			.unwrap_or_else(|_| Orientation::identity());
		self.orientation = delta_orientation.compose(self.orientation);
		self.position = center_of_mass + delta_orientation.rotate_vector(center_offset);
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
		(Shapes::ConvexHull { .. }, _) | (_, Shapes::ConvexHull { .. }) => return None,
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
			acceleration: Vector::zero(),
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
	fn bounds_are_world_space_even_when_shape_data_is_local() {
		let mut body = body();
		body.position = Point::new(3.0, 0.0, 0.0);

		assert_eq!(body.bounds().min(), Point::new(2.0, -1.0, -1.0));
	}
}
