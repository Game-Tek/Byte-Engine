use core::listener::BasicListener;
use std::{collections::HashMap, future::join};

use maths_rs::{Vec3f, mag};
use utils::BoxedFuture;

use crate::{application::Time, core::{entity::{EntityBuilder, EntityHash}, event::{Event, EventLike,}, listener::{EntitySubscriber, Listener}, orchestrator, property::Property, Entity, EntityHandle}, utils, Vector3};

pub trait PhysicsEntity: Entity {
	fn on_collision(&mut self) -> &mut Event<EntityHandle<dyn PhysicsEntity>>;

	fn get_body_type(&self) -> BodyTypes;

	fn get_position(&self) -> Vec3f;
	fn set_position(&mut self, position: Vec3f);
	
	fn get_velocity(&self) -> Vec3f;
}

/// The type of body that an entity has.
#[derive(Debug, Clone, Copy)]
pub enum BodyTypes {
	/// Static bodies are not affected by forces or collisions.
	Static,
	/// Kinematic bodies are not affected by forces, but are affected by collisions.
	Kinematic,
	/// Dynamic bodies are affected by forces and collisions.
	Dynamic,
}

pub struct Sphere {
	body_type: BodyTypes,
	position: Vec3f,
	velocity: Vec3f,
	radius: f32,
	collision_event: Event<EntityHandle<dyn PhysicsEntity>>,
}

pub enum CollisionShapes {
	Sphere {
		radius: f32,
	},
	Cube {
		size: Vec3f,
	},
}

struct Body {
	body_type: BodyTypes,
	collision_shape: CollisionShapes,
	position: Vec3f,
	acceleration: Vec3f,
	velocity: Vec3f,
	handle: EntityHandle<dyn PhysicsEntity>,
}

impl Sphere {
	pub fn new(body_type: BodyTypes, position: Vec3f, velocity: Vec3f, radius: f32) -> EntityBuilder<'static, Self> {
		Self {
			body_type,
			position,
			velocity,
			radius,
			collision_event: Event::default(),
		}.into()
	}
}

impl Entity for Sphere {
	fn call_listeners<'a>(&'a self, listener: &'a BasicListener, handle: EntityHandle<Self>) -> BoxedFuture<'a, ()> where Self: Sized { Box::pin(async move {
		let se = listener.invoke_for(handle.clone(), self);
		let pe = listener.invoke_for(handle.clone() as EntityHandle<dyn PhysicsEntity>, self as &dyn PhysicsEntity);
		join!(se, pe).await;
	}) }
}

impl PhysicsEntity for Sphere {
	fn on_collision(&mut self) -> &mut Event<EntityHandle<dyn PhysicsEntity>> { &mut self.collision_event }

	fn get_body_type(&self) -> BodyTypes { self.body_type }

	fn get_position(&self) -> Vec3f { self.position }
	fn get_velocity(&self) -> Vec3f { self.velocity }

	fn set_position(&mut self, position: Vec3f) { self.position = position; }
}

pub struct PhysicsWorld {
	bodies: Vec<Body>,
	spheres_map: HashMap<EntityHash, usize>,
	ongoing_collisions: Vec<(usize, usize)>,
}

impl PhysicsWorld {
	fn new() -> Self {
		Self {
			bodies: Vec::new(),
			spheres_map: HashMap::new(),
			ongoing_collisions: Vec::new(),
		}
	}

	pub fn new_as_system<'c>() -> EntityBuilder<'c, Self> {
		EntityBuilder::new(Self::new()).listen_to::<dyn PhysicsEntity>()
	}

	fn add_sphere(&mut self, sphere: Body) -> usize {
		let index = self.bodies.len();
		self.bodies.push(sphere);
		index
	}

	pub fn update(&mut self, time: Time) {
		let dt = time.delta();
		let dt = dt.as_secs_f32();

		for sphere in self.bodies.iter_mut() {
			match sphere.body_type {
				BodyTypes::Static => continue,
				BodyTypes::Kinematic => {
					sphere.position = sphere.handle.write_sync().get_position();
				},
				BodyTypes::Dynamic => {
					let forces = Vector3::new(0f32, -9.81f32, 0f32);
					sphere.acceleration = forces;
					sphere.velocity += sphere.acceleration * dt;
					sphere.position += sphere.velocity * dt;
					sphere.handle.write_sync().set_position(sphere.position);
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
					_ => unimplemented!(),
				}
			}
		}

		for &(i, j) in &collisions {
			if self.ongoing_collisions.contains(&(i, j)) { continue; }
			
			self.ongoing_collisions.push((i, j));

			self.bodies[j].handle.map(|e| {
				let mut e = e.write_sync();
				e.on_collision().ocurred(&self.bodies[i].handle);
			});
		}

		self.ongoing_collisions.retain(|(i, j)| {
			collisions.contains(&(*i, *j))
		});
	}
}

impl Entity for PhysicsWorld {}

impl EntitySubscriber<dyn PhysicsEntity> for PhysicsWorld {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<dyn PhysicsEntity>, params: &'a dyn PhysicsEntity) -> utils::BoxedFuture<()> {
		Box::pin(async move {
			let index = self.add_sphere(Body{ body_type: params.get_body_type(), position: params.get_position(), velocity: params.get_velocity(), acceleration: Vector3::new(0f32, 0f32, 0f32), collision_shape: CollisionShapes::Sphere { radius: 0.1f32 }, handle: handle.clone() });
			self.spheres_map.insert(EntityHash::from(&handle), index);
		})
	}
}