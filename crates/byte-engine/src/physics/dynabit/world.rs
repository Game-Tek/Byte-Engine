use std::alloc::Allocator;

use math::{Scale, Vector};
use utils::{
	StableVec, StableVecHandle,
	hash::{HashMap, HashMapExt},
};

use crate::{
	application::Time,
	core::{
		channel::Channel,
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
	},
	gameplay::transform::{Transform, TransformationUpdate},
	physics::{
		body::{Body, BodyTypes},
		dynabit::{
			body::{PhysicsBody, intersect},
			contact::{Contact, Pair},
		},
		intersection::broadphase,
	},
	time::MediaTime,
};

/// The `World` struct owns Dynabit simulation state and synchronizes it with entity handles.
#[derive(Clone)]
pub struct World {
	bodies: StableVec<PhysicsBody>,
	gravity: Vector,
	body_listener: DefaultListener<CreateMessage<Body>>,
	body_delete_listener: DefaultListener<DeleteMessage>,
	handles_to_bodies: HashMap<Handle, StableVecHandle>,
}

impl World {
	/// Creates a Dynabit world connected to body creation and deletion channels.
	pub fn new(
		body_listener: DefaultListener<CreateMessage<Body>>,
		body_delete_listener: DefaultListener<DeleteMessage>,
	) -> Self {
		Self {
			bodies: StableVec::new(),
			gravity: Vector::new(0.0, -16.0, 0.0),
			body_listener,
			body_delete_listener,
			handles_to_bodies: HashMap::with_capacity(1024),
		}
	}

	/// Applies a world-space linear impulse to the simulated body identified by `handle`.
	///
	/// Returns `false` when `handle` is not registered with this world, such as before
	/// creation is processed or after deletion. Use the lifecycle handle from the body's
	/// creation message; [`Self::update`] registers pending creations.
	pub fn apply_impulse(&mut self, handle: Handle, impulse: Vector) -> bool {
		debug_assert!(
			impulse.x().is_finite() && impulse.y().is_finite() && impulse.z().is_finite(),
			"Physics impulse is invalid. The most likely cause is non-finite force or mass data."
		);
		let Some(index) = self.handles_to_bodies.get(&handle).copied() else {
			return false;
		};
		let Some(body) = self.bodies.get_mut(index) else {
			return false;
		};

		body.apply_linear_impulse(impulse);
		true
	}

	/// Processes entity updates, advances physics, and publishes resulting transforms.
	pub fn update(
		&mut self,
		time: Time,
		transforms_rx: &mut impl Listener<TransformationUpdate>,
		transforms_tx: &mut impl Channel<TransformationUpdate>,
		allocator: &mut bumpalo::Bump,
	) {
		while let Some(message) = self.body_listener.read() {
			let handle = *message.handle();
			let body = message.into_data();
			self.create_body(handle, body);
		}
		while let Some(message) = transforms_rx.read() {
			if let Some(index) = self.handles_to_bodies.get(message.handle()).copied() {
				let body = &mut self.bodies[index];
				body.position = message.transform().get_position();
				body.orientation = message.transform().get_orientation();
				body.scale = message.transform().scale();
			}
		}

		let step = time.delta();
		self.update_velocities(step);
		let remaining = step - self.update_collisions(step, allocator);
		self.update_bodies(remaining, transforms_tx);
	}

	/// Applies gravity-derived impulses to dynamic bodies.
	pub fn update_velocities(&mut self, dt: MediaTime) {
		let seconds = dt.as_seconds_f32();
		for body in self.bodies.iter_mut().filter(|body| body.body_type == BodyTypes::Dynamic) {
			body.apply_linear_impulse(self.gravity * body.mass() * seconds);
		}
	}

