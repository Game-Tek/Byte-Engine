use std::ops::Range;

use math::{Scale, Vector};
use utils::{StableVec, StableVecHandle, hash::HashMap, sync::Mutex};

use crate::{
	application::Time,
	core::{
		alley::{Alley, Lane as _},
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
			contact::Contact,
			solver::{self, ContactBatches, Response},
		},
		intersection::SweepBounds,
	},
	time::MediaTime,
};

/// The fewest contacts a batch needs before [`World::update_with_alley`] shares it between lanes.
///
/// Sharing a batch costs two more stages that every lane has to meet at, which is more than a small batch takes
/// to resolve on one lane. With four lanes on an Apple M4, steps of 1,000 and of 4,000 stacked cubes got no
/// faster below this size.
const SHARED_BATCH_CONTACTS: usize = 128;

/// The number of bodies from which [`World::update_with_alley`] runs a step on [`LARGE_STEP_LANES`] lanes
/// instead of [`SMALL_STEP_LANES`].
const LARGE_STEP_BODIES: usize = 4000;

/// The lanes that run a step of fewer than [`LARGE_STEP_BODIES`] bodies.
///
/// Every stage of a step ends only once each running lane has reached it, and a small step's shares are short,
/// so more lanes spend longer meeting than they save.
const SMALL_STEP_LANES: usize = 2;

/// The lanes that run a step of at least [`LARGE_STEP_BODIES`] bodies.
const LARGE_STEP_LANES: usize = 4;

/// The most batches of one step [`World::update_with_alley`] shares between lanes.
///
/// An alley execution has a fixed number of stages, and every shared batch takes two of them.
const MAX_SHARED_BATCHES: usize = 8;

