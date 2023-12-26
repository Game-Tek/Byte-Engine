use maths_rs::Vec3f;

use crate::orchestrator::{Entity, System, EntityReturn};

pub struct Sphere {
	position: Vec3f,
	velocity: Vec3f,
	radius: f32,
}

impl Sphere {
	pub fn new(position: Vec3f, velocity: Vec3f, radius: f32) -> Self {
		Self {
			position,
			velocity,
			radius,
		}
	}
}

pub struct PhysicsWorld {
	spheres: Vec<Sphere>,
}

impl PhysicsWorld {
	fn new() -> Self {
		Self {
			spheres: Vec::new(),
		}
	}

	pub fn new_as_system() -> EntityReturn<'static, Self> {
		EntityReturn::new(Self::new())
	}

	pub fn add_sphere(&mut self, sphere: Sphere) {
		self.spheres.push(sphere);
	}

	pub fn update(&mut self) {
		for sphere in self.spheres.iter_mut() {
			sphere.position += sphere.velocity;

			log::info!("Sphere position: {:?}", sphere.position);
		}
	}
}

impl Entity for PhysicsWorld {}
impl System for PhysicsWorld {}