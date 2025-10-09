use math::{collision::{cube_vs_cube, sphere_vs_sphere, Intersection}, cube::Cube, dot, magnitude, magnitude_squared, normalize, sphere::Sphere, Base, Vector3};
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
			sphere_vs_sphere(&Sphere{ center: a.position, radius: ra }, &Sphere{ center: b.position, radius: rb }).map(|i| {
				Intersection{ normal: i.normal, depth: i.depth, point_on_a: i.point_on_a, point_on_b: i.point_on_b }
			})
		},
		(CollisionShapes::Cube { size: sa }, CollisionShapes::Cube { size: sb }) => {
			cube_vs_cube(&Cube::new(a.position, sa), &Cube::new(b.position, sb)).map(|i| {
				Intersection{ normal: i.normal, depth: i.depth, point_on_a: i.point_on_a, point_on_b: i.point_on_b }
			})
		},
		(CollisionShapes::Sphere { radius: ra }, CollisionShapes::Cube { size: sb }) | (CollisionShapes::Cube { size: sb }, CollisionShapes::Sphere { radius: ra }) => {
			let a_min = a.position - Vector3::new(ra, ra, ra);
			let a_max = a.position + Vector3::new(ra, ra, ra);
			let b_min = b.position - sb / 2f32;
			let b_max = b.position + sb / 2f32;

			if a_min.x < b_max.x && a_max.x > b_min.x && a_min.y < b_max.y && a_max.y > b_min.y && a_min.z < b_max.z && a_max.z > b_min.z {
				Some(Intersection{ normal: normalize(b.position - a.position), depth: ra, point_on_a: Vector3::zero(), point_on_b: Vector3::zero() })
			} else {
				None
			}
		},
	}
}
