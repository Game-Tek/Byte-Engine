use std::collections::HashMap;

use maths_rs::{Vec3f, mag};

use crate::{core::{entity::{EntityBuilder, EntityHash}, event::{Event, EventLike,}, listener::{EntitySubscriber, Listener}, orchestrator::{self,}, property::Property, Entity, EntityHandle}, utils};

pub trait PhysicsEntity: Entity {
	fn on_collision(&mut self) -> &mut Event<EntityHandle<dyn PhysicsEntity>>;

	fn get_position(&self) -> Vec3f;
	fn set_position(&mut self, position: Vec3f);
	
	fn get_velocity(&self) -> Vec3f;
}

pub struct Sphere {
	position: Vec3f,
	velocity: Vec3f,
	radius: f32,
	collision_event: Event<EntityHandle<dyn PhysicsEntity>>,
}

struct InternalSphere {
	position: Vec3f,
	velocity: Vec3f,
	radius: f32,
	handle: EntityHandle<dyn PhysicsEntity>,
}

impl Sphere {
	pub fn new(position: Vec3f, velocity: Vec3f, radius: f32) -> EntityBuilder<'static, Self> {
		Self {
			position,
			velocity,
			radius,
			collision_event: Event::default(),
		}.into()
	}
}

impl Entity for Sphere {
	fn call_listeners(&self, listener: &crate::core::listener::BasicListener, handle: EntityHandle<Self>) where Self: Sized {
		listener.invoke_for(handle.clone(), self);
		listener.invoke_for(handle.clone() as EntityHandle<dyn PhysicsEntity>, self as &dyn PhysicsEntity);
	}
}

impl PhysicsEntity for Sphere {
	fn on_collision(&mut self) -> &mut Event<EntityHandle<dyn PhysicsEntity>> { &mut self.collision_event }

	fn get_position(&self) -> Vec3f { self.position }
	fn get_velocity(&self) -> Vec3f { self.velocity }

	fn set_position(&mut self, position: Vec3f) { self.position = position; }
}

pub struct PhysicsWorld {
	spheres: Vec<InternalSphere>,
	spheres_map: HashMap<EntityHash, usize>,
}

impl PhysicsWorld {
	fn new() -> Self {
		Self {
			spheres: Vec::new(),
			spheres_map: HashMap::new(),
		}
	}

	pub fn new_as_system<'c>() -> EntityBuilder<'c, Self> {
		EntityBuilder::new(Self::new()).listen_to::<dyn PhysicsEntity>()
	}

	fn add_sphere(&mut self, sphere: InternalSphere) -> usize {
		let index = self.spheres.len();
		self.spheres.push(sphere);
		index
	}

	pub fn update(&mut self) {
		for sphere in self.spheres.iter_mut() {
			sphere.position += sphere.velocity;
			sphere.handle.write_sync().set_position(sphere.position);
		}

		let mut collisions = Vec::new();

		for (i, a) in self.spheres.iter().enumerate() {
			for (j, b) in self.spheres.iter().enumerate() {
				if i == j { continue; }

				if collisions.contains(&(j, i)) { continue; }

				if mag(a.position - b.position) < a.radius + b.radius {
					collisions.push((i, j));
				}
			}
		}

		for (i, j) in collisions {
			self.spheres[j].handle.map(|e| {
				let mut e = e.write_sync();
				e.on_collision().ocurred(&self.spheres[i].handle);
			});
		}
	}
}

impl Entity for PhysicsWorld {}

impl EntitySubscriber<dyn PhysicsEntity> for PhysicsWorld {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<dyn PhysicsEntity>, params: &'a dyn PhysicsEntity) -> utils::BoxedFuture<()> {
		Box::pin(async move {
			let index = self.add_sphere(InternalSphere{ position: params.get_position(), velocity: params.get_velocity(), radius: 0.1f32, handle: handle.clone() });
			self.spheres_map.insert(EntityHash::from(&handle), index);
		})
	}
}