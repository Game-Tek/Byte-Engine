use math::{collision::{cube_vs_cube, sphere_vs_sphere}, cube::Cube, dot, magnitude, magnitude_squared, normalize, sphere::Sphere, Base, Vector3};
use crate::{application::Time, core::{entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle}, physics::{body::{Body, BodyTypes}, collider::{Collider, CollisionShapes}, collision::Collision, dynabit::body::{intersect, PhysicsBody}}};

pub struct World {
	bodies: Vec<PhysicsBody>,
}

impl World {
	pub fn new() -> Self {
		Self {
			bodies: Vec::new(),
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
		self.update_velocities(time);
		self.update_collisions(time);
		self.update_positions(time);
	}

	pub fn update_velocities(&mut self, time: Time) {
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
	}

	pub fn update_collisions(&mut self, time: Time) {
		let bodies = self.bodies.iter().enumerate();

		let mut collisions: Vec<Collision> = Vec::new();

		for (i, a) in bodies.clone() {
			for (j, b) in bodies.clone() {
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

					let collision = Collision {
						pair,
						normal: intersection.normal,
						depth: intersection.depth,
						impulses: (vector_impulse, -vector_impulse),
					};

					collisions.push(collision);
				}
			}
		}

		for c in &collisions {
			let pair = c.pair;

			let a = &mut self.bodies[pair.0];
			a.apply_linear_impulse(c.impulses.0);

			let b = &mut self.bodies[pair.1];
			b.apply_linear_impulse(c.impulses.1);
		}
	}

	pub fn update_positions(&mut self, time: Time) {
		let dt = time.delta();
		let dt = dt.as_secs_f32();

		for body in self.bodies.iter_mut() {
			match body.body_type {
				BodyTypes::Dynamic => {
					body.position += body.velocity * dt;
					body.collider.write().set_position(body.position);
				}
				_ => continue,
			}
		}
	}
}

impl Entity for World {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self).listen_to::<CreateEvent<dyn Body>>().listen_to::<CreateEvent<dyn Collider>>()
	}
}

impl crate::physics::World for World {
	fn update(&mut self, time: Time) {
    	self.update(time);
	}
}

impl Listener<CreateEvent<dyn Body>> for World {
	fn handle(&mut self, event: &CreateEvent<dyn Body>) {
		let handle = event.handle();
		let body = handle.read();
		self.add_body(PhysicsBody{ body_type: body.body_type(), position: body.position(), velocity: body.velocity(), acceleration: Vector3::new(0f32, 0f32, 0f32), collision_shape: body.shape(), collider: handle.clone() as EntityHandle<dyn Collider>, body: Some(handle.clone()), inv_mass: 1f32 / body.mass(), elasticity: body.elasticity() });
	}
}

impl Listener<CreateEvent<dyn Collider>> for World {
	fn handle(&mut self, event: &CreateEvent<dyn Collider>) {
		let handle = event.handle();
		let collider = handle.read();
		self.add_body(PhysicsBody{ body_type: BodyTypes::Static, position: collider.position(), velocity: Vector3::zero(), acceleration: Vector3::zero(), collision_shape: collider.shape(), collider: handle.clone(), body: None, inv_mass: 0f32, elasticity: collider.elasticity() });
	}
}
