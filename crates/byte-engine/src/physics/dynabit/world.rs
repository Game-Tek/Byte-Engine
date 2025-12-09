use math::{collision::{cube_vs_cube, sphere_vs_sphere, Intersection}, cross, cube::Cube, dot, length, magnitude, magnitude_squared, mat::{MatInverse as _, MatTranspose as _}, normalize, sphere::Sphere, Base, Matrix3, Quaternion, Vector3};
use crate::{application::Time, core::{entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle}, physics::{body::{Body, BodyTypes}, collider::{Collider, Shapes}, dynabit::{body::{intersect, PhysicsBody}, contact::{Contact, Side}}}};

pub struct World {
	bodies: Vec<PhysicsBody>,
	gravity: Vector3,
}

impl World {
	pub fn new() -> Self {
		Self {
			bodies: Vec::new(),
			gravity: Vector3::new(0f32, -16f32, 0f32),
		}
	}

	fn add_body(&mut self, body: PhysicsBody) -> usize {
		let index = self.bodies.len();
		self.bodies.push(body);
		index
	}

	pub fn apply_impulse(&mut self, entity: EntityHandle<dyn Body>, impulse: Vector3) {
		let entity = Some(entity);

		if let Some(body) = self.bodies.iter_mut().find(|e| &e.body == &entity) {
			body.apply_linear_impulse(impulse);
		}
	}

	pub fn update(&mut self, time: Time) {
		self.update_velocities(time);
		self.update_collisions(time);
		self.update_bodies(time);
		self.update_entities(time);
	}

	/// Applies all initial impulses to bodies based on forces.
	pub fn update_velocities(&mut self, time: Time) {
		let dt = time.delta();
		let dt = dt.as_secs_f32();

		for body in self.bodies.iter_mut() {
			match body.body_type {
				BodyTypes::Dynamic => {
					let forces = self.gravity;

					let mass = 1f32 / body.inv_mass;

					let impulse = forces * mass * dt;

					body.apply_linear_impulse(impulse);
				}
				_ => continue,
			}
		}
	}

	/// Calculates and solves collisions.
	pub fn update_collisions(&mut self, time: Time) {
		let contacts = self.detect_collisions(time);
		for contact in &contacts {
			self.resolve_contact(contact);
		}
	}

	fn detect_collisions(&self, time: Time) -> Vec<Contact> {
		let bodies = self.bodies.iter().enumerate();

		let mut contacts: Vec<Contact> = Vec::new();

		for (i, a) in bodies.clone() {
			for (j, b) in bodies.clone() {
				if i == j { continue; }

				let pair = (i, j);

				if contacts.iter().find(|c| (c.a.object, c.b.object) == pair).is_some() { continue; }

				let intersection = intersect(a, b);

				if let Some(intersection) = intersection {
					let normal = intersection.normal;

					let relative_velocity = b.linear_velocity - a.linear_velocity;
					let relative_velocity_along_normal = dot(relative_velocity, normal);

					let elasticity = a.elasticity * b.elasticity;
					let impulse = (1f32 + elasticity) * relative_velocity_along_normal / (a.inv_mass + b.inv_mass);
					let vector_impulse = normal * impulse;

					contacts.push(Contact {
						normal: intersection.normal,
						depth: intersection.depth,
						a: Side {
							object: pair.0,
							point: intersection.point_on_a,
						},
						b: Side {
							object: pair.1,
							point: intersection.point_on_b,
						},
					});
				}
			}
		}

		contacts
	}

	/// Updates bodies' positions and orientation based on their velocities.
	pub fn update_bodies(&mut self, time: Time) {
		let dt = time.delta();
		let dt = dt.as_secs_f32();

		for body in self.bodies.iter_mut() {
			match body.body_type {
				BodyTypes::Dynamic => {
					body.update(time);
				}
				_ => continue,
			}
		}
	}

	/// Synchronizes game bodies with their internal representation.
	pub fn update_entities(&mut self, time: Time) {
		let dt = time.delta();
		let dt = dt.as_secs_f32();

		for body in self.bodies.iter_mut() {
			match body.body_type {
				BodyTypes::Dynamic => {
					let mut e = body.body.as_ref().unwrap().write();
					e.set_position(body.position);
					e.transform_mut().set_orientation(body.orientation);
				}
				_ => continue,
			}
		}
	}

