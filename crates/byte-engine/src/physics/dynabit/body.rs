use math::{collision::{cube_vs_cube, sphere_vs_cube, sphere_vs_sphere, Intersection}, cube::Cube, dot, magnitude, magnitude_squared, normalize, sphere::Sphere, Base, Vector3};
use crate::{application::Time, core::{entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle}, physics::{body::{Body, BodyTypes}, collider::{Collider, CollisionShapes}}};

pub struct PhysicsBody {
	pub(crate) body_type: BodyTypes,
	pub(crate) collision_shape: CollisionShapes,
	pub(crate) position: Vector3,
	pub(crate) acceleration: Vector3,
	pub(crate) velocity: Vector3,
	/// Reciprocal mass of the body.
	pub(crate) inv_mass: f32,
	pub(crate) collider: EntityHandle<dyn Collider>,
	pub(crate) body: Option<EntityHandle<dyn Body>>,
	pub(crate) elasticity: f32,
}

impl PhysicsBody {
	pub fn apply_linear_impulse(&mut self, impulse: Vector3) {
		if self.inv_mass == 0f32 { return; }
		self.velocity += impulse * self.inv_mass;
	}
}

pub fn intersect(a: &PhysicsBody, b: &PhysicsBody) -> Option<Intersection> {
	match (a.collision_shape, b.collision_shape) {
		(CollisionShapes::Sphere { radius: ra }, CollisionShapes::Sphere { radius: rb }) => {
			sphere_vs_sphere(&Sphere{ center: a.position, radius: ra }, &Sphere{ center: b.position, radius: rb })
		},
		(CollisionShapes::Cube { size: sa }, CollisionShapes::Cube { size: sb }) => {
			cube_vs_cube(&Cube::new(a.position, sa), &Cube::new(b.position, sb))
		},
		(CollisionShapes::Sphere { radius: ra }, CollisionShapes::Cube { size: sb }) => {
			sphere_vs_cube(&Sphere::new(a.position, ra), &Cube::new(b.position, sb))
		},
		(CollisionShapes::Cube { size: sa }, CollisionShapes::Sphere { radius: rb }) => {
			sphere_vs_cube(&Sphere::new(b.position, rb), &Cube::new(a.position, sa))
		},
	}
}
