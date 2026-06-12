//! Built-in rigid-body world implementation.
//!
//! This world consumes body creation and deletion messages from
//! [`crate::gameplay::world::DefaultWorld`] and publishes resulting transform
//! updates back to gameplay. Applications normally use it through that world
//! rather than constructing it independently.

#[derive(Clone)]
/// The [`World`] struct owns Dynabit simulation state and synchronizes it with
/// engine entity handles.
pub struct World {
	bodies: StableVec<PhysicsBody>,
	gravity: Vector3,

	body_listener: DefaultListener<CreateMessage<EntityHandle<dyn Body>>>,
	body_delete_listener: DefaultListener<DeleteMessage>,

	handles_to_bodies: HashMap<Handle, usize>,
}

impl World {
	pub fn new(
		body_listener: DefaultListener<CreateMessage<EntityHandle<dyn Body>>>,
		body_delete_listener: DefaultListener<DeleteMessage>,
	) -> Self {
		Self {
			bodies: StableVec::new(),
			gravity: Vector3::new(0f32, -16f32, 0f32),
			body_listener,
			body_delete_listener,

			handles_to_bodies: HashMap::with_capacity(1024),
		}
	}

	fn add_body(&mut self, body: PhysicsBody) -> usize {
		self.bodies.push(body)
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
			let handle = *message.handle();
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

		let dt = time.delta();

		self.update_velocities(dt);

		let dt = dt - self.update_collisions(dt);
		self.update_bodies(dt, transforms_tx);
	}

	/// Applies all initial impulses to bodies based on forces.
	pub fn update_velocities(&mut self, dt: Duration) {
		let dt = dt.as_secs_f32();

		for body in self.bodies.iter_mut() {
			match body.body_type {
				BodyTypes::Dynamic => {
					let forces = self.gravity;

					let mass = body.mass();

					let impulse = forces * mass * dt;

					body.apply_linear_impulse(impulse);
				}
				_ => continue,
			}
		}
	}

	/// Calculates and solves collisions.
	pub fn update_collisions(&mut self, dt: Duration) -> Duration {
		let use_broadphase = true;

		let mut contacts = if use_broadphase {
			let broadphase = broadphase(self.bodies.indexed_iter(), dt.as_secs_f32());
			self.detect_collisions_from_pairs(&broadphase, dt.as_secs_f32())
		} else {
			self.detect_collisions(dt)
		};

		contacts.sort(); // Sort contacts by time of impact

		let mut accumulated_time = Duration::ZERO;

		for contact in &contacts {
			let contact_time = Duration::from_secs_f32(contact.toi.max(0.0));
			let dt = contact_time.saturating_sub(accumulated_time);

			// Contacts from dynamic detection are expressed at their time of impact, so
			// bodies must be advanced before impulses are applied at those points.
			for body in self.bodies.iter_mut() {
				body.update(dt);
			}

			self.resolve_contact(contact);

			accumulated_time += dt;
		}

		accumulated_time
	}

	/// Brute-force collision detection for all bodies in the world.
	fn detect_collisions(&self, dt: Duration) -> Vec<Contact> {
		detect_collisions_for_bodies(&self.bodies, dt.as_secs_f32())
	}

	/// Collision detection for a subset of body pairs.
	fn detect_collisions_from_pairs(&self, pairs: &[Pair], dt: f32) -> Vec<Contact> {
		let pairs = pairs.iter().map(|p| ((p.a, &self.bodies[p.a]), (p.b, &self.bodies[p.b])));
		detect_collisions_for_body_pairs(pairs, dt)
	}

	/// Updates bodies' positions and orientation based on their velocities.
	pub fn update_bodies(&mut self, dt: Duration, transforms_tx: &mut impl Channel<TransformationUpdate>) {
		for body in self.bodies.iter_mut() {
			match body.body_type {
				BodyTypes::Dynamic => {
					body.update(dt);

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

		let a = self.bodies.get(a_index).cloned().unwrap();
		let b = self.bodies.get(b_index).cloned().unwrap();

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

		if let Some(a) = self.bodies.get_mut(a_index) {
			a.apply_impulse(a_point, -impulse_vector);
		}

		if let Some(b) = self.bodies.get_mut(b_index) {
			b.apply_impulse(b_point, impulse_vector);
		}

		let vel_normal = n * dot(vab, n);
		let vel_tangent = vab - vel_normal;

		let relative_vel_tangent = normalize(vel_tangent);

		let a_inertia = cross(a_inv_world_inertia * cross(ra, relative_vel_tangent), ra);
		let b_inertia = cross(b_inv_world_inertia * cross(rb, relative_vel_tangent), rb);
		let inv_inertia = dot(a_inertia + b_inertia, relative_vel_tangent);

		let reduced_mass = 1.0 / (a_inv_mass + b_inv_mass + inv_inertia);
		let impulse_friction = vel_tangent * reduced_mass * friction;

		if let Some(a) = self.bodies.get_mut(a_index) {
			a.apply_impulse(a_point, -impulse_friction);
		}

		if let Some(b) = self.bodies.get_mut(b_index) {
			b.apply_impulse(b_point, impulse_friction);
		}

		if contact.toi == 0f32 {
			let separation = n * contact.depth;

			//let separation = b_point - a_point; // Book suggests this way but it causes orbiting around the world center

			let t_a = a_inv_mass / inv_mass_sum;
			let t_b = b_inv_mass / inv_mass_sum;

			if let Some(a) = self.bodies.get_mut(a_index) {
				a.position -= separation * t_a;
			}

			if let Some(b) = self.bodies.get_mut(b_index) {
				b.position += separation * t_b;
			}
		}
	}

	fn create_body(&mut self, handle: Handle, body: &dyn Body) {
		let body_type = body.body_type();

		let inv_mass = match body_type {
			BodyTypes::Dynamic => 1f32 / body.mass(),
			_ => 0f32,
		};

		let index = self.add_body(PhysicsBody {
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
			handle,
			friction: body.friction(),
		});
		self.handles_to_bodies.insert(handle, index);
	}

	pub fn process_pending_deletions(&mut self) {
		while let Some(message) = self.body_delete_listener.read() {
			self.remove_body(message.into_handle());
		}
	}

	pub fn remove_body(&mut self, handle: Handle) -> Option<PhysicsBody> {
		let index = self.handles_to_bodies.remove(&handle)?;
		self.bodies.remove(index)
	}
}

/// Detects intersections and builds contact data for each unique body pair.
fn detect_collisions_for_bodies(bodies: &StableVec<PhysicsBody>, dt: f32) -> Vec<Contact> {
	let iter = bodies.iter().enumerate().flat_map(|(i, a)| {
		bodies
			.iter()
			.enumerate()
			.filter(move |(j, _)| *j > i)
			.map(move |(j, b)| ((i, a), (j, b)))
	});

	detect_collisions_for_body_pairs(iter, dt)
}

/// Detects intersections and builds contact data for each unique body pair.
fn detect_collisions_for_body_pairs<'a>(
	pairs: impl Iterator<Item = ((usize, &'a PhysicsBody), (usize, &'a PhysicsBody))>,
	dt: f32,
) -> Vec<Contact> {
	let mut contacts = Vec::with_capacity((pairs.size_hint().0 as f32 * 16f32).sqrt() as usize); // Arbitrary heuristic

	pairs
		.filter_map(|((i, a), (j, b))| intersect((a, i), (b, j), dt))
		.collect_into(&mut contacts);

	contacts
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::channel::DefaultChannel;
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
			friction: 0.0,
			handle: test_handle(),
		}
	}