	/// Detects and resolves contacts for this time step.
	pub fn update_collisions(&mut self, dt: MediaTime, allocator: &mut bumpalo::Bump) -> MediaTime {
		let mut contacts = bumpalo::collections::Vec::with_capacity_in(64, allocator);
		let pairs = broadphase(self.bodies.indexed_iter(), dt.as_seconds_f32());
		contacts.extend(self.detect_collisions_from_pairs(&pairs, dt.as_seconds_f32()));
		contacts.sort();

		let mut accumulated = MediaTime::ZERO;
		for contact in &contacts {
			let contact_time = MediaTime::from_seconds_f32(contact.toi.max(0.0));
			let advance = contact_time.saturating_sub(accumulated);
			for body in self.bodies.iter_mut() {
				body.update(advance);
			}
			self.resolve_contact(contact);
			accumulated += advance;
		}
		accumulated
	}

	/// Advances dynamic bodies and publishes their transforms.
	pub fn update_bodies(&mut self, dt: MediaTime, transforms_tx: &mut impl Channel<TransformationUpdate>) {
		for body in self.bodies.iter_mut().filter(|body| body.body_type == BodyTypes::Dynamic) {
			body.update(dt);
			transforms_tx.send(TransformationUpdate::new(
				body.handle,
				Transform::new(body.position, body.scale, body.orientation),
			));
		}
	}

	fn detect_collisions_from_pairs<'a>(&'a self, pairs: &'a [Pair], dt: f32) -> impl Iterator<Item = Contact> + 'a {
		pairs
			.iter()
			.filter_map(move |pair| {
				Some((
					(self.bodies.get_slot(pair.a)?, pair.a),
					(self.bodies.get_slot(pair.b)?, pair.b),
				))
			})
			.filter_map(move |(a, b)| intersect(a, b, dt))
	}

	fn resolve_contact(&mut self, contact: &Contact) {
		let Some(a) = self.bodies.get_slot(contact.a.object).cloned() else {
			return;
		};
		let Some(b) = self.bodies.get_slot(contact.b.object).cloned() else {
			return;
		};
		let inverse_mass_sum = a.inv_mass + b.inv_mass;
		if inverse_mass_sum == 0.0 {
			return;
		}

		let a_center_of_mass = a.world_space_center_of_mass();
		let b_center_of_mass = b.world_space_center_of_mass();
		let mut normal = contact.normal;
		if (b_center_of_mass - a_center_of_mass).dot(normal.into_vector()) < 0.0 {
			normal = -normal;
		}
		let a_radius = contact.a.point - a_center_of_mass;
		let b_radius = contact.b.point - b_center_of_mass;
		let normal_vector = normal.into_vector();
		let a_inverse_inertia = a.inverse_world_space_inertia_tensor();
		let b_inverse_inertia = b.inverse_world_space_inertia_tensor();
		let a_angular_factor =
			Vector::from_maths(a_inverse_inertia * a_radius.cross(normal_vector).into_maths()).cross(a_radius);
		let b_angular_factor =
			Vector::from_maths(b_inverse_inertia * b_radius.cross(normal_vector).into_maths()).cross(b_radius);
		let angular_factor = (a_angular_factor + b_angular_factor).dot(normal_vector);
		let a_velocity = a.linear_velocity + a.angular_velocity.cross(a_radius);
		let b_velocity = b.linear_velocity + b.angular_velocity.cross(b_radius);
		let relative_velocity = a_velocity - b_velocity;
		let impulse_denominator = inverse_mass_sum + angular_factor;
		debug_assert!(
			impulse_denominator.is_finite() && impulse_denominator > f32::EPSILON,
			"Collision impulse denominator is invalid. The most likely cause is non-finite body mass or inertia data."
		);
		if !impulse_denominator.is_finite() || impulse_denominator <= f32::EPSILON {
			return;
		}
		let impulse = (1.0 + a.elasticity * b.elasticity) * relative_velocity.dot(normal_vector) / impulse_denominator;
		let impulse_vector = normal * impulse;
		if let Some(a) = self.bodies.get_slot_mut(contact.a.object) {
			a.apply_impulse(contact.a.point, -impulse_vector);
		}
		if let Some(b) = self.bodies.get_slot_mut(contact.b.object) {
			b.apply_impulse(contact.b.point, impulse_vector);
		}

		let normal_velocity = normal * relative_velocity.dot(normal_vector);
		let tangent_velocity = relative_velocity - normal_velocity;
		if tangent_velocity.length_squared() > f32::EPSILON {
			let tangent = tangent_velocity.normalized().expect("non-zero tangent velocity");
			let tangent_vector = tangent.into_vector();
			let a_friction_factor =
				Vector::from_maths(a_inverse_inertia * a_radius.cross(tangent_vector).into_maths()).cross(a_radius);
			let b_friction_factor =
				Vector::from_maths(b_inverse_inertia * b_radius.cross(tangent_vector).into_maths()).cross(b_radius);
			let friction_denominator = inverse_mass_sum + (a_friction_factor + b_friction_factor).dot(tangent_vector);
			debug_assert!(
				friction_denominator.is_finite() && friction_denominator > f32::EPSILON,
				"Friction impulse denominator is invalid. The most likely cause is non-finite body mass or inertia data."
			);
			if friction_denominator.is_finite() && friction_denominator > f32::EPSILON {
				let friction_impulse = tangent_velocity * ((a.friction * b.friction) / friction_denominator);
				if let Some(a) = self.bodies.get_slot_mut(contact.a.object) {
					a.apply_impulse(contact.a.point, -friction_impulse);
				}
				if let Some(b) = self.bodies.get_slot_mut(contact.b.object) {
					b.apply_impulse(contact.b.point, friction_impulse);
				}
			}
		}

		if contact.toi == 0.0 {
			let separation = normal * contact.depth;
			if let Some(a) = self.bodies.get_slot_mut(contact.a.object) {
				a.position = a.position - separation * (a.inv_mass / inverse_mass_sum);
			}
			if let Some(b) = self.bodies.get_slot_mut(contact.b.object) {
				b.position += separation * (b.inv_mass / inverse_mass_sum);
			}
		}
	}

