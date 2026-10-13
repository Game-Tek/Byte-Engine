use std::ops::Range;

use math::{Scale, Vector};
use utils::{StableVec, StableVecHandle, hash::HashMap, sync::Mutex};

use crate::{
	application::Time,
	core::{
		alley::{Alley, COLLECTIVE_SECTION_COUNT, ConcreteLane, Lane as _},
		channel::Channel,
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
	},
	gameplay::transform::{Transform, TransformationUpdate},
	physics::{
		body::{Body, BodyTypes},
		dynabit::{
			body::{Motion, PhysicsBody, intersect},
			contact::{Contact, Contacts},
			solver::{self, ContactSolver, WideContact},
			step_lanes::StepLanes,
		},
		epa::ContactScratch,
		intersection::SweepBounds,
	},
	time::MediaTime,
};

/// The fewest contacts a batch needs before [`World::update_with_alley`] shares it between lanes.
///
/// Sharing a batch costs one more stage in every pass over the contacts, which every lane has to meet at. That is
/// more than a very small batch takes to solve on one lane. On an Apple M4, piles of 1,000 to 16,000 mixed boxes and
/// spheres stepped fastest with batches shared from about this size, and up to a third slower with none shared.
const SHARED_BATCH_CONTACTS: usize = 32;

/// The stages of an alley step besides the ones that prepare and solve contacts: three to find them, one to batch
/// them, one to finish them, and two to move the bodies.
const OTHER_STEP_STAGES: usize = 7;

/// The most batches of one step [`World::update_with_alley`] shares between lanes.
///
/// An alley execution has a fixed number of stages. Preparing the contacts, and then every pass over them, takes one
/// for each shared batch and one for the rest.
const MAX_SHARED_BATCHES: usize = (COLLECTIVE_SECTION_COUNT - OTHER_STEP_STAGES) / (solver::ITERATIONS + 1) - 1;