	fn resolve_contact(&mut self, contact: &Contact) {
		let a_index = contact.a.object;
		let b_index = contact.b.object;

		let a = &self.bodies[a_index];
		let b = &self.bodies[b_index];

		let a_point = contact.a.point;
		let b_point = contact.b.point;

		let a_elasticity = a.elasticity;
		let b_elasticity = b.elasticity;
		let elasticity = a_elasticity * b_elasticity;

		let a_friction = a.friction;
		let b_friction = b.friction;
		let friction = a_friction * b_friction;

		let a_inv_mass = a.inv_mass;
		let b_inv_mass = b.inv_mass;

		let a_inv_world_inertia = a.inverse_world_space_inertia_tensor();
		let b_inv_world_inertia = b.inverse_world_space_inertia_tensor();

		let n = contact.normal;

		let ra = a_point - a.world_space_center_of_mass();
		let rb = b_point - b.world_space_center_of_mass();

		let a_angular_j = cross(a_inv_world_inertia * cross(ra, n), ra);
		let b_angular_j = cross(b_inv_world_inertia * cross(rb, n), rb);
		let angular_factor = dot(a_angular_j + b_angular_j, n);

		let a_vel = a.linear_velocity + cross(a.angular_velocity, ra);
		let b_vel = b.linear_velocity + cross(b.angular_velocity, rb);

		let vab = a_vel - b_vel;

		let impulse = (1.0 + elasticity) * dot(vab, n) / (a_inv_mass + b_inv_mass + angular_factor);
		let impulse_vector = impulse * n;

		let a = &mut self.bodies[a_index];
		let t_a = a_inv_mass / (a_inv_mass + b_inv_mass);
		a.apply_impulse(a_point, -impulse_vector);

		let b = &mut self.bodies[b_index];
		let t_b = b_inv_mass / (a_inv_mass + b_inv_mass);
		b.apply_impulse(b_point, impulse_vector);

		let vel_normal = n * dot(vab, n);
		let vel_tangent = vab - vel_normal;

		let relative_vel_tangent = normalize(vel_tangent);

		let a_inertia = cross(a_inv_world_inertia * cross(ra, relative_vel_tangent), ra);
		let b_inertia = cross(b_inv_world_inertia * cross(rb, relative_vel_tangent), rb);
		let inv_inertia = dot(a_inertia + b_inertia, relative_vel_tangent);

		let reduced_mass = 1.0 / (a_inv_mass + b_inv_mass + inv_inertia);
		let impulse_friction = vel_tangent * reduced_mass * friction;

		let a = &mut self.bodies[a_index];
		a.apply_impulse(a_point, -impulse_friction);

		let b = &mut self.bodies[b_index];
		b.apply_impulse(b_point, impulse_friction);

		let separation = n * contact.depth;
		//let separation = b_point - a_point; // Book suggests this way but it causes orbiting around the world center
		let t_a = a_inv_mass / (a_inv_mass + b_inv_mass);
		let t_b = b_inv_mass / (a_inv_mass + b_inv_mass);

		let a = &mut self.bodies[a_index];
		a.position -= separation * t_a;

		let b = &mut self.bodies[b_index];
		b.position += separation * t_b;
	}
}

impl Entity for World {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self).listen_to::<CreateEvent<dyn Body>>().listen_to::<CreateEvent<dyn Collider>>()
	}
}

impl crate::physics::World for World {
	fn update(&mut self, time: Time) {
    	self.update(time);
	}
}

impl Listener<CreateEvent<dyn Body>> for World {
	fn handle(&mut self, event: &CreateEvent<dyn Body>) {
		let handle = event.handle();
		let body = handle.read();

		let body_type = body.body_type();

		let inv_mass = match body_type {
			BodyTypes::Dynamic => 1f32 / body.mass(),
			_ => 0f32,
		};

		let inertia_tensor = body.inertia_tensor();

		self.add_body(PhysicsBody{
			body_type,
			position: body.position(),
			orientation: Quaternion::identity(),
			linear_velocity: body.velocity(),
			angular_velocity: Vector3::new(0f32, 0f32, 0f32),
			acceleration: Vector3::new(0f32, 0f32, 0f32),
			collision_shape: body.shape(),
			collider: handle.clone() as EntityHandle<dyn Collider>,
			body: Some(handle.clone()),
			inv_mass,
			center_of_mass: body.center_of_mass(),
			elasticity: body.elasticity(),
			inertia_tensor,
			friction: body.friction(),
		});
	}
}

impl Listener<CreateEvent<dyn Collider>> for World {
	fn handle(&mut self, event: &CreateEvent<dyn Collider>) {
		let handle = event.handle();
		let collider = handle.read();
		self.add_body(PhysicsBody{
			body_type: BodyTypes::Static,
			position: collider.position(),
			orientation: Quaternion::identity(),
			linear_velocity: Vector3::zero(),
			angular_velocity: Vector3::new(0f32, 0f32, 0f32),
			acceleration: Vector3::zero(),
			collision_shape: collider.shape(),
			collider: handle.clone(),
			body: None,
			inv_mass: 0f32,
			center_of_mass: Vector3::zero(),
			elasticity: collider.elasticity(),
			inertia_tensor: Matrix3::identity(),
			friction: collider.friction(),
		});
	}
}
