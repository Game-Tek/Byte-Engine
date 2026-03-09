use std::ops::Deref;

use crate::{
	application::Time,
	core::{
		channel::Channel,
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
		Entity, EntityHandle,
	},
	gameplay::transform::{Transform, TransformationUpdate},
	physics::{
		body::{Body, BodyTypes},
		collider::{Collider, Shapes},
		dynabit::{
			body::{intersect, PhysicsBody},
			contact::{Contact, Side},
		},
	},
};
use math::{
	collision::{cube_vs_cube, sphere_vs_sphere, Intersection},
	cross,
	cube::Cube,
	dot, length, magnitude, magnitude_squared,
	mat::{MatInverse as _, MatTranspose as _},
	normalize,
	sphere::Sphere,
	Base, Matrix3, Quaternion, Vector3,
};

use utils::hash::{HashMap, HashMapExt};

/// Detects intersections and builds contact data for each unique body pair.
fn detect_collisions_for_bodies(bodies: &[PhysicsBody]) -> Vec<Contact> {
	let mut contacts = Vec::new();

	for i in 0..bodies.len() {
		for j in (i + 1)..bodies.len() {
			let a = &bodies[i];
			let b = &bodies[j];

			let Some(intersection) = intersect(a, b) else {
				continue;
			};

			contacts.push(Contact {
				normal: intersection.normal,
				depth: intersection.depth,
				a: Side {
					object: i,
					point: intersection.point_on_a,
				},
				b: Side {
					object: j,
					point: intersection.point_on_b,
				},
			});
		}
	}

	contacts
}

pub struct World {
	bodies: Vec<PhysicsBody>,
	gravity: Vector3,

	body_listener: DefaultListener<CreateMessage<EntityHandle<dyn Body>>>,

	handles_to_bodies: HashMap<Handle, usize>,
}

impl World {
	pub fn new(body_listener: DefaultListener<CreateMessage<EntityHandle<dyn Body>>>) -> Self {
		Self {
			bodies: Vec::new(),
			gravity: Vector3::new(0f32, -16f32, 0f32),
			body_listener,

			handles_to_bodies: HashMap::with_capacity(1024),
		}
	}

	fn add_body(&mut self, body: PhysicsBody) -> usize {
		let index = self.bodies.len();
		self.bodies.push(body);
		index
	}

	pub fn apply_impulse(&mut self, entity: EntityHandle<dyn Body>, impulse: Vector3) {
		let entity = Some(entity);

		// if let Some(body) = self.bodies.iter_mut().find(|e| &e.body == &entity) {
		// 	body.apply_linear_impulse(impulse);
		// }
	}

	pub fn update(
		&mut self,
		time: Time,
		transforms_rx: &mut impl Listener<TransformationUpdate>,
		transforms_tx: &mut impl Channel<TransformationUpdate>,
	) {
		while let Some(message) = self.body_listener.read() {
			let handle = message.handle().clone();
			let body_handle = message.into_data();
			let body = body_handle;

			self.create_body(handle, body.deref());
		}

		while let Some(message) = transforms_rx.read() {
			let transform = message.transform();
			let handle = message.handle();

			let idx = self.handles_to_bodies.get(handle).copied();

			if let Some(idx) = idx {
				let body = &mut self.bodies[idx];
				body.position = transform.get_position();
				body.orientation = transform.get_orientation();
			}
		}

		self.update_velocities(time);
		self.update_collisions(time);
		self.update_bodies(time, transforms_tx);
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

	fn detect_collisions(&self, _time: Time) -> Vec<Contact> {
		detect_collisions_for_bodies(&self.bodies)
	}

	/// Updates bodies' positions and orientation based on their velocities.
	pub fn update_bodies(&mut self, time: Time, transforms_tx: &mut impl Channel<TransformationUpdate>) {
		let dt = time.delta();
		let dt = dt.as_secs_f32();

		for body in self.bodies.iter_mut() {
			match body.body_type {
				BodyTypes::Dynamic => {
					body.update(time);

					transforms_tx.send(TransformationUpdate::new(
						body.handle,
						Transform::new(body.position, Vector3::one(), body.orientation),
					));
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
		let inv_mass_sum = a_inv_mass + b_inv_mass;

		if inv_mass_sum == 0.0 {
			return;
		}

		let a_inv_world_inertia = a.inverse_world_space_inertia_tensor();
		let b_inv_world_inertia = b.inverse_world_space_inertia_tensor();

		let mut n = contact.normal;
		let center_delta = b.world_space_center_of_mass() - a.world_space_center_of_mass();
		// Keep normals oriented from A toward B so separation always moves bodies apart.
		if dot(center_delta, n) < 0.0 {
			n = -n;
		}

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
		a.apply_impulse(a_point, -impulse_vector);

		let b = &mut self.bodies[b_index];
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
		let t_a = a_inv_mass / inv_mass_sum;
		let t_b = b_inv_mass / inv_mass_sum;

		let a = &mut self.bodies[a_index];
		a.position -= separation * t_a;

		let b = &mut self.bodies[b_index];
		b.position += separation * t_b;
	}

	fn create_body(&mut self, handle: Handle, body: &dyn Body) {
		let body_type = body.body_type();

		let inv_mass = match body_type {
			BodyTypes::Dynamic => 1f32 / body.mass(),
			_ => 0f32,
		};

		let inertia_tensor = body.inertia_tensor();

		self.add_body(PhysicsBody {
			body_type,
			position: body.position(),
			orientation: Quaternion::identity(),
			linear_velocity: body.velocity(),
			angular_velocity: Vector3::new(0f32, 0f32, 0f32),
			acceleration: Vector3::new(0f32, 0f32, 0f32),
			collision_shape: body.shape(),
			inv_mass,
			center_of_mass: body.center_of_mass(),
			elasticity: body.elasticity(),
			inertia_tensor,
			handle,
			friction: body.friction(),
		});
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::factory::Factory;

	fn test_handle() -> Handle {
		let mut handle_factory = Factory::<()>::new();
		handle_factory.create(())
	}

	fn make_ground_body() -> PhysicsBody {
		PhysicsBody {
			body_type: BodyTypes::Static,
			collision_shape: Shapes::Cube {
				size: Vector3::new(4.0, 1.0, 4.0),
			},
			position: Vector3::new(0.0, 0.0, 0.0),
			orientation: Quaternion::identity(),
			acceleration: Vector3::zero(),
			linear_velocity: Vector3::zero(),
			angular_velocity: Vector3::zero(),
			inv_mass: 0.0,
			center_of_mass: Vector3::zero(),
			elasticity: 0.0,
			inertia_tensor: Matrix3::identity(),
			friction: 0.0,
			handle: test_handle(),
		}
	}

	fn make_sphere_body() -> PhysicsBody {
		PhysicsBody {
			body_type: BodyTypes::Dynamic,
			collision_shape: Shapes::Sphere { radius: 0.5 },
			position: Vector3::new(0.0, 1.4, 0.0),
			orientation: Quaternion::identity(),
			acceleration: Vector3::zero(),
			linear_velocity: Vector3::zero(),
			angular_velocity: Vector3::zero(),
			inv_mass: 1.0,
			center_of_mass: Vector3::zero(),
			elasticity: 0.0,
			inertia_tensor: Matrix3::identity(),
			friction: 0.0,
			handle: test_handle(),
		}
	}

	fn resolve_penetration_depth(mut bodies: Vec<PhysicsBody>) -> f32 {
		let body_factory = Factory::<EntityHandle<dyn Body>>::new();
		let listener = body_factory.listener();
		let mut world = World::new(listener);
		world.bodies = std::mem::take(&mut bodies);

		let contacts = detect_collisions_for_bodies(&world.bodies);
		assert_eq!(contacts.len(), 1);
		world.resolve_contact(&contacts[0]);

		intersect(&world.bodies[0], &world.bodies[1]).map_or(0.0, |intersection| intersection.depth)
	}

	#[test]
	fn detects_each_pair_once() {
		let contacts = detect_collisions_for_bodies(&[make_ground_body(), make_sphere_body()]);

		assert_eq!(contacts.len(), 1);
		assert_eq!((contacts[0].a.object, contacts[0].b.object), (0, 1));
	}

	#[test]
	fn resolves_sphere_ground_penetration_for_both_body_orders() {
		let depth_when_ground_first = resolve_penetration_depth(vec![make_ground_body(), make_sphere_body()]);
		let depth_when_sphere_first = resolve_penetration_depth(vec![make_sphere_body(), make_ground_body()]);

		assert!(depth_when_ground_first <= 1e-4);
		assert!(depth_when_sphere_first <= 1e-4);
	}
}