/// The `World` struct owns Dynabit simulation state and synchronizes it with entity handles.
pub struct World {
	bodies: StableVec<PhysicsBody>,
	gravity: Vector,
	body_listener: DefaultListener<CreateMessage<Body>>,
	body_delete_listener: DefaultListener<DeleteMessage>,
	handles_to_bodies: HashMap<Handle, StableVecHandle>,
	/// The bounds every body sweeps through during the current step, which contact detection scans for overlaps.
	bounds: SweepBounds,
	/// What each lane produced from its share of a stage, indexed by lane. Each lane locks only its own entry.
	shares: Vec<Mutex<Share>>,
	/// How many lanes [`Self::update_with_alley`] runs a step on.
	step_lanes: StepLanes,
	/// The step's contacts in the order they were found, kept between steps so gathering them does not allocate.
	contacts: Contacts,
	/// Working storage for finding contacts on the calling thread, kept between steps so that it does not allocate.
	/// Lanes have their own in [`Share`].
	scratch: ContactScratch,
	/// Solves the step's contacts.
	solver: ContactSolver,
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
			bounds: SweepBounds::default(),
			shares: Vec::new(),
			step_lanes: StepLanes::for_machine(),
			contacts: Contacts::default(),
			scratch: ContactScratch::default(),
			solver: ContactSolver::default(),
		}
	}

	/// Replaces how many lanes [`Self::update_with_alley`] runs a step on, which starts as
	/// [`StepLanes::for_machine`].
	///
	/// Use this when measurements on a machine show that other lane counts step faster there.
	pub fn set_step_lanes(&mut self, step_lanes: StepLanes) {
		self.step_lanes = step_lanes;
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
		self.step(time.delta(), transforms_tx);
	}

	/// Advances physics by `step` on the calling thread and publishes the resulting transforms.
	fn step(&mut self, step: MediaTime, transforms_tx: &impl Channel<TransformationUpdate>) {
		self.update_velocities(step);
		self.update_collisions(step);
		self.update_bodies(step, transforms_tx);
	}

	/// Processes entity updates, advances physics as one execution of `alley`, and publishes resulting transforms.
	///
	/// The world's [`StepLanes`] decide from the number of bodies how many of `alley`'s lanes run the step; the
	/// other lanes stay asleep. A step they give one lane runs on the calling thread, like [`Self::update`],
	/// because lanes would cost more than they save. Every running lane runs the same step. One lane applies
	/// gravity and sorts the bodies' bounds while the others wait, every lane then lists its part of the bounds in
	/// grid cells, gives the solver its share of the bodies, and finds the contacts in its share of the bounds, and
	/// one lane batches them. The lanes then prepare the contacts and pass over them several times, sharing the
	/// larger batches, before every lane moves its share of the bodies and one lane publishes their transforms.
	/// Contacts are prepared and solved in the order [`Self::update`] does it in, so both produce the same
	/// simulation with any number of lanes.
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
		let lanes = self.step_lanes.lanes(self.bodies.len()).min(alley.parallelism());
		if lanes <= 1 {
			return self.step(step, transforms_tx);
		}
		// Entries are only added, so each lane keeps the storage it grew in earlier steps.
		if self.shares.len() < lanes {
			self.shares.resize_with(lanes, Default::default);
		}
		let executed = alley.execute_phased(self, lanes, |lane, world| {
			let in_cells = lane.broadcast_mut(world, |world| {
				world.update_velocities(step);
				world.solver.reserve(world.bodies.slots_len());
				world.stage_bodies(step, lanes)
			});
			if in_cells {
				let _ = lane.each_ref(world, |world, lane| world.bounds.fill(lane.idx()));
			}
			let _ = lane.each_ref(world, |world, lane| {
				world.take_share_bodies(lane);
				world.detect_share_contacts(lane, step)
			});
			let (shared_batches, rest) = lane.broadcast_mut(world, |world| world.batch_share_contacts());
			for batch in 0..shared_batches {
				let _ = lane.each_ref(world, |world, lane| world.prepare_share(batch, lane, step));
			}
			if rest {
				lane.broadcast_mut(world, |world| world.prepare_rest(shared_batches, step));
			}
			for _ in 0..solver::ITERATIONS {
				for batch in 0..shared_batches {
					let _ = lane.each_ref(world, |world, lane| world.solve_share(batch, lane));
				}
				if rest {
					lane.broadcast_mut(world, |world| world.solver.solve_rest());
				}
			}
			lane.broadcast_mut(world, |world| world.finish_contacts());
			let _ = lane.each_ref(world, |world, lane| world.advance_share(lane, step));
			lane.broadcast_mut(world, |world| world.publish_share_motions(transforms_tx));
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

	/// Sorts the bounds every body sweeps through during the step, ready for [`detect_contacts`].
	fn sweep_bodies(&mut self, dt: MediaTime) {
		if self.stage_bodies(dt, 1) {
			self.bounds.fill(0);
		}
	}

	/// Sorts the bounds every body sweeps through during the step, and leaves their grid cells to be listed as
	/// `parts` parts. Returns whether there are cells to list, with [`SweepBounds::fill`].
	fn stage_bodies(&mut self, dt: MediaTime, parts: usize) -> bool {
		let seconds = dt.as_seconds_f32();
		let bounds = self.bodies.indexed_iter();
		self.bounds
			.stage(bounds.map(|(index, body)| (index, body.swept_bounds(seconds))), parts)
	}

	/// Gives the solver what the step's contacts need of the bodies in one lane's share of the body slots.
	#[allow(unsafe_code, reason = "Lanes take different shares of the body slots.")]
	fn take_share_bodies(&self, lane: &ConcreteLane<'_>) {
		let share = lane.share_range(self.bodies.slots_len());
		// SAFETY: This runs in a stage where every lane takes its own share of the slots, which do not overlap, and
		// no contact is prepared or solved.
		unsafe { self.solver.take_bodies(&self.bodies, share) };
	}

	/// Finds the contacts in one lane's share of the swept bounds and keeps them in that lane's storage.
	fn detect_share_contacts(&self, lane: &ConcreteLane<'_>, dt: MediaTime) {
		let storage = &self.shares[lane.idx()];
		// Work on the stack so lanes do not write to neighboring storage headers while they test bodies.
		let (mut contacts, mut scratch) = {
			let mut storage = storage.lock();
			(std::mem::take(&mut storage.contacts), std::mem::take(&mut storage.scratch))
		};
		let range = lane.share_range(self.bounds.count());
		detect_contacts(
			&self.bounds,
			&self.bodies,
			range,
			dt.as_seconds_f32(),
			&mut contacts,
			&mut scratch,
		);
		let mut storage = storage.lock();
		storage.contacts = contacts;
		storage.scratch = scratch;
	}

	/// Gathers the contacts found in every share and sorts them into batches. Returns how many batches lanes should
	/// share, and whether any contacts are left for one lane to prepare and solve.
	fn batch_share_contacts(&mut self) -> (usize, bool) {
		// Shares are consecutive ranges of the swept bounds, so taking them in lane order keeps the order one scan
		// finds. Each share gets the storage its contacts had in the last step.
		for (lane, share) in self.shares.iter_mut().enumerate() {
			std::mem::swap(self.contacts.part_mut(lane), &mut share.get_mut().contacts);
		}
		self.contacts.seal();
		self.solver.begin(&self.contacts);
		// Batches fill from the first one, so the large ones lead.
		let shared = (0..self.solver.batches().min(MAX_SHARED_BATCHES))
			.take_while(|batch| self.solver.batch_contacts(*batch) >= SHARED_BATCH_CONTACTS)
			.count();
		(shared, self.solver.has_rest(shared))
	}

	/// Prepares one lane's share of a batch's groups and keeps them in that lane's storage, after the groups of the
	/// batches before it.
	#[allow(
		unsafe_code,
		reason = "Lanes prepare shares of one batch, whose groups share no movable body."
	)]
	fn prepare_share(&self, batch: usize, lane: &ConcreteLane<'_>, dt: MediaTime) {
		let storage = &self.shares[lane.idx()];
		// Work on the stack so lanes do not write to neighboring storage headers while they prepare groups.
		let mut groups = std::mem::take(&mut storage.lock().groups);
		let share = lane.share_range(self.solver.batch_groups(batch));
		let seconds = dt.as_seconds_f32();
		// SAFETY: This runs in a stage where every lane prepares its own share of the groups of this one batch, and
		// nothing else touches the solver. Shares do not overlap, and the groups of a batch share no movable body.
		unsafe { self.solver.prepare_batch(&self.contacts, seconds, batch, share, &mut groups) };
		let mut storage = storage.lock();
		storage.batch_ends.push(groups.len());
		storage.groups = groups;
	}

	/// Prepares the contacts that follow the first `shared_batches` batches, which one lane then passes over in turn.
	fn prepare_rest(&mut self, shared_batches: usize, dt: MediaTime) {
		self.solver.prepare_rest(&self.contacts, dt.as_seconds_f32(), shared_batches);
	}

	/// Passes once over the groups one lane prepared of a batch.
	#[allow(
		unsafe_code,
		reason = "Lanes solve shares of one batch, whose groups share no movable body."
	)]
	fn solve_share(&self, batch: usize, lane: &ConcreteLane<'_>) {
		let storage = self.shares[lane.idx()].lock();
		let start = batch.checked_sub(1).map_or(0, |previous| storage.batch_ends[previous]);
		// SAFETY: This runs in a stage where every lane passes over the groups it prepared of this one batch, and
		// nothing else touches the solver. Those are different groups for every lane, and the groups of a batch
		// share no movable body.
		unsafe { self.solver.solve(&storage.groups[start..storage.batch_ends[batch]]) };
	}

	/// Works out where the dynamic bodies in one lane's share of the body slots are after `dt`, and keeps the
	/// results in that lane's storage.
	fn advance_share(&self, lane: &ConcreteLane<'_>, dt: MediaTime) {
		let storage = &self.shares[lane.idx()];
		let mut motions = std::mem::take(&mut storage.lock().motions);
		motions.extend(
			lane.share_range(self.bodies.slots_len())
				.filter_map(|slot| Some((slot, self.bodies.get_slot(slot)?)))
				.filter(|(_, body)| body.body_type == BodyTypes::Dynamic)
				.map(|(slot, body)| (slot, body.advanced(dt))),
		);
		storage.lock().motions = motions;
	}

	/// Moves the bodies to where [`Self::advance_share`] found them to be and publishes their transforms.
	///
	/// Shares are consecutive ranges of the body slots, so this publishes in the order [`Self::update_bodies`] does.
	fn publish_share_motions(&mut self, transforms_tx: &impl Channel<TransformationUpdate>) {
		for share in &mut self.shares {
			for (slot, motion) in share.get_mut().motions.drain(..) {
				if let Some(body) = self.bodies.get_slot_mut(slot) {
					body.apply(motion);
					publish(body, transforms_tx);
				}
			}
		}
	}

	/// Applies gravity-derived impulses to dynamic bodies.
	fn update_velocities(&mut self, dt: MediaTime) {
		let seconds = dt.as_seconds_f32();
		for body in self.bodies.iter_mut().filter(|body| body.body_type == BodyTypes::Dynamic) {
			body.apply_linear_impulse(self.gravity * body.mass() * seconds);
		}
	}

	/// Detects and solves contacts for this time step.
	fn update_collisions(&mut self, dt: MediaTime) {
		self.sweep_bodies(dt);
		let (range, seconds) = (0..self.bounds.count(), dt.as_seconds_f32());
		detect_contacts(
			&self.bounds,
			&self.bodies,
			range,
			seconds,
			self.contacts.part_mut(0),
			&mut self.scratch,
		);
		self.contacts.seal();
		// Without contacts the solver has nothing to do with the bodies.
		if self.contacts.len() != 0 {
			self.solver.take_all_bodies(&self.bodies);
		}
		self.solver.begin(&self.contacts);
		self.solver.prepare_rest(&self.contacts, seconds, 0);
		for _ in 0..solver::ITERATIONS {
			self.solver.solve_rest();
		}
		self.finish_contacts();
	}

	/// Gives the bodies the velocities the passes over the contacts left them with, and clears the step's contacts.
	fn finish_contacts(&mut self) {
		let shared = self.shares.iter_mut().flat_map(|share| share.get_mut().groups.iter());
		self.solver.finish(&mut self.bodies, shared);
		for share in &mut self.shares {
			let share = share.get_mut();
			share.groups.clear();
			share.batch_ends.clear();
		}
		self.contacts.clear();
	}

	/// Advances dynamic bodies and publishes their transforms.
	fn update_bodies(&mut self, dt: MediaTime, transforms_tx: &impl Channel<TransformationUpdate>) {
		for body in self.bodies.iter_mut().filter(|body| body.body_type == BodyTypes::Dynamic) {
			body.update(dt);
			publish(body, transforms_tx);
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
		let rotates = body.rotates();
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
			rotates,
			handle,
		});
		// The slot may have held another body, whose contacts the solver still remembers.
		self.solver.forget(index.index());
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