	fn create_body(&mut self, handle: Handle, body: Body) {
		// Creation messages are upserts so one entity handle always owns at most one simulated body.
		self.remove_body(handle);

		let body_type = body.body_type();
		let mass = body.mass();
		debug_assert!(
			body_type != BodyTypes::Dynamic || (mass.is_finite() && mass > 0.0),
			"Dynamic body mass is invalid. The most likely cause is creating a body with a nonpositive or non-finite mass."
		);
		let inv_mass = if body_type == BodyTypes::Dynamic { 1.0 / mass } else { 0.0 };
		let linear_velocity = body.velocity();
		let center_of_mass = body.center_of_mass();
		let elasticity = body.elasticity();
		let friction = body.friction();
		let collision_shape = body.into_shape();
		let index = self.bodies.push(PhysicsBody {
			body_type,
			// Spatial state arrives independently through TransformationUpdate.
			position: math::Point::origin(),
			orientation: math::Orientation::identity(),
			scale: Scale::identity(),
			linear_velocity,
			angular_velocity: Vector::zero(),
			acceleration: Vector::zero(),
			collision_shape,
			inv_mass,
			center_of_mass,
			elasticity,
			friction,
			handle,
		});
		self.handles_to_bodies.insert(handle, index);
	}

	/// Removes body state for every pending deletion message.
	pub fn process_pending_deletions(&mut self) {
		while let Some(message) = self.body_delete_listener.read() {
			self.remove_body(message.into_handle());
		}
	}

	/// Removes and returns the physics body for `handle`.
	pub fn remove_body(&mut self, handle: Handle) -> Option<PhysicsBody> {
		self.handles_to_bodies
			.remove(&handle)
			.and_then(|index| self.bodies.remove(index))
	}
}

#[cfg(test)]
mod tests {

	use math::Orientation;
	use smallvec::SmallVec;

