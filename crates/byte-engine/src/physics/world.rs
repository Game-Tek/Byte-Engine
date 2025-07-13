use math::{collision::{cube_vs_cube, sphere_vs_sphere}, cube::Cube, dot, magnitude, magnitude_squared, normalize, sphere::Sphere, Base, Vector3};

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

	pub fn apply_impulse(&mut self, entity: EntityHandle<dyn Body>, impulse: Vector3) {
		let entity = Some(entity);

		if let Some(body) = self.bodies.iter_mut().find(|e| &e.body == &entity) {
			body.apply_linear_impulse(impulse);
		}
	}

	pub fn update(&mut self, time: Time) {
		let dt = time.delta();
		let dt = dt.as_secs_f32();

		for body in self.bodies.iter_mut() {
			match body.body_type {
				BodyTypes::Dynamic => {
					let forces = Vector3::new(0f32, -9.81f32, 0f32);

					let mass = 1f32 / body.inv_mass;

					let impulse = forces * mass * dt;

					body.apply_linear_impulse(impulse);
				}
				_ => continue,
			}
		}

		let static_bodies = self.bodies.iter().enumerate().filter(|(_, b)| b.body_type == BodyTypes::Static);
		let dynamic_bodies = self.bodies.iter().enumerate().filter(|(_, b)| b.body_type == BodyTypes::Kinematic || b.body_type == BodyTypes::Dynamic);

		let mut collisions: Vec<Collision> = Vec::new();

		for (i, a) in static_bodies {
			for (j, b) in dynamic_bodies.clone() {
				if i == j { continue; }

				let pair = (i, j);

				if collisions.iter().find(|c| c.pair == pair).is_some() { continue; }

				let intersection = intersect(a, b);

				if let Some(intersection) = intersection {
					let normal = intersection.normal;

					let relative_velocity = b.velocity - a.velocity;
					let relative_velocity_along_normal = dot(relative_velocity, normal);

					let elasticity = a.elasticity * b.elasticity;
					let impulse = (1f32 + elasticity) * relative_velocity_along_normal / (a.inv_mass + b.inv_mass);
					let vector_impulse = normal * impulse;

					collisions.push(Collision {
						pair,
						normal: intersection.normal,
						depth: intersection.depth,
						impulses: (vector_impulse, -vector_impulse),
					});
				}
			}
		}

		for c in &collisions {
			let pair = c.pair;

			if !self.ongoing_collisions.contains(&pair) {
				self.ongoing_collisions.push(pair);
			}

			let a = &mut self.bodies[pair.0];
			a.apply_linear_impulse(c.impulses.0);

			let b = &mut self.bodies[pair.1];
			b.apply_linear_impulse(c.impulses.1);

			log::debug!("Collision between {:?} and {:?}", pair.0, pair.1);
		}

		for body in self.bodies.iter_mut() {
			match body.body_type {
				BodyTypes::Dynamic => {
					body.position += body.velocity * dt;
					body.collider.write().set_position(body.position);
				}
				_ => continue,
			}
		}

		self.ongoing_collisions.retain(|p| {
			collisions.iter().find(|c| c.pair == *p).is_some()
		});
	}
}

impl Entity for World {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self).listen_to::<CreateEvent<dyn Body>>().listen_to::<CreateEvent<dyn Collider>>()
	}
}

impl Listener<CreateEvent<dyn Body>> for World {
	fn handle(&mut self, event: &CreateEvent<dyn Body>) {
		let handle = event.handle();
		let body = handle.read();
		self.add_body(PhysicsBody{ body_type: body.get_body_type(), position: body.get_position(), velocity: body.get_velocity(), acceleration: Vector3::new(0f32, 0f32, 0f32), collision_shape: body.shape(), collider: handle.clone() as EntityHandle<dyn Collider>, body: Some(handle.clone()), inv_mass: 1f32 / body.get_mass(), elasticity: 0.5 });
	}
}

impl Listener<CreateEvent<dyn Collider>> for World {
	fn handle(&mut self, event: &CreateEvent<dyn Collider>) {
		let handle = event.handle();
		let collider = handle.read();
		self.add_body(PhysicsBody{ body_type: BodyTypes::Static, position: collider.get_position(), velocity: Vector3::zero(), acceleration: Vector3::zero(), collision_shape: collider.shape(), collider: handle.clone(), body: None, inv_mass: 0f32, elasticity: 0.5 });
	}
}

struct PhysicsBody {
	body_type: BodyTypes,
	collision_shape: CollisionShapes,
	position: Vector3,
	acceleration: Vector3,
	velocity: Vector3,
	/// Reciprocal mass of the body.
	inv_mass: f32,
	collider: EntityHandle<dyn Collider>,
	body: Option<EntityHandle<dyn Body>>,
	elasticity: f32,
}

impl PhysicsBody {
	fn apply_linear_impulse(&mut self, impulse: Vector3) {
		if self.inv_mass == 0f32 { return; }
		self.velocity += impulse * self.inv_mass;
	}
}

struct Intersection {
	normal: Vector3,
	depth: f32,
	point_on_a: Vector3,
	point_on_b: Vector3,
}

#[derive(Debug)]
struct Collision {
	pair: (usize, usize),
	normal: Vector3,
	depth: f32,
	impulses: (Vector3, Vector3),
}

fn intersect(a: &PhysicsBody, b: &PhysicsBody) -> Option<Intersection> {
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