/// Adds to `contacts` the contacts between each body in `range` of the swept `bounds` and the bodies after it.
///
/// Ranges that cover every scan position find each contact once, in the same order whatever the ranges are.
/// `scratch` is working storage that only this call may use while it runs.
fn detect_contacts(
	bounds: &SweepBounds,
	bodies: &StableVec<PhysicsBody>,
	range: Range<usize>,
	dt: f32,
	contacts: &mut Vec<Contact>,
	scratch: &mut ContactScratch,
) {
	bounds.overlaps(range, |[first, second]| {
		let (Some(a), Some(b)) = (bodies.get_slot(first), bodies.get_slot(second)) else {
			return;
		};
		// Two bodies without mass cannot exchange an impulse, so their contact would change nothing.
		if a.inv_mass + b.inv_mass != 0.0 {
			intersect((a, first), (b, second), dt, contacts, scratch);
		}
	});
}

/// Publishes where `body` is.
fn publish(body: &PhysicsBody, transforms_tx: &impl Channel<TransformationUpdate>) {
	transforms_tx.send(TransformationUpdate::new(
		body.handle,
		Transform::new(body.position, body.scale, body.orientation),
	));
}

/// The `Share` struct holds what one lane produces from its share of a stage until a serial stage collects it.
///
/// The storage is kept between steps so lanes do not allocate.
#[derive(Default)]
struct Share {
	/// The contacts found in a share of the swept bounds.
	contacts: Vec<Contact>,
	/// Working storage for finding those contacts.
	scratch: ContactScratch,
	/// Where the bodies in a share of the body slots end the step, with the slot of each.
	motions: Vec<(usize, Motion)>,
	/// What solving the lane's share of each shared batch needs, batch after batch.
	groups: Vec<WideContact>,
	/// Where each shared batch's groups end in `groups`.
	batch_ends: Vec<usize>,
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
			rotates: false,
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
			rotates: true,
			handle: test_handle(),
		}
	}

	/// Returns the contacts one step of `dt` seconds finds between the world's bodies.
	fn detected_contacts(world: &mut World, dt: f32) -> Vec<Contact> {
		world.sweep_bodies(MediaTime::from_seconds_f32(dt));
		let mut contacts = Vec::new();
		detect_contacts(
			&world.bounds,
			&world.bodies,
			0..world.bounds.count(),
			dt,
			&mut contacts,
			&mut ContactScratch::default(),
		);
		contacts
	}

	/// Steps a world of `bodies` for `seconds` at sixty steps a second and returns it.
	fn stepped(bodies: Vec<PhysicsBody>, seconds: f32) -> World {
		let mut world = make_world();
		world.bodies = bodies.into_iter().collect();
		let transforms = DefaultChannel::new();
		for _ in 0..(seconds * 60.0).round() as usize {
			world.step(MediaTime::from_seconds_f32(1.0 / 60.0), &transforms);
		}
		world
	}

	/// Returns how far the first two bodies of `world` overlap, or zero when they do not touch.
	fn overlap(world: &World) -> f32 {
		let mut contacts = Vec::new();
		intersect(
			(world.bodies.get_slot(0).expect("first test body"), 0),
			(world.bodies.get_slot(1).expect("second test body"), 1),
			0.0,
			&mut contacts,
			&mut ContactScratch::default(),
		);
		contacts.iter().map(|contact| contact.depth).fold(0.0, f32::max)
	}

	/// Returns a dynamic one-meter box with friction.
	fn make_dynamic_box_body(position: math::Point, linear_velocity: Vector) -> PhysicsBody {
		let mut body = make_dynamic_sphere_body(position, linear_velocity, 0.5);
		body.collision_shape = Shapes::Cube {
			size: Vector::new(0.5, 0.5, 0.5),
		};
		body.friction = 0.5;
		body.rotates = false;
		body
	}

	/// The radius of the capsule [`make_dynamic_character_body`] returns.
	const CHARACTER_RADIUS: f32 = 0.3;

	/// Returns a dynamic capsule 1.8 meters tall, as a character would use, with its origin at its feet.
	fn make_dynamic_character_body(position: math::Point, linear_velocity: Vector) -> PhysicsBody {
		let middle = math::Point::new(0.0, 0.9, 0.0);
		let mut body = make_dynamic_sphere_body(position, linear_velocity, CHARACTER_RADIUS);
		body.collision_shape = Shapes::capsule(CHARACTER_RADIUS, 1.8, middle);
		body.center_of_mass = middle;
		body.rotates = false;
		body
	}

	#[test]
	fn a_character_capsule_dropped_on_the_ground_comes_to_rest_standing_on_its_feet() {
		// The ground's top is one unit up, so the character's feet start half a unit above it.
		let mut character = make_dynamic_character_body(math::Point::new(0.0, 1.5, 0.0), Vector::zero());
		character.friction = 0.5;
		let mut ground = make_ground_body();
		ground.friction = 0.5;
		let world = stepped(vec![ground, character], 3.0);

		let character = world.bodies.get_slot(1).unwrap();
		assert!(
			(character.position.y() - 1.0).abs() <= 0.01,
			"the character rests at {}",
			character.position.y()
		);
		assert!(character.position.x().abs() <= 1e-4 && character.position.z().abs() <= 1e-4);
		assert!(character.linear_velocity.length() <= 0.01);
		// Contacts do not turn a capsule, so the character cannot fall over.
		assert_eq!(character.orientation, Orientation::identity());
		assert!(character.angular_velocity.length() <= 1e-6);
	}

	#[test]
	fn a_character_capsule_walking_into_a_wall_stops_against_it() {
		// A wall that stands on the ground, with its near face two and a half units ahead of the character.
		let mut wall = make_ground_body();
		wall.collision_shape = Shapes::Cube {
			size: Vector::new(0.5, 2.0, 2.0),
		};
		wall.position = math::Point::new(3.0, 3.0, 0.0);
		// Nothing has friction, so only the wall can stop the character.
		let character = make_dynamic_character_body(math::Point::new(0.0, 1.0, 0.0), Vector::new(3.0, 0.0, 0.0));
		let world = stepped(vec![make_ground_body(), wall, character], 2.0);

		let character = world.bodies.get_slot(2).unwrap();
		assert!(
			(character.position.x() - (2.5 - CHARACTER_RADIUS)).abs() <= 0.02,
			"the character stopped at {}",
			character.position.x()
		);
		assert!(character.linear_velocity.x().abs() <= 0.01);
		assert!((character.position.y() - 1.0).abs() <= 0.01);
	}

	#[test]
	fn character_capsules_that_walk_into_each_other_stop_side_by_side() {
		let first = make_dynamic_character_body(math::Point::new(-1.0, 1.0, 0.0), Vector::new(1.0, 0.0, 0.0));
		let second = make_dynamic_character_body(math::Point::new(1.0, 1.0, 0.0), Vector::new(-1.0, 0.0, 0.0));
		let world = stepped(vec![make_ground_body(), first, second], 2.0);

		let (first, second) = (world.bodies.get_slot(1).unwrap(), world.bodies.get_slot(2).unwrap());
		let gap = second.position.x() - first.position.x();
		assert!(
			(gap - 2.0 * CHARACTER_RADIUS).abs() <= 0.02,
			"the characters stopped {gap} apart"
		);
		// Equal masses without elasticity cancel each other's approach.
		assert!(first.linear_velocity.length() <= 0.01 && second.linear_velocity.length() <= 0.01);
	}

	#[test]
	fn a_sphere_that_rolls_into_a_standing_capsule_stops_against_it() {
		// A capsule that nothing moves, like a character that stands its ground.
		let mut character = make_dynamic_character_body(math::Point::new(3.0, 1.0, 0.0), Vector::zero());
		character.body_type = BodyTypes::Static;
		character.inv_mass = 0.0;
		let ball = make_dynamic_sphere_body(math::Point::new(0.0, 1.5, 0.0), Vector::new(2.0, 0.0, 0.0), 0.5);
		let world = stepped(vec![make_ground_body(), character, ball], 2.0);

		// The ball's center is as high as the capsule's side, so they touch when their centers are 0.8 apart.
		let ball = world.bodies.get_slot(2).unwrap();
		assert!(
			(ball.position.x() - (3.0 - CHARACTER_RADIUS - 0.5)).abs() <= 0.02,
			"the ball stopped at {}",
			ball.position.x()
		);
		assert!(ball.linear_velocity.x().abs() <= 0.01);
		assert_eq!(world.bodies.get_slot(1).unwrap().position, math::Point::new(3.0, 1.0, 0.0));
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
	fn a_sphere_sunk_into_the_ground_comes_to_rest_on_it_for_both_body_orders() {
		// The ground's top is one unit up, so the sphere starts a tenth of a unit inside it.
		let sphere = || make_dynamic_sphere_body(math::Point::new(0.0, 1.4, 0.0), Vector::zero(), 0.5);
		let ground_first = stepped(vec![make_ground_body(), sphere()], 2.0);
		let sphere_first = stepped(vec![sphere(), make_ground_body()], 2.0);

		// A resting body keeps a few millimeters of overlap, which holds it in contact from step to step.
		assert!(overlap(&ground_first) <= 0.01);
		assert!(overlap(&sphere_first) <= 0.01);
		assert!((ground_first.bodies.get_slot(1).unwrap().position.y() - 1.5).abs() <= 0.01);
		assert!((sphere_first.bodies.get_slot(0).unwrap().position.y() - 1.5).abs() <= 0.01);
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

		world.update_collisions(step);
		world.update_bodies(step, &transforms);

		let free_body = world.bodies.get_slot(2).expect("free test body");
		assert!((free_body.position - math::Point::new(20.0, 0.0, 1.0)).length() <= 1e-4);
	}

	/// The bodies along each side of the grid [`crowded_bodies`] returns.
	const CROWD_SIDE: usize = 16;

	/// Returns a ground body under a grid of spheres and boxes that overlap it and each other, so one step has
	/// enough contacts with the ground alone for lanes to share a batch.
	fn crowded_bodies() -> Vec<PhysicsBody> {
		const { assert!(CROWD_SIDE * CROWD_SIDE >= 2 * SHARED_BATCH_CONTACTS) };
		const SPACING: f32 = 0.8;
		let reach = (CROWD_SIDE - 1) as f32 * SPACING * 0.5;
		let mut ground = make_ground_body();
		ground.collision_shape = Shapes::Cube {
			size: Vector::new(reach + 2.0, 1.0, reach + 2.0),
		};
		let mut bodies = vec![ground];
		for index in 0..CROWD_SIDE * CROWD_SIDE {
			let (column, row) = ((index % CROWD_SIDE) as f32, (index / CROWD_SIDE) as f32);
			let position = math::Point::new(column * SPACING - reach, 1.4, row * SPACING - reach);
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
		// Every step runs on lanes, whatever the machine running the test would choose.
		world.set_step_lanes(StepLanes::new([(0, 4)]));
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
	fn fast_spheres_that_would_pass_through_each_other_meet_and_stop() {
		let mut world = make_world();
		world.bodies = [
			make_dynamic_sphere_body(math::Point::new(-2.0, 0.0, 0.0), Vector::new(4.0, 0.0, 0.0), 0.5),
			make_dynamic_sphere_body(math::Point::new(2.0, 0.0, 0.0), Vector::new(-4.0, 0.0, 0.0), 0.5),
		]
		.into_iter()
		.collect();
		let transforms = DefaultChannel::new();
		let step = MediaTime::from_seconds_f32(1.0);

		// The spheres would close their three-meter gap in under half the step and end it past each other. Their
		// contact slows them so that they end it touching.
		world.update_collisions(step);
		world.update_bodies(step, &transforms);
		let (first, second) = (world.bodies.get_slot(0).unwrap(), world.bodies.get_slot(1).unwrap());
		assert!((first.position - math::Point::new(-0.5, 0.0, 0.0)).length() <= 1e-3);
		assert!((second.position - math::Point::new(0.5, 0.0, 0.0)).length() <= 1e-3);

		// Once they touch, equal masses without elasticity cancel each other's approach.
		world.update_collisions(step);
		let (first, second) = (world.bodies.get_slot(0).unwrap(), world.bodies.get_slot(1).unwrap());
		assert!(first.linear_velocity.length() <= 1e-3);
		assert!(second.linear_velocity.length() <= 1e-3);
	}

	#[test]
	fn a_body_that_takes_over_a_slot_does_not_inherit_its_contacts() {
		let mut world = make_world();
		let transforms = DefaultChannel::new();
		let step = MediaTime::from_seconds_f32(1.0 / 60.0);
		// Boxes that do not rotate, so that the light one stands still under fifty times its weight within the
		// thirty steps it gets.
		let one_meter_box = || Shapes::cube(Vector::new(0.5, 0.5, 0.5));
		let dynamic = |shape| Body::new(BodyTypes::Dynamic, shape).with_rotation(false);
		let place = |world: &mut World, handle: Handle, body: Body, height: f32| {
			world.create_body(handle, body);
			let slot = world.handles_to_bodies[&handle];
			world.bodies[slot].position = math::Point::new(0.0, height, 0.0);
			slot
		};

		// A light box rests on the ground under a heavy one, until the contact between them carries the heavy one.
		let (bottom, top) = (test_handle(), test_handle());
		let ground = Body::new(BodyTypes::Static, Shapes::cube(Vector::new(4.0, 1.0, 4.0)));
		place(&mut world, test_handle(), ground, 0.0);
		let bottom_slot = place(&mut world, bottom, dynamic(one_meter_box()), 1.5);
		place(&mut world, top, dynamic(one_meter_box()).with_mass(50.0), 2.5);
		for _ in 0..30 {
			world.step(step, &transforms);
		}

		// A light box takes the heavy one's place, and its slot, resting on the box below as the heavy one did.
		let rest = world.bodies[bottom_slot].position.y() + 0.997;
		let top_slot = place(&mut world, top, dynamic(one_meter_box()), rest);
		world.step(step, &transforms);

		// Started from the heavy box's contact, the two light boxes would be thrown apart.
		assert!(world.bodies[bottom_slot].linear_velocity.length() <= 0.05);
		assert!(world.bodies[top_slot].linear_velocity.length() <= 0.05);
	}

	#[test]
	fn overlapping_bodies_that_are_parting_are_left_to_part() {
		let mut world = make_world();
		world.bodies = [
			make_dynamic_sphere_body(math::Point::origin(), Vector::new(-1.0, 0.0, 0.0), 1.0),
			make_dynamic_sphere_body(math::Point::new(1.5, 0.0, 0.0), Vector::new(1.0, 0.0, 0.0), 1.0),
		]
		.into_iter()
		.collect();

		world.update_collisions(MediaTime::from_seconds_f32(1.0 / 60.0));

		// The contact pushes the spheres apart a little faster and never pulls them back together.
		let (first, second) = (world.bodies.get_slot(0).unwrap(), world.bodies.get_slot(1).unwrap());
		assert!(second.linear_velocity.x() - first.linear_velocity.x() >= 2.0);
	}

	#[test]
	fn a_stack_of_boxes_stays_stacked() {
		let mut ground = make_ground_body();
		ground.friction = 0.5;
		let boxes = (0..6).map(|layer| make_dynamic_box_body(math::Point::new(0.0, 1.5 + layer as f32, 0.0), Vector::zero()));
		let world = stepped(std::iter::once(ground).chain(boxes).collect(), 5.0);

		for layer in 0..6 {
			let position = world.bodies.get_slot(layer + 1).unwrap().position;
			assert!(
				(position.y() - (1.5 + layer as f32)).abs() <= 0.05,
				"layer {layer} rests at {}",
				position.y()
			);
			assert!(position.x().abs() <= 0.01 && position.z().abs() <= 0.01);
		}
	}

	#[test]
	fn a_sliding_box_stops_where_friction_brings_it_to_rest() {
		let mut ground = make_ground_body();
		ground.collision_shape = Shapes::Cube {
			size: Vector::new(20.0, 1.0, 20.0),
		};
		ground.friction = 0.5;
		let sliding = make_dynamic_box_body(math::Point::new(0.0, 1.5, 0.0), Vector::new(5.0, 0.0, 0.0));
		let world = stepped(vec![ground, sliding], 3.0);

		// Friction of a quarter of the box's weight under gravity of 16 slows it by 4 units per second squared, which
		// stops it from 5 units per second after 3.125 units.
		let body = world.bodies.get_slot(1).unwrap();
		assert!(body.linear_velocity.length() <= 0.01);
		assert!(
			(body.position.x() - 3.125).abs() <= 0.15,
			"the box stopped at {}",
			body.position.x()
		);
		// A box does not turn, so it cannot roll on.
		assert!(body.angular_velocity.length() <= 1e-6);
	}
	/// Returns a hull collider with the corners of a box of these half sizes around `middle`.
	fn box_hull(half_sizes: [f32; 3], middle: [f32; 3]) -> Shapes {
		let mut corners = Vec::new();
		for x in [-1.0, 1.0] {
			for y in [-1.0, 1.0] {
				for z in [-1.0, 1.0] {
					corners.push(math::Point::new(
						middle[0] + x * half_sizes[0],
						middle[1] + y * half_sizes[1],
						middle[2] + z * half_sizes[2],
					));
				}
			}
		}
		Shapes::convex_hull(crate::physics::build_convex_hull(&corners).expect("a box encloses a volume"))
	}

	#[test]
	fn a_stack_of_hulls_stays_stacked_on_a_box() {
		let mut ground = make_ground_body();
		ground.friction = 0.5;
		let one_meter_hull = box_hull([0.5; 3], [0.0; 3]);
		let hulls = (0..4).map(|layer| {
			let mut body = make_dynamic_box_body(math::Point::new(0.0, 1.5 + layer as f32, 0.0), Vector::zero());
			body.collision_shape = one_meter_hull.clone();
			body
		});
		let world = stepped(std::iter::once(ground).chain(hulls).collect(), 5.0);

		for layer in 0..4 {
			let body = world.bodies.get_slot(layer + 1).unwrap();
			assert!(
				(body.position.y() - (1.5 + layer as f32)).abs() <= 0.05,
				"layer {layer} rests at {}",
				body.position.y()
			);
			assert!(body.position.x().abs() <= 0.01 && body.position.z().abs() <= 0.01);
			assert!(body.linear_velocity.length() <= 0.05);
		}
	}

	#[test]
	fn a_falling_hull_lands_on_every_other_shape() {
		// A wedge-like hull whose lowest side is flat, dropped from a little above each kind of static body.
		let hull = box_hull([0.5, 0.25, 0.5], [0.0, 0.25, 0.0]);
		let mut sphere = make_dynamic_sphere_body(math::Point::origin(), Vector::zero(), 1.0);
		sphere.body_type = BodyTypes::Static;
		sphere.inv_mass = 0.0;
		let mut capsule = make_ground_body();
		capsule.collision_shape = Shapes::capsule(0.5, 2.0, math::Point::origin());
		let mut static_hull = make_ground_body();
		static_hull.collision_shape = box_hull([4.0, 1.0, 4.0], [0.0; 3]);

		for (ground, top) in [(make_ground_body(), 1.0), (sphere, 1.0), (capsule, 1.0), (static_hull, 1.0)] {
			let mut falling = make_dynamic_box_body(math::Point::new(0.0, top + 0.5, 0.0), Vector::zero());
			falling.collision_shape = hull.clone();
			let shape = format!("{:?}", ground.collision_shape).chars().take(12).collect::<String>();
			let world = stepped(vec![ground, falling], 3.0);

			// The hull's origin is at its flat underside, which comes to rest on top of the static body.
			let body = world.bodies.get_slot(1).unwrap();
			assert!(
				(body.position.y() - top).abs() <= 0.05,
				"on {shape} the hull rests at {}",
				body.position.y()
			);
			assert!(body.linear_velocity.length() <= 0.05, "on {shape} the hull still moves");
		}
	}
	/// Returns a dynamic box of one meter that rotates.
	fn make_rotating_box_body(position: math::Point) -> PhysicsBody {
		let mut body = make_dynamic_box_body(position, Vector::zero());
		body.rotates = true;
		body
	}

	/// Returns how nearly one of a body's own axes points straight up or down: one when one does.
	fn uprightness(body: &PhysicsBody) -> f32 {
		[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
			.map(|axis| {
				body.orientation
					.rotate_vector(Vector::<math::WorldSpace>::from_array(axis))
					.y()
					.abs()
			})
			.into_iter()
			.fold(0.0, f32::max)
	}

	#[test]
	fn a_rotating_box_dropped_on_an_edge_comes_to_rest_on_a_side() {
		let mut ground = make_ground_body();
		ground.friction = 0.5;
		let mut tilted = make_rotating_box_body(math::Point::new(0.0, 2.0, 0.0));
		tilted.orientation = Orientation::try_from_rotation_vector(Vector::<math::WorldSpace>::new(0.0, 0.0, 0.6)).unwrap();
		let world = stepped(vec![ground, tilted], 3.0);

		let body = world.bodies.get_slot(1).unwrap();
		assert!(uprightness(body) >= 0.9999, "the box lies {} from level", uprightness(body));
		assert!(
			(body.position.y() - 1.5).abs() <= 0.01,
			"the box rests at {}",
			body.position.y()
		);
		assert!(body.linear_velocity.length() <= 0.01 && body.angular_velocity.length() <= 0.01);
	}

	#[test]
	fn a_stack_of_rotating_boxes_stays_stacked() {
		let mut ground = make_ground_body();
		ground.friction = 0.5;
		let boxes = (0..4).map(|layer| make_rotating_box_body(math::Point::new(0.0, 1.5 + layer as f32, 0.0)));
		let world = stepped(std::iter::once(ground).chain(boxes).collect(), 5.0);

		for layer in 0..4 {
			let body = world.bodies.get_slot(layer + 1).unwrap();
			assert!(
				(body.position.y() - (1.5 + layer as f32)).abs() <= 0.05,
				"layer {layer} rests at {}",
				body.position.y()
			);
			assert!(body.position.x().abs() <= 0.05 && body.position.z().abs() <= 0.05);
			assert!(uprightness(body) >= 0.999);
			assert!(
				body.linear_velocity.length() <= 0.1,
				"layer {layer} moves at {}",
				body.linear_velocity.length()
			);
		}
	}

	#[test]
	fn a_rotating_box_slides_to_a_stop_without_tumbling() {
		let mut ground = make_ground_body();
		ground.collision_shape = Shapes::Cube {
			size: Vector::new(20.0, 1.0, 20.0),
		};
		ground.friction = 0.5;
		let mut sliding = make_rotating_box_body(math::Point::new(0.0, 1.5, 0.0));
		sliding.linear_velocity = Vector::new(5.0, 0.0, 0.0);
		let world = stepped(vec![ground, sliding], 3.0);

		// Friction stops it after about 3.125 units, as it does a box that cannot rotate.
		let body = world.bodies.get_slot(1).unwrap();
		assert!(
			(body.position.x() - 3.125).abs() <= 0.15,
			"the box stopped at {}",
			body.position.x()
		);
		assert!(uprightness(body) >= 0.9999);
		assert!(body.linear_velocity.length() <= 0.01 && body.angular_velocity.length() <= 0.01);
	}

	#[test]
	fn a_character_that_does_not_rotate_stays_upright_when_it_is_hit() {
		// A heavy ball hits each capsule at head height. Only the one that rotates is knocked over.
		let struck = |rotates: bool| {
			let mut character = make_dynamic_character_body(math::Point::new(0.0, 1.0, 0.0), Vector::zero());
			character.rotates = rotates;
			let mut ball = make_dynamic_sphere_body(math::Point::new(-1.5, 2.6, 0.0), Vector::new(6.0, 0.0, 0.0), 0.3);
			ball.inv_mass = 0.2;
			let world = stepped(vec![make_ground_body(), character, ball], 0.5);
			let character = world.bodies.get_slot(1).unwrap();
			// How nearly the capsule's own length still points straight up: one when it does.
			let standing = character
				.orientation
				.rotate_vector(Vector::<math::WorldSpace>::new(0.0, 1.0, 0.0))
				.y();
			(standing, character.angular_velocity.length())
		};

		assert_eq!(struck(false), (1.0, 0.0));
		let (standing, _) = struck(true);
		assert!(standing < 0.9, "the capsule that rotates still stands, at {standing}");
	}

	#[test]
	fn a_sphere_rolls_down_a_floor_that_is_turned() {
		// The ground is static, and turned so that its top slopes down toward -x.
		let mut ground = make_ground_body();
		ground.rotates = true;
		ground.friction = 0.5;
		ground.orientation = Orientation::try_from_rotation_vector(Vector::<math::WorldSpace>::new(0.0, 0.0, 0.2)).unwrap();
		let mut sphere = make_dynamic_sphere_body(math::Point::new(0.0, 2.0, 0.0), Vector::zero(), 0.5);
		sphere.friction = 0.5;
		let world = stepped(vec![ground, sphere], 1.0);

		let sphere = world.bodies.get_slot(1).unwrap();
		assert!(
			sphere.position.x() < -0.3 && sphere.linear_velocity.x() < -0.5,
			"the sphere is at {:?}",
			sphere.position
		);
		// It is still on the slope, which at its place is lower than where the level top would be.
		assert!(
			sphere.position.y() < 1.5 && sphere.position.y() > 0.5,
			"the sphere is at {:?}",
			sphere.position
		);
	}

	#[test]
	fn a_rotating_hull_dropped_on_an_edge_comes_to_rest_on_a_side() {
		let mut ground = make_ground_body();
		ground.friction = 0.5;
		let mut tilted = make_rotating_box_body(math::Point::new(0.0, 2.0, 0.0));
		tilted.collision_shape = box_hull([0.5; 3], [0.0; 3]);
		tilted.orientation = Orientation::try_from_rotation_vector(Vector::<math::WorldSpace>::new(0.3, 0.0, 0.5)).unwrap();
		let world = stepped(vec![ground, tilted], 3.0);

		let body = world.bodies.get_slot(1).unwrap();
		assert!(uprightness(body) >= 0.9999, "the hull lies {} from level", uprightness(body));
		assert!(
			(body.position.y() - 1.5).abs() <= 0.01,
			"the hull rests at {}",
			body.position.y()
		);
		assert!(body.linear_velocity.length() <= 0.02 && body.angular_velocity.length() <= 0.02);
	}
}
