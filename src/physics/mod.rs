use std::collections::HashMap;

use maths_rs::{Vec3f, mag};

use crate::orchestrator::{Entity, System, EntityReturn, EntitySubscriber, EntityHandle, Component, EntityHash, EventDescription, Event, EventImplementation,};

pub struct Sphere {
	position: Vec3f,
	velocity: Vec3f,
	radius: f32,
	events: Vec<Box<dyn Event<()>>>,
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

	pub fn subscribe_to_collision<T: Entity>(&mut self, handle: EntityHandle<T>, callback: fn(&mut T, &())) {
		self.events.push(Box::new(EventImplementation::new(handle, callback)));
	}
}

impl Entity for Sphere {}
impl Component for Sphere {}

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

	pub fn new_as_system() -> EntityReturn<'static, Self> {
		EntityReturn::new(Self::new()).add_listener::<Sphere>()
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
					println!("Collision!");
					collisions.push((i, j));
				}
			}
		}

		for (_, j) in collisions {
			self.spheres[j].handle.get(|e| {
				for event in &e.events {
					event.fire(&())
				}
			});
		}
	}
}

impl Entity for PhysicsWorld {}
impl System for PhysicsWorld {}

impl EntitySubscriber<Sphere> for PhysicsWorld {
	fn on_create(&mut self, orchestrator: crate::orchestrator::OrchestratorReference, handle: EntityHandle<Sphere>, params: &Sphere) {
		let index = self.add_sphere(InternalSphere{ position: params.position, velocity: params.velocity, radius: params.radius, handle: handle.clone() });
		self.spheres_map.insert(EntityHash::from(&handle), index);
	}

	fn on_update(&mut self, orchestrator: crate::orchestrator::OrchestratorReference, handle: EntityHandle<Sphere>, params: &Sphere) {
		
	}
}