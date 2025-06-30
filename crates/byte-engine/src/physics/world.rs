use math::Vector3;
use maths_rs::mag;

use crate::{application::Time, core::{entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle}, physics::{body::{Body, BodyTypes}, collider::{Collider, CollisionShapes}}};

pub struct World {
	bodies: Vec<PhysicsBody>,
	ongoing_collisions: Vec<(usize, usize)>,
}

impl World {
	pub fn new() -> Self {
		Self {
			bodies: Vec::new(),
			ongoing_collisions: Vec::new(),
		}
	}

	fn add_body(&mut self, body: PhysicsBody) -> usize {
		let index = self.bodies.len();
		self.bodies.push(body);
		index
	}

	pub fn update(&mut self, time: Time) {
		let dt = time.delta();
		let dt = dt.as_secs_f32();

		for body in self.bodies.iter_mut() {
			match body.body_type {
				BodyTypes::Static => continue,
				BodyTypes::Kinematic => {
					body.position = body.handle.write().get_position();
				},
				BodyTypes::Dynamic => {
					let forces = Vector3::new(0f32, -9.81f32, 0f32);
					body.acceleration = forces;
					body.velocity += body.acceleration * dt;
					body.position += body.velocity * dt;
					body.handle.write().set_position(body.position);
				}
			}
		}

		let mut collisions = Vec::new();

		for (i, a) in self.bodies.iter().enumerate() {
			for (j, b) in self.bodies.iter().enumerate() {
				if i == j { continue; }

				if collisions.contains(&(j, i)) { continue; }

				match (&a.collision_shape, &b.collision_shape) {
					(CollisionShapes::Sphere { radius: ra }, CollisionShapes::Sphere { radius: rb }) => {
						if mag(a.position - b.position) < ra + rb {
							collisions.push((i, j));
						}
					},
					(&CollisionShapes::Cube { size: sa }, &CollisionShapes::Cube { size: sb }) => {
						let a_min = a.position - sa / 2f32;
						let a_max = a.position + sa / 2f32;
						let b_min = b.position - sb / 2f32;
						let b_max = b.position + sb / 2f32;

						if a_min.x < b_max.x && a_max.x > b_min.x &&
							a_min.y < b_max.y && a_max.y > b_min.y &&
							a_min.z < b_max.z && a_max.z > b_min.z {
							collisions.push((i, j));
						}
					},
					// Calculate collision between a sphere and a cube (cube size is half-size)
					(&CollisionShapes::Sphere { radius: ra }, &CollisionShapes::Cube { size: sb }) | (&CollisionShapes::Cube { size: sb }, &CollisionShapes::Sphere { radius: ra }) => {
						let a_min = a.position - Vector3::new(ra, ra, ra);
						let a_max = a.position + Vector3::new(ra, ra, ra);
						let b_min = b.position - sb / 2f32;
						let b_max = b.position + sb / 2f32;

						if a_min.x < b_max.x && a_max.x > b_min.x &&
							a_min.y < b_max.y && a_max.y > b_min.y &&
							a_min.z < b_max.z && a_max.z > b_min.z {
							collisions.push((i, j));
						}
					},
				}
			}
		}

		for &(i, j) in &collisions {
			if self.ongoing_collisions.contains(&(i, j)) { continue; }

			self.ongoing_collisions.push((i, j));

			self.bodies[j].handle.map(|e| {
				let mut e = e.write();
				if let Some(collision_event) = e.on_collision() {
					// collision_event.ocurred(&self.bodies[i].handle);
				}
			});

			log::info!("Collision between {:?} and {:?}", i, j);
		}

		self.ongoing_collisions.retain(|(i, j)| {
			collisions.contains(&(*i, *j))
		});
	}
}

impl Entity for World {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self).listen_to::<CreateEvent<dyn Body>>()
	}
}

impl Listener<CreateEvent<dyn Body>> for World {
	fn handle(&mut self, event: &CreateEvent<dyn Body>) {
		let handle = event.handle();
		let body = handle.read();
		self.add_body(PhysicsBody{ body_type: body.get_body_type(), position: body.get_position(), velocity: body.get_velocity(), acceleration: Vector3::new(0f32, 0f32, 0f32), collision_shape: body.shape(), handle: handle.clone() });
	}
}

impl Listener<CreateEvent<dyn Collider>> for World {
	fn handle(&mut self, event: &CreateEvent<dyn Collider>) {
		let handle = event.handle();
		let collider = handle.read();
		todo!();
	}
}

struct PhysicsBody {
	body_type: BodyTypes,
	collision_shape: CollisionShapes,
	position: Vector3,
	acceleration: Vector3,
	velocity: Vector3,
	handle: EntityHandle<dyn Body>,
}