	use super::*;
	use crate::{
		core::{channel::DefaultChannel, factory::Factory},
		physics::collider::Shapes,
	};

	fn test_handle() -> Handle {
		let mut factory = Factory::<()>::new();
		factory.create(())
	}

	fn make_world() -> World {
		let body_factory = Factory::<Body>::new();
		let delete_channel = DefaultChannel::new();
		World::new(body_factory.listener(), delete_channel.listener())
	}

	fn make_ground_body() -> PhysicsBody {
		PhysicsBody {
			body_type: BodyTypes::Static,
			collision_shape: Shapes::Cube {
				size: Vector::new(4.0, 1.0, 4.0),
			},
			position: math::Point::origin(),
			orientation: Orientation::identity(),
			scale: Scale::identity(),
			acceleration: Vector::zero(),
			linear_velocity: Vector::zero(),
			angular_velocity: Vector::zero(),
			inv_mass: 0.0,
			center_of_mass: math::Point::origin(),
			elasticity: 0.0,
			friction: 0.0,
			handle: test_handle(),
		}
	}

	fn make_dynamic_sphere_body(position: math::Point, linear_velocity: Vector, radius: f32) -> PhysicsBody {
		PhysicsBody {
			body_type: BodyTypes::Dynamic,
			collision_shape: Shapes::Sphere { radius },
			position,
			orientation: Orientation::identity(),
			scale: Scale::identity(),
			acceleration: Vector::zero(),
			linear_velocity,
			angular_velocity: Vector::zero(),
			inv_mass: 1.0,
			center_of_mass: math::Point::origin(),
			elasticity: 0.0,
			friction: 0.0,
			handle: test_handle(),
		}
	}

	fn resolve_penetration_depth(bodies: Vec<PhysicsBody>, dt: f32) -> f32 {
		let mut world = make_world();
		world.bodies = bodies.into_iter().collect();
		let pairs = broadphase(world.bodies.indexed_iter(), dt);
		let contacts = world
			.detect_collisions_from_pairs(&pairs, dt)
			.collect::<SmallVec<[Contact; 8]>>();

		assert_eq!(contacts.len(), 1);
		world.resolve_contact(&contacts[0]);

		intersect(
			(world.bodies.get_slot(0).expect("first test body"), 0),
			(world.bodies.get_slot(1).expect("second test body"), 1),
			dt,
		)
		.map_or(0.0, |contact| contact.depth)
	}

	#[test]
	fn apply_impulse_updates_registered_body_and_reports_unknown_handles() {
		let mut world = make_world();
		let body = make_dynamic_sphere_body(math::Point::origin(), Vector::zero(), 1.0);
		let handle = body.handle;
		let index = world.bodies.push(body);
		world.handles_to_bodies.insert(handle, index);

		assert!(world.apply_impulse(handle, Vector::new(2.0, 0.0, 0.0)));
		assert_eq!(world.bodies[index].linear_velocity, Vector::new(2.0, 0.0, 0.0));
		assert!(world.remove_body(handle).is_some());
		assert!(!world.apply_impulse(handle, Vector::new(1.0, 0.0, 0.0)));
		assert!(!world.apply_impulse(test_handle(), Vector::new(1.0, 0.0, 0.0)));
	}

	#[test]
	fn creation_with_an_existing_handle_replaces_the_registered_body() {
		let mut world = make_world();
		let handle = test_handle();
		let first = Body::new(BodyTypes::Dynamic, Shapes::sphere(1.0));
		let replacement_velocity = Vector::new(4.0, 5.0, 6.0);
		let replacement = Body::new(BodyTypes::Dynamic, Shapes::sphere(2.0)).with_velocity(replacement_velocity);

		world.create_body(handle, first);
		world.create_body(handle, replacement);

		assert_eq!(world.bodies.len(), 1);
		let index = world.handles_to_bodies[&handle];
		assert_eq!(world.bodies[index].position, math::Point::origin());
		assert_eq!(world.bodies[index].linear_velocity, replacement_velocity);
		assert!(matches!(
			&world.bodies[index].collision_shape,
			Shapes::Sphere { radius } if *radius == 2.0
		));
	}