/// The `World` struct owns Dynabit simulation state and synchronizes it with entity handles.
pub struct World {
	bodies: StableVec<PhysicsBody>,
	gravity: Vector,
	body_listener: DefaultListener<CreateMessage<Body>>,
	body_delete_listener: DefaultListener<DeleteMessage>,
	handles_to_bodies: HashMap<Handle, StableVecHandle>,
	/// Scratch buffer, indexed by body slot, recording how far into the current step each body has advanced.
	body_times: Vec<MediaTime>,
	/// The bounds every body sweeps through during the current step, which contact detection scans for overlaps.
	bounds: SweepBounds,
	/// The contacts each lane found in its share of `bounds`, indexed by lane. Each lane locks only its own entry.
	share_contacts: Vec<Mutex<Vec<Contact>>>,
	/// The step's contacts in time-of-impact order, kept between steps so gathering them does not allocate.
	contacts: Vec<Contact>,
	/// The contacts at the start of the step, sorted into batches that can each be resolved in any order.
	batches: ContactBatches,
	/// The responses each lane computed for its share of one batch, indexed by lane like `share_contacts`.
	share_responses: Vec<Mutex<Vec<Response>>>,
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
			bounds: SweepBounds::default(),
			share_contacts: Vec::new(),
			contacts: Vec::new(),
			batches: ContactBatches::default(),
			share_responses: Vec::new(),
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
	) {
		self.adopt_updates(transforms_rx);

		let step = time.delta();
		self.update_velocities(step);
		let remaining = step - self.update_collisions(step);
		self.update_bodies(remaining, transforms_tx);
	}

	/// Processes entity updates, advances physics as one execution of `alley`, and publishes resulting transforms.
	///
	/// A step of fewer than 4,000 bodies runs on two of `alley`'s lanes and a larger one on four; the other lanes
	/// stay asleep. Every running lane runs the same step. One lane applies gravity and sorts the bodies' bounds
	/// while the others wait, every lane then finds the contacts in its share of the bounds, and one lane resolves
	/// them and moves the bodies. Lanes also share the larger batches of simultaneous contacts. Contacts are
	/// resolved in the order [`Self::update`] resolves them, so both produce the same simulation with any number
	/// of lanes.
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
		let preferred_lanes = if self.bodies.len() < LARGE_STEP_BODIES {
			SMALL_STEP_LANES
		} else {
			LARGE_STEP_LANES
		};
		let lanes = preferred_lanes.min(alley.parallelism());
		// Entries are only added, so each lane keeps the storage it grew in earlier steps.
		if self.share_contacts.len() < lanes {
			self.share_contacts.resize_with(lanes, Default::default);
			self.share_responses.resize_with(lanes, Default::default);
		}
		let executed = alley.execute_phased(self, lanes, |lane, world| {
			lane.broadcast_mut(world, |world| {
				world.update_velocities(step);
				world.sweep_bodies(step);
			});
			let _ = lane.each_ref(world, |world, lane| world.detect_share_contacts(lane.idx(), lanes, step));
			let shared_batches = lane.broadcast_mut(world, |world| world.batch_share_contacts());
			for batch in 0..shared_batches {
				let _ = lane.each_ref(world, |world, lane| world.respond_to_share(batch, lane.idx(), lanes));
				lane.broadcast_mut(world, |world| world.apply_share_responses());
			}
			lane.broadcast_mut(world, |world| {
				let remaining = step - world.resolve_contacts(shared_batches);
				world.update_bodies(remaining, transforms_tx);
			});
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

	/// Sorts the bounds every body sweeps through during the step, ready for [`Self::detect_contacts`].
	fn sweep_bodies(&mut self, dt: MediaTime) {
		let seconds = dt.as_seconds_f32();
		self.bounds.rebuild(
			self.bodies
				.indexed_iter()
				.map(|(index, body)| (index, body.swept_bounds(seconds))),
		);
	}

	/// Adds to `contacts` the contacts between each body in `range` of the swept bounds and the bodies after it.
	///
	/// Ranges that cover every swept bound find each contact once, in the same order whatever the ranges are.
	fn detect_contacts(&self, range: Range<usize>, dt: f32, contacts: &mut Vec<Contact>) {
		self.bounds.overlaps(range, |pair| {
			let (Some(a), Some(b)) = (self.bodies.get_slot(pair.a), self.bodies.get_slot(pair.b)) else {
				return;
			};
			// Two bodies without mass cannot exchange an impulse, so their contact would change nothing.
			if a.inv_mass + b.inv_mass != 0.0 {
				contacts.extend(intersect((a, pair.a), (b, pair.b), dt));
			}
		});
	}

	/// Finds the contacts in one of `shares` equal parts of the swept bounds and keeps them in that share's storage.
	fn detect_share_contacts(&self, share: usize, shares: usize, dt: MediaTime) {
		let storage = &self.share_contacts[share];
		// Work on the stack so lanes do not write to neighboring storage headers while they test bodies.
		let mut contacts = std::mem::take(&mut *storage.lock());
		self.detect_contacts(
			share_range(self.bounds.count(), share, shares),
			dt.as_seconds_f32(),
			&mut contacts,
		);
		*storage.lock() = contacts;
	}

	/// Gathers the contacts found in every share and batches them, then returns how many batches lanes should share.
	fn batch_share_contacts(&mut self) -> usize {
		// Shares are consecutive ranges of the swept bounds, so gathering by share keeps the order one scan finds.
		for share_contacts in &mut self.share_contacts {
			self.contacts.append(share_contacts.get_mut());
		}
		self.batch_contacts();
		// Batches fill from the first one, so the large ones lead.
		(0..self.batches.len().min(MAX_SHARED_BATCHES))
			.take_while(|batch| self.batches.batch(*batch).len() >= SHARED_BATCH_CONTACTS)
			.count()
	}

	/// Orders the step's contacts by time of impact and batches the ones at the start of the step.
	///
	/// Bodies that already overlap are in contact at the same time, so no order among their contacts is more
	/// correct than another. Batching picks one that lets a batch's contacts be resolved together.
	fn batch_contacts(&mut self) {
		self.contacts.sort();
		let immediate = self.immediate_contacts();
		let bodies = &self.bodies;
		// A body without mass is never changed by a contact, so any number of a batch's contacts can touch it.
		let movable = |index: usize| {
			bodies
				.get_slot(index)
				.is_some_and(|body| body.inv_mass != 0.0)
				.then_some(index)
		};
		self.batches.rebuild(
			bodies.slots_len(),
			self.contacts[..immediate]
				.iter()
				.map(|contact| [movable(contact.a.object), movable(contact.b.object)]),
		);
	}

	/// Returns how many of the sorted contacts happen at the start of the step.
	fn immediate_contacts(&self) -> usize {
		self.contacts.partition_point(|contact| contact.toi <= 0.0)
	}

	/// Computes the responses to one of `shares` equal parts of a batch and keeps them in that share's storage.
	fn respond_to_share(&self, batch: usize, share: usize, shares: usize) {
		let storage = &self.share_responses[share];
		let mut responses = std::mem::take(&mut *storage.lock());
		let batch = self.batches.batch(batch);
		responses.extend(
			batch[share_range(batch.len(), share, shares)]
				.chunks(solver::WIDTH)
				.map(|group| solver::respond(&self.bodies, &self.contacts, group)),
		);
		*storage.lock() = responses;
	}

	/// Writes the responses computed for every share of a batch to the bodies.
	fn apply_share_responses(&mut self) {
		for share_responses in &mut self.share_responses {
			for response in share_responses.get_mut().drain(..) {
				solver::apply(&mut self.bodies, &response);
			}
		}
	}

	/// Applies gravity-derived impulses to dynamic bodies.
	pub fn update_velocities(&mut self, dt: MediaTime) {
		let seconds = dt.as_seconds_f32();
		for body in self.bodies.iter_mut().filter(|body| body.body_type == BodyTypes::Dynamic) {
			body.apply_linear_impulse(self.gravity * body.mass() * seconds);
		}
	}

	/// Detects and resolves contacts for this time step, and returns how far into the step the last one happened.
	pub fn update_collisions(&mut self, dt: MediaTime) -> MediaTime {
		self.sweep_bodies(dt);
		let mut contacts = std::mem::take(&mut self.contacts);
		self.detect_contacts(0..self.bounds.count(), dt.as_seconds_f32(), &mut contacts);
		self.contacts = contacts;
		self.batch_contacts();
		self.resolve_contacts(0)
	}

	/// Resolves the step's batches from `first_batch` on, then the remaining contacts in time-of-impact order, and
	/// returns how far into the step the last contact happened.
	///
	/// Every body is left advanced to that time, and the step's contacts are cleared.
	fn resolve_contacts(&mut self, first_batch: usize) -> MediaTime {
		for batch in first_batch..self.batches.len() {
			for group in self.batches.batch(batch).chunks(solver::WIDTH) {
				let response = solver::respond(&self.bodies, &self.contacts, group);
				solver::apply(&mut self.bodies, &response);
			}
		}
		// Contacts that fit in no batch share bodies, so each one has to see the result of the one before it.
		for contact in self.batches.leftover() {
			let response = solver::respond(&self.bodies, &self.contacts, std::slice::from_ref(contact));
			solver::apply(&mut self.bodies, &response);
		}

		let immediate = self.immediate_contacts();
		let mut accumulated = MediaTime::ZERO;
		if immediate < self.contacts.len() {
			// Each body records how far into the step it has advanced. A contact then only advances its own two
			// bodies, so the loop costs O(contacts + bodies) instead of advancing every body once per contact.
			self.body_times.clear();
			self.body_times.resize(self.bodies.slots_len(), MediaTime::ZERO);
			for index in immediate..self.contacts.len() {
				let contact = &self.contacts[index];
				// Contacts are sorted by time of impact, so contact times never go backwards.
				let contact_time = MediaTime::from_seconds_f32(contact.toi.max(0.0));
				for body_index in [contact.a.object, contact.b.object] {
					if let Some(body) = self.bodies.get_slot_mut(body_index) {
						advance_body(body, &mut self.body_times[body_index], contact_time);
					}
				}
				let response = solver::respond(&self.bodies, &self.contacts, &[index as u32]);
				solver::apply(&mut self.bodies, &response);
				accumulated = contact_time;
			}
			// Catch up the bodies that no contact touched, or that were last touched earlier in the step.
			for (index, body) in self.bodies.indexed_iter_mut() {
				advance_body(body, &mut self.body_times[index], accumulated);
			}
		}
		self.contacts.clear();
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

/// Returns part `share` of `count` items split into `shares` consecutive parts of nearly equal size.
fn share_range(count: usize, share: usize, shares: usize) -> Range<usize> {
	count * share / shares..count * (share + 1) / shares
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

	/// Returns the contacts one step of `dt` seconds finds between the world's bodies.
	fn detected_contacts(world: &mut World, dt: f32) -> Vec<Contact> {
		world.sweep_bodies(MediaTime::from_seconds_f32(dt));
		let mut contacts = Vec::new();
		world.detect_contacts(0..world.bounds.count(), dt, &mut contacts);
		contacts
	}

	fn resolve_penetration_depth(bodies: Vec<PhysicsBody>, dt: f32) -> f32 {
		let mut world = make_world();
		world.bodies = bodies.into_iter().collect();

		assert_eq!(detected_contacts(&mut world, dt).len(), 1);
		world.update_collisions(MediaTime::from_seconds_f32(dt));

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
		let contacts = detected_contacts(&mut world, 1.0);

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

		let remaining = step - world.update_collisions(step);
		world.update_bodies(remaining, &transforms);

		let free_body = world.bodies.get_slot(2).expect("free test body");
		assert!((free_body.position - math::Point::new(20.0, 0.0, 1.0)).length() <= 1e-4);
	}

	/// The bodies along each side of the grid [`crowded_bodies`] returns.
	const CROWD_SIDE: usize = 16;

	/// Returns a ground body under a grid of spheres and boxes that overlap it and each other, so one step has
	/// enough contacts with the ground alone for lanes to share a batch.
	fn crowded_bodies() -> Vec<PhysicsBody> {
		const { assert!(CROWD_SIDE * CROWD_SIDE >= 2 * SHARED_BATCH_CONTACTS) };
		let mut ground = make_ground_body();
		ground.collision_shape = Shapes::Cube {
			size: Vector::new(8.0, 1.0, 8.0),
		};
		let mut bodies = vec![ground];
		for index in 0..CROWD_SIDE * CROWD_SIDE {
			let (column, row) = ((index % CROWD_SIDE) as f32, (index / CROWD_SIDE) as f32);
			let position = math::Point::new(column * 0.8 - 6.0, 1.4, row * 0.8 - 6.0);
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
		let serial = published_positions(|world, time, authored, published| {
			world.update(time, authored, published);
		});
		let mut alley = Alley::with_parallelism(4);
		let with_alley = published_positions(|world, time, authored, published| {
			world.update_with_alley(time, authored, published, &mut alley);
		});

		assert_eq!(serial.len(), 30 * CROWD_SIDE * CROWD_SIDE);
		assert_eq!(with_alley, serial);
	}

	#[test]
	fn spheres_that_meet_during_the_step_stop_where_they_touch() {
		let mut world = make_world();
		world.bodies = [
			make_dynamic_sphere_body(math::Point::new(-2.0, 0.0, 0.0), Vector::new(4.0, 0.0, 0.0), 0.5),
			make_dynamic_sphere_body(math::Point::new(2.0, 0.0, 0.0), Vector::new(-4.0, 0.0, 0.0), 0.5),
		]
		.into_iter()
		.collect();

		// The spheres close a three-meter gap at eight meters per second.
		let impact = world.update_collisions(MediaTime::from_seconds_f32(1.0));

		assert!((impact.as_seconds_f32() - 0.375).abs() <= 1e-4);
		let (first, second) = (world.bodies.get_slot(0).unwrap(), world.bodies.get_slot(1).unwrap());
		assert!((first.position - math::Point::new(-0.5, 0.0, 0.0)).length() <= 1e-3);
		assert!((second.position - math::Point::new(0.5, 0.0, 0.0)).length() <= 1e-3);
		// Equal masses without elasticity cancel each other's approach.
		assert!(first.linear_velocity.length() <= 1e-3);
		assert!(second.linear_velocity.length() <= 1e-3);
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
