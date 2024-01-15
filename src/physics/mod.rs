use std::collections::HashMap;

use maths_rs::{Vec3f, mag};

use crate::core::{orchestrator::{self, EntityReturn, EntitySubscriber, EventDescription,}, Entity, EntityHandle, entity::EntityHash, event::{Event, EventImplementation}, listener::Listener};


pub struct Sphere {
	position: Vec3f,
	velocity: Vec3f,
	radius: f32,
	events: Vec<Box<dyn Event<EntityHandle<Sphere>>>>,
}

struct InternalSphere {
	position: Vec3f,
	velocity: Vec3f,
	radius: f32,
	handle: EntityHandle<Sphere>,
}

impl Sphere {
	pub fn new(position: Vec3f, velocity: Vec3f, radius: f32) -> Self {
		Self {
			position,
			velocity,
			radius,

			events: Vec::new(),
		}
	}

	pub const fn on_collision() -> EventDescription<Self, ()> { EventDescription::new() }

	pub fn subscribe_to_collision<T: Entity>(&mut self, handle: EntityHandle<T>, callback: fn(&mut T, &EntityHandle<Sphere>)) {
		self.events.push(Box::new(EventImplementation::new(handle, callback)));
	}
}

impl Entity for Sphere {}

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

	pub fn new_as_system<'c>(listener: &'c mut impl Listener) -> EntityReturn<'c, Self> {
		EntityReturn::new(Self::new()).listen_to::<Sphere>(listener)
	}

	fn add_sphere(&mut self, sphere: InternalSphere) -> usize {
		let index = self.spheres.len();
		self.spheres.push(sphere);
		index
	}

	pub fn update(&mut self) {
		for sphere in self.spheres.iter_mut() {
			sphere.position += sphere.velocity;
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
				let e = e.read_sync();
				for event in &e.events {
					event.fire(&self.spheres[i].handle)
				}
			});
		}
	}
}

impl Entity for PhysicsWorld {}

impl EntitySubscriber<Sphere> for PhysicsWorld {
	async fn on_create<'a>(&'a mut self, handle: EntityHandle<Sphere>, params: &Sphere) {
		let index = self.add_sphere(InternalSphere{ position: params.position, velocity: params.velocity, radius: params.radius, handle: handle.clone() });
		self.spheres_map.insert(EntityHash::from(&handle), index);
	}

	async fn on_update(&'static mut self, handle: EntityHandle<Sphere>, params: &Sphere) {
		
	}
}