	#[test]
	fn transformation_update_after_creation_sets_all_spatial_state() {
		let mut body_factory = Factory::<Body>::new();
		let delete_channel = DefaultChannel::new();
		let mut world = World::new(body_factory.listener(), delete_channel.listener());
		let mut transforms = DefaultChannel::new();
		let mut transforms_rx = transforms.listener();
		let handle = body_factory.create(Body::new(BodyTypes::Static, Shapes::sphere(1.0)));
		let expected = Transform::new(
			math::Point::new(3.0, 2.0, 1.0),
			Scale::new(2.0, 3.0, 4.0),
			Orientation::identity(),
		);
		transforms.send(TransformationUpdate::new(handle, expected.clone()));

		world.update(
			Time::new(MediaTime::ZERO, MediaTime::ZERO),
			&mut transforms_rx,
			&mut transforms,
			&mut bumpalo::Bump::new(),
		);

		let body = &world.bodies[world.handles_to_bodies[&handle]];
		assert_eq!(body.position, expected.get_position());
		assert_eq!(body.orientation, expected.get_orientation());
		assert_eq!(body.scale, expected.scale());
	}

	#[test]
	fn published_transform_preserves_scale() {
		let mut world = make_world();
		let mut body = make_dynamic_sphere_body(math::Point::origin(), Vector::zero(), 1.0);
		body.scale = Scale::new(2.0, 3.0, 4.0);
		let expected_scale = body.scale;
		let handle = body.handle;
		world.bodies.push(body);
		let mut transforms = DefaultChannel::new();
		let mut transforms_rx = transforms.listener();

		world.update_bodies(MediaTime::ZERO, &mut transforms);

		let update = transforms_rx.read().expect("dynamic body transform update");
		assert_eq!(*update.handle(), handle);
		assert_eq!(update.transform().scale(), expected_scale);
	}

	#[test]
	fn detects_each_pair_once() {
		let mut world = make_world();
		world.bodies = [
			make_ground_body(),
			make_dynamic_sphere_body(math::Point::new(0.0, 1.4, 0.0), Vector::zero(), 0.5),
		]
		.into_iter()
		.collect();
		let pairs = broadphase(world.bodies.indexed_iter(), 1.0);
		let contacts = world
			.detect_collisions_from_pairs(&pairs, 1.0)
			.collect::<SmallVec<[Contact; 8]>>();

		assert_eq!(contacts.len(), 1);
		assert_eq!((contacts[0].a.object, contacts[0].b.object), (0, 1));
	}

	#[test]
	fn resolves_sphere_ground_penetration_for_both_body_orders() {
		let ground_first = resolve_penetration_depth(
			vec![
				make_ground_body(),
				make_dynamic_sphere_body(math::Point::new(0.0, 1.4, 0.0), Vector::zero(), 0.5),
			],
			1.0,
		);
		let sphere_first = resolve_penetration_depth(
			vec![
				make_dynamic_sphere_body(math::Point::new(0.0, 1.4, 0.0), Vector::zero(), 0.5),
				make_ground_body(),
			],
			1.0,
		);

		assert!(ground_first <= 1e-4);
		assert!(sphere_first <= 1e-4);
	}

	#[test]
	fn resolves_overlapping_spheres_without_deepening_penetration() {
		let depth = resolve_penetration_depth(
			vec![
				make_dynamic_sphere_body(math::Point::origin(), Vector::new(-1.0, 0.0, 0.0), 1.0),
				make_dynamic_sphere_body(math::Point::new(1.5, 0.0, 0.0), Vector::new(1.0, 0.0, 0.0), 1.0),
			],
			1.0,
		);

		assert!(depth <= 1e-4);
	}
}
