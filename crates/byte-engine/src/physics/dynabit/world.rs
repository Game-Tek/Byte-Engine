use math::{Scale, Vector};
use smallvec::SmallVec;
use utils::{StableVec, StableVecHandle, hash::HashMap, sync::Mutex};

use crate::{
	application::Time,
	core::{
		alley::{Alley, ConcreteLane, Lane as _},
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
pub struct World {
	bodies: StableVec<PhysicsBody>,
	gravity: Vector,
	body_listener: DefaultListener<CreateMessage<Body>>,
	body_delete_listener: DefaultListener<DeleteMessage>,
	handles_to_bodies: HashMap<Handle, StableVecHandle>,
	/// Scratch buffer, indexed by body slot, recording how far into the current step each body has advanced.
	body_times: Vec<MediaTime>,
	/// The candidate pairs of the step [`Self::update_with_alley`] is running, shared by every lane's narrow phase.
	pairs: SmallVec<[Pair; 32]>,
	/// The contacts each lane found in its share of `pairs`, indexed by lane. Each lane locks only its own entry.
	lane_contacts: Vec<Mutex<Vec<Contact>>>,
	/// The step's contacts gathered from every lane, kept between steps so gathering does not allocate.
	contacts: Vec<Contact>,
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
			handles_to_bodies: HashMap::with_capacity_and_hasher(1024, Default::default()),
			body_times: Vec::new(),
			pairs: SmallVec::new(),
			lane_contacts: Vec::new(),
			contacts: Vec::new(),
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
		transforms_tx: &impl Channel<TransformationUpdate>,
		allocator: &mut bumpalo::Bump,
	) {
		self.adopt_updates(transforms_rx);

		let step = time.delta();
		self.update_velocities(step);
		let remaining = step - self.update_collisions(step, allocator);
		self.update_bodies(remaining, transforms_tx);
	}

	/// Processes entity updates, advances physics as one execution of `alley`, and publishes resulting transforms.
	///
	/// Every lane runs the same step. One lane applies gravity and finds the candidate pairs while the others wait,
	/// every lane then tests its share of the pairs, and one lane resolves the contacts and moves the bodies.
	/// Contacts are resolved in the order [`Self::update`] resolves them, so both produce the same simulation.
	pub fn update_with_alley(
		&mut self,
		time: Time,
		transforms_rx: &mut impl Listener<TransformationUpdate>,
		transforms_tx: &(impl Channel<TransformationUpdate> + Sync),
		alley: &mut Alley,
	) {
		// Listeners belong to the calling thread, so messages are adopted before the lanes start.
		self.adopt_updates(transforms_rx);

		let step = time.delta();
		// Entries are only added, so each lane keeps the contact storage it grew in earlier steps.
		if self.lane_contacts.len() < alley.parallelism() {
			self.lane_contacts.resize_with(alley.parallelism(), Default::default);
		}
		let executed = alley.execute_phased(self, |lane, world| {
			lane.broadcast_mut(world, |world| world.find_pairs(step));
			let _ = lane.each_ref(world, |world, lane| world.detect_lane_contacts(lane, step));
			lane.broadcast_mut(world, |world| world.resolve_lane_contacts(step, transforms_tx));
		});
		if let Err(panic) = executed {
			std::panic::resume_unwind(panic);
		}
	}

	/// Registers created bodies and applies authored transforms to the bodies they target.
	fn adopt_updates(&mut self, transforms_rx: &mut impl Listener<TransformationUpdate>) {
		while let Some(message) = self.body_listener.read() {
			let handle = message.handle();
			let body = message.into_data();
			self.create_body(handle, body);
		}
		while let Some(message) = transforms_rx.read() {
			if let Some(index) = self.handles_to_bodies.get(&message.handle()).copied() {
				let body = &mut self.bodies[index];
				body.position = message.transform().get_position();
				body.orientation = message.transform().get_orientation();
				body.scale = message.transform().scale();
			}
		}
	}

	/// Applies gravity and finds the step's candidate pairs for every lane's narrow phase.
	fn find_pairs(&mut self, dt: MediaTime) {
		self.update_velocities(dt);
		self.pairs = broadphase(self.bodies.indexed_iter(), dt.as_seconds_f32());
	}

	/// Tests one lane's share of the candidate pairs and keeps the contacts found in that lane's storage.
	fn detect_lane_contacts(&self, lane: &ConcreteLane<'_>, dt: MediaTime) {
		let storage = &self.lane_contacts[lane.idx()];
		// Work on the stack so lanes do not write to neighboring storage headers while they test pairs.
		let mut contacts = std::mem::take(&mut *storage.lock());
		contacts.extend(self.detect_collisions_from_pairs(lane.share(&self.pairs), dt.as_seconds_f32()));
		*storage.lock() = contacts;
	}

	/// Resolves the contacts every lane found, then moves the bodies through the rest of the step.
	fn resolve_lane_contacts(&mut self, dt: MediaTime, transforms_tx: &impl Channel<TransformationUpdate>) {
		let mut contacts = std::mem::take(&mut self.contacts);
		// Lane shares are contiguous and in pair order, so gathering by lane keeps the order pairs were found in.
		for lane_contacts in &mut self.lane_contacts {
			contacts.append(lane_contacts.get_mut());
		}
		let remaining = dt - self.resolve_contacts(&mut contacts);
		contacts.clear();
		self.contacts = contacts;
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
		self.resolve_contacts(&mut contacts)
	}

	/// Resolves `contacts` in time-of-impact order and returns how far into the step the last one happened.
	///
	/// Every body is left advanced to that time.
	fn resolve_contacts(&mut self, contacts: &mut [Contact]) -> MediaTime {
		contacts.sort();

		// Each body records how far into the step it has advanced. A contact then only advances its own two bodies,
		// so the loop costs O(contacts + bodies) instead of advancing every body once per contact.
		self.body_times.clear();
		self.body_times.resize(self.bodies.slots_len(), MediaTime::ZERO);
		let mut accumulated = MediaTime::ZERO;
		for contact in contacts.iter() {
			// Contacts are sorted by time of impact, so contact times never go backwards.
			let contact_time = MediaTime::from_seconds_f32(contact.toi.max(0.0));
			for index in [contact.a.object, contact.b.object] {
				if let Some(body) = self.bodies.get_slot_mut(index) {
					advance_body(body, &mut self.body_times[index], contact_time);
				}
			}
			self.resolve_contact(contact);
			accumulated = contact_time;
		}
		// Catch up the bodies that no contact touched, or that were last touched earlier in the step.
		for (index, body) in self.bodies.indexed_iter_mut() {
			advance_body(body, &mut self.body_times[index], accumulated);
		}
		accumulated
	}

	/// Advances dynamic bodies and publishes their transforms.
	pub fn update_bodies(&mut self, dt: MediaTime, transforms_tx: &impl Channel<TransformationUpdate>) {
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
			// Two bodies without mass cannot exchange an impulse, so their contact would be discarded when resolved.
			.filter(|((a, _), (b, _))| a.inv_mass + b.inv_mass != 0.0)
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

/// Advances a dynamic `body` from `body_time` to `time`, which is never earlier, within the current step.
///
/// Only dynamic bodies move, matching [`World::update_bodies`].
fn advance_body(body: &mut PhysicsBody, body_time: &mut MediaTime, time: MediaTime) {
	if body.body_type == BodyTypes::Dynamic && time > *body_time {
		body.update(time - *body_time);
	}
	*body_time = time;
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
		let factory = Factory::<()>::new();
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
	fn published_transform_preserves_scale() {
		let mut world = make_world();
		let mut body = make_dynamic_sphere_body(math::Point::origin(), Vector::zero(), 1.0);
		body.scale = Scale::new(2.0, 3.0, 4.0);
		let expected_scale = body.scale;
		let handle = body.handle;
		world.bodies.push(body);
		let transforms = DefaultChannel::new();
		let mut transforms_rx = transforms.listener();

		world.update_bodies(MediaTime::ZERO, &transforms);

		let update = transforms_rx.read().expect("dynamic body transform update");
		assert_eq!(update.handle(), handle);
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
	fn bodies_outside_contacts_reach_the_end_of_the_step() {
		let mut world = make_world();
		let velocity = Vector::new(0.0, 0.0, 2.0);
		world.bodies = [
			make_ground_body(),
			make_dynamic_sphere_body(math::Point::new(0.0, 1.4, 0.0), Vector::zero(), 0.5),
			make_dynamic_sphere_body(math::Point::new(20.0, 0.0, 0.0), velocity, 0.5),
		]
		.into_iter()
		.collect();
		let transforms = DefaultChannel::new();
		let step = MediaTime::from_seconds_f32(0.5);

		let remaining = step - world.update_collisions(step, &mut bumpalo::Bump::new());
		world.update_bodies(remaining, &transforms);

		let free_body = world.bodies.get_slot(2).expect("free test body");
		assert!((free_body.position - math::Point::new(20.0, 0.0, 1.0)).length() <= 1e-4);
	}

	/// Returns a ground body under a grid of spheres and boxes that overlap it and each other, so one step
	/// produces more contacts than lanes.
	fn crowded_bodies() -> Vec<PhysicsBody> {
		let mut bodies = vec![make_ground_body()];
		for index in 0..24 {
			let position = math::Point::new((index % 6) as f32 * 0.8 - 2.0, 1.4, (index / 6) as f32 * 0.8 - 1.2);
			let mut body = make_dynamic_sphere_body(position, Vector::new(0.0, -1.0, 0.0), 0.5);
			if index % 3 == 0 {
				body.collision_shape = Shapes::Cube {
					size: Vector::new(0.5, 0.5, 0.5),
				};
			}
			bodies.push(body);
		}
		bodies
	}

	/// Steps a world of [`crowded_bodies`] and returns every transform it published, in publication order.
	fn published_positions(
		mut step: impl FnMut(&mut World, Time, &mut DefaultListener<TransformationUpdate>, &DefaultChannel<TransformationUpdate>),
	) -> Vec<(math::Point, Orientation)> {
		let mut world = make_world();
		world.bodies = crowded_bodies().into_iter().collect();
		let authored = DefaultChannel::new();
		let mut authored_rx = authored.listener();
		let published = DefaultChannel::new();
		let mut published_rx = published.listener();
		let delta = MediaTime::from_seconds_f32(1.0 / 60.0);
		let mut elapsed = MediaTime::ZERO;
		let mut positions = Vec::new();

		for _ in 0..30 {
			elapsed += delta;
			step(&mut world, Time::new(elapsed, delta), &mut authored_rx, &published);
			while let Some(update) = published_rx.read() {
				positions.push((update.transform().get_position(), update.transform().get_orientation()));
			}
		}
		positions
	}

	#[test]
	fn alley_step_publishes_the_same_transforms_as_the_serial_step() {
		let mut allocator = bumpalo::Bump::new();
		let serial = published_positions(|world, time, authored, published| {
			world.update(time, authored, published, &mut allocator);
			allocator.reset();
		});
		let mut alley = Alley::with_parallelism(4);
		let with_alley = published_positions(|world, time, authored, published| {
			world.update_with_alley(time, authored, published, &mut alley);
		});

		assert_eq!(serial.len(), 30 * 24);
		assert_eq!(with_alley, serial);
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