	fn make_sphere_body() -> PhysicsBody {
		make_dynamic_sphere_body(Vector3::new(0.0, 1.4, 0.0), Vector3::zero(), 0.5)
	}

	fn make_dynamic_sphere_body(position: Vector3, linear_velocity: Vector3, radius: f32) -> PhysicsBody {
		PhysicsBody {
			body_type: BodyTypes::Dynamic,
			collision_shape: Shapes::Sphere { radius },
			position,
			orientation: Quaternion::identity(),
			acceleration: Vector3::zero(),
			linear_velocity,
			angular_velocity: Vector3::zero(),
			inv_mass: 1.0,
			center_of_mass: Vector3::zero(),
			elasticity: 0.0,
			friction: 0.0,
			handle: test_handle(),
		}
	}

	fn resolve_penetration_depth(mut bodies: Vec<PhysicsBody>, dt: f32) -> f32 {
		let body_factory = Factory::<EntityHandle<dyn Body>>::new();
		let listener = body_factory.listener();
		let delete_channel = DefaultChannel::new();
		let delete_listener = delete_channel.listener();
		let mut world = World::new(listener, delete_listener);
		world.bodies = std::mem::take(&mut bodies).into_iter().collect();

		let contacts = detect_collisions_for_bodies(&world.bodies, dt);
		assert_eq!(contacts.len(), 1);
		world.resolve_contact(&contacts[0]);

		intersect((&world.bodies[0], 0), (&world.bodies[1], 1), dt).map_or(0.0, |intersection| intersection.depth)
	}

	#[test]
	fn detects_each_pair_once() {
		let bodies = [make_ground_body(), make_sphere_body()].into_iter().collect();
		let contacts = detect_collisions_for_bodies(&bodies, 1.0);

		assert_eq!(contacts.len(), 1);
		assert_eq!((contacts[0].a.object, contacts[0].b.object), (0, 1));
	}

	#[test]
	fn resolves_sphere_ground_penetration_for_both_body_orders() {
		let depth_when_ground_first = resolve_penetration_depth(vec![make_ground_body(), make_sphere_body()], 1.0);
		let depth_when_sphere_first = resolve_penetration_depth(vec![make_sphere_body(), make_ground_body()], 1.0);

		assert!(depth_when_ground_first <= 1e-4);
		assert!(depth_when_sphere_first <= 1e-4);
	}

	#[test]
	fn resolves_overlapping_spheres_without_deepening_penetration() {
		let body_factory = Factory::<EntityHandle<dyn Body>>::new();
		let listener = body_factory.listener();
		let delete_channel = DefaultChannel::new();
		let delete_listener = delete_channel.listener();
		let mut world = World::new(listener, delete_listener);
		world.bodies = vec![
			make_dynamic_sphere_body(Vector3::new(0.0, 0.0, 0.0), Vector3::new(-1.0, 0.0, 0.0), 1.0),
			make_dynamic_sphere_body(Vector3::new(1.5, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0), 1.0),
		]
		.into_iter()
		.collect();

		let contacts = detect_collisions_for_bodies(&world.bodies, 1.0);
		assert_eq!(contacts.len(), 1);
		world.resolve_contact(&contacts[0]);

		let separation = length(world.bodies[1].position - world.bodies[0].position);
		assert!(separation >= 2.0 - 1e-4);
	}
}

use std::{ops::Deref, time::Duration};

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
use utils::{
	hash::{HashMap, HashMapExt},
	StableVec,
};

use crate::{
	application::Time,
	core::{
		channel::Channel,
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
		Entity, EntityHandle,
	},
	gameplay::transform::{Transform, TransformationUpdate},
	physics::{
		body::{Body, BodyTypes},
		collider::{Collider, Shapes},
		dynabit::{
			body::{intersect, PhysicsBody},
			contact::{Contact, Pair, Side},
		},
		intersection::broadphase,
	},
};
