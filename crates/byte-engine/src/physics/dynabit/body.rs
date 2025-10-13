use math::{collision::{cube_vs_cube, sphere_vs_cube, sphere_vs_sphere, Intersection}, cross, cube::Cube, dot, length, magnitude, magnitude_squared, mat::{MatInverse as _, MatTranspose as _}, normalize, sphere::Sphere, Base, Magnitude as _, Matrix3, Quaternion, Vector3};
use crate::{application::Time, core::{entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle}, physics::{body::{Body, BodyTypes}, collider::{Collider, Shapes}}};

pub struct PhysicsBody {
	pub(crate) body_type: BodyTypes,
	pub(crate) collision_shape: Shapes,
	pub(crate) position: Vector3,
	pub(crate) orientation: Quaternion,
	pub(crate) acceleration: Vector3,
	pub(crate) linear_velocity: Vector3,
	pub(crate) angular_velocity: Vector3,
	/// Reciprocal mass of the body.
	pub(crate) inv_mass: f32,
	pub(crate) center_of_mass: Vector3,
	pub(crate) collider: EntityHandle<dyn Collider>,
	pub(crate) body: Option<EntityHandle<dyn Body>>,
	pub(crate) elasticity: f32,
	pub(crate) inertia_tensor: Matrix3,
	pub(crate) friction: f32,
}

impl PhysicsBody {
	pub fn apply_impulse(&mut self, point: Vector3, impulse: Vector3) {
		if self.inv_mass == 0f32 { return; }
		self.apply_linear_impulse(impulse);
		let world_space_center_of_mass = self.world_space_center_of_mass();
		let r = point - world_space_center_of_mass;
		let dl = cross(r, impulse);
		self.apply_angular_impulse(dl);
	}

	pub fn apply_linear_impulse(&mut self, impulse: Vector3) {
		if self.inv_mass == 0f32 { return; }
		self.linear_velocity += impulse * self.inv_mass;
	}

	pub fn apply_angular_impulse(&mut self, impulse: Vector3) {
		if self.inv_mass == 0f32 { return; }
		self.angular_velocity += self.inverse_world_space_inertia_tensor() * impulse;
	}

	pub fn world_space_center_of_mass(&self) -> Vector3 {
		self.position + self.orientation.get_matrix() * self.center_of_mass
	}

	pub fn inverse_world_space_inertia_tensor(&self) -> Matrix3 {
		let inertia_tensor = self.inertia_tensor;
		let inverse = inertia_tensor.inverse();
		let orientation = self.orientation.get_matrix();
		orientation * inverse * orientation.transpose()
	}

	pub fn update(&mut self, time: Time) {
		let dt = time.delta();
		let dt = dt.as_secs_f32();
		self.position += self.linear_velocity * dt;

		let world_space_center_of_mass = self.world_space_center_of_mass();
		let cp = self.position - world_space_center_of_mass;

		let orientation = self.orientation.get_matrix();
		let inertia_tensor = orientation * self.inertia_tensor * orientation.transpose();
		let alpha = inertia_tensor.inverse() * (cross(self.angular_velocity, inertia_tensor * self.angular_velocity));
		self.angular_velocity += alpha * dt;

		let d = self.angular_velocity * dt;
		let dq = Quaternion::from_axis_angle(d, length(d));

		self.orientation = Quaternion::normalize(self.orientation * dq);

		self.position = world_space_center_of_mass + dq * cp;
	}
}

pub fn intersect(a: &PhysicsBody, b: &PhysicsBody) -> Option<Intersection> {
	match (a.collision_shape, b.collision_shape) {
		(Shapes::Sphere { radius: ra }, Shapes::Sphere { radius: rb }) => {
			sphere_vs_sphere(&Sphere{ center: a.position, radius: ra }, &Sphere{ center: b.position, radius: rb })
		},
		(Shapes::Cube { size: sa }, Shapes::Cube { size: sb }) => {
			cube_vs_cube(&Cube::new(a.position, sa), &Cube::new(b.position, sb))
		},
		(Shapes::Sphere { radius: ra }, Shapes::Cube { size: sb }) => {
			sphere_vs_cube(&Sphere::new(a.position, ra), &Cube::new(b.position, sb))
		},
		(Shapes::Cube { size: sa }, Shapes::Sphere { radius: rb }) => {
			sphere_vs_cube(&Sphere::new(b.position, rb), &Cube::new(a.position, sa)).map(|e| e.flip())
		},
	}
}
