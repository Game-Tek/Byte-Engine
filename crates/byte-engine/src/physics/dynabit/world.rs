use std::{ops::Range, time::Instant};

use math::{AABB, Scale, Vector};
use utils::{StableVec, StableVecHandle, hash::HashMap};

use crate::{
	application::Time,
	core::{
		alley::{Alley, ConcreteLane, Lane as _, LaneOwned, LaneRanges, Stage},
		channel::Channel,
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
	},
	gameplay::transform::{Transform, TransformationUpdate},
	physics::{
		body::{Body, BodyTypes},
		constraint::Constraint,
		dynabit::{
			body::{PhysicsBody, intersect},
			constraint::Constraints,
			contact::{ContactPart, Contacts},
			solver::{self, ContactSolver, WideContact},
			step_lanes::StepLanes,
			work_shares::WorkShares,
		},
		epa::ContactScratch,
		intersection::SweepBounds,
	},
	time::MediaTime,
};

/// The fewest contacts a batch needs before [`World::update`] shares it between lanes.
///
/// Sharing a batch costs one more stage in every pass over the contacts, which every lane has to meet at. That is
/// more than a very small batch takes to solve on one lane. On an Apple M4, piles of 1,000 to 16,000 mixed boxes and
/// spheres stepped fastest with batches shared from about this size, and up to a third slower with none shared.
const SHARED_BATCH_CONTACTS: usize = 32;

/// The `World` struct owns Dynabit simulation state and synchronizes it with entity handles.
pub struct World {
	/// The bodies, by slot. Lanes change them side by side, each the bodies in its own range of the slots.
	bodies: LaneRanges<StableVec<PhysicsBody>>,
	gravity: Vector,
	body_listener: DefaultListener<CreateMessage<Body>>,
	constraint_listener: DefaultListener<CreateMessage<Constraint>>,
	/// Deletions of any entity. The world removes the body and the constraint that were created with the handle.
	delete_listener: DefaultListener<DeleteMessage>,
	handles_to_bodies: HashMap<Handle, StableVecHandle>,
	/// What pairs of bodies are held to, such as a distance between them.
	constraints: Constraints,
	/// The bounds every body sweeps through during the current step, which contact detection scans for overlaps.
	bounds: SweepBounds,
	/// What each lane produced from its share of a stage, until one lane collects it.
	shares: LaneOwned<Share>,
	/// How many lanes [`Self::update`] runs a step on.
	step_lanes: StepLanes,
	/// How the lanes split the swept bounds to find contacts, by how long each share took in the last step.
	detection: WorkShares,
	/// The step's contacts in the order they were found, kept between steps so gathering them does not allocate.
	contacts: Contacts,
	/// Solves the step's contacts.
	solver: ContactSolver,
}

impl World {
	/// Creates a Dynabit world that listens for the bodies and constraints that are created, and for the handles
	/// that are deleted.
	///
	/// Next, create [`Body`] and [`Constraint`] values through the factories the listeners come from, and call
	/// [`Self::update`] every step.
	pub fn new(
		body_listener: DefaultListener<CreateMessage<Body>>,
		constraint_listener: DefaultListener<CreateMessage<Constraint>>,
		delete_listener: DefaultListener<DeleteMessage>,
	) -> Self {
		Self {
			bodies: LaneRanges::new(StableVec::new()),
			gravity: Vector::new(0.0, -16.0, 0.0),
			body_listener,
			constraint_listener,
			delete_listener,
			handles_to_bodies: HashMap::with_capacity_and_hasher(1024, Default::default()),
			constraints: Constraints::default(),
			bounds: SweepBounds::default(),
			shares: LaneOwned::default(),
			step_lanes: StepLanes::for_machine(),
			detection: WorkShares::default(),
			contacts: Contacts::default(),
			solver: ContactSolver::default(),
		}
	}

	/// Replaces how many lanes [`Self::update`] runs a step on, which starts as
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
		let Some(body) = self.bodies.get_mut().get_mut(index) else {
			return false;
		};

		body.apply_linear_impulse(impulse);
		true
	}

	/// Processes entity updates, advances physics as one execution of `alley`, and publishes resulting transforms.
	///
	/// The world's [`StepLanes`] decide from the number of bodies how many of `alley`'s lanes run the step; the
	/// other lanes stay asleep. A step they give one lane runs on the calling thread, because waking a lane would
	/// cost more than it saves. Every running lane runs the same step:
	///
	/// 1. Every lane applies gravity to its share of the bodies, gives the solver what it needs of them, and works
	///    out the bounds they sweep through.
	/// 2. One lane gathers the bounds while the others wait, every lane repairs its part of their order, one lane
	///    splits the order into the parts that every lane then merges, one lane lays out grid cells over the bounds,
	///    and every lane lists its part of the bounds in those cells.
	/// 3. Every lane finds the contacts in its share of the bounds, and one lane batches them. That lane also gets
	///    the constraints ready.
	/// 4. The lanes prepare the contacts and pass over them several times, sharing every batch that is large
	///    enough, one batch to a stage. One lane takes the small ones. Before each pass, one lane solves the
	///    constraints in turn, so that the contacts have the last word on where bodies may be. After the last pass
	///    it keeps what the constraints start the next step from.
	/// 5. Every lane keeps the impulses of its contacts for the next step, gives its share of the bodies their new
	///    velocities, moves them, and publishes their transforms.
	///
	/// Contacts are prepared and solved in the same order whatever the number of lanes, so the simulation does not
	/// depend on it. The order transforms are published in does: each lane publishes the bodies of its share in slot
	/// order, and the lanes' runs follow each other as they finish.
	pub fn update(
		&mut self,
		time: Time,
		transforms_rx: &mut impl Listener<TransformationUpdate>,
		transforms_tx: &(impl Channel<TransformationUpdate> + Sync),
		alley: &mut Alley,
	) {
		// Listeners belong to the calling thread, so messages are adopted before the lanes start.
		self.adopt_updates(transforms_rx);

		let step = time.delta();
		let bodies = self.bodies.get_mut();
		let (count, slots) = (bodies.len(), bodies.slots_len());
		let lanes = self.step_lanes.lanes(count).min(alley.parallelism());
		// What lanes write side by side is lent to this alley's lanes, each lane its own part.
		self.shares.lend(alley.id());
		self.shares.grow(lanes);
		self.bodies.lend(alley.id());
		self.bodies.split(lanes);
		self.bounds.lend(alley.id());
		self.solver.reserve(slots, alley.id(), lanes);
		// A world without constraints, as most are, has no stage for them.
		let constrained = !self.constraints.is_empty();
		let executed = alley.execute_phased(self, lanes, |lane, world| {
			let _ = lane.each_ref(world, |world, lane| world.start_share(lane, step));
			lane.broadcast_mut(world, |world| world.stage_shares(lanes));
			let _ = lane.each_ref(world, |world, lane| world.bounds.repair(lane));
			lane.broadcast_mut(world, |world| world.bounds.share_order());
			let _ = lane.each_ref(world, |world, lane| world.bounds.merge(lane));
			let in_cells = lane.broadcast_mut(world, |world| world.bounds.lay_out(lanes));
			if in_cells {
				let _ = lane.each_ref(world, |world, lane| world.bounds.fill(lane));
				lane.broadcast_mut(world, |world| world.bounds.share_cells());
			}
			let _ = lane.each_ref(world, |world, lane| world.detect_share_contacts(lane, lanes, step));
			let (shared_batches, rest) = lane.broadcast_mut(world, |world| {
				let batches = world.batch_share_contacts(lanes);
				world.pre_solve_constraints();
				batches
			});
			// Every lane works on one batch at a time, which the stage's key says.
			for batch in 0..shared_batches {
				let _ = lane.each_ref_keyed(world, batch, |world, stage| world.prepare_share(stage, step));
			}
			if rest {
				lane.broadcast_mut(world, |world| world.prepare_rest(shared_batches, step));
			}
			for _ in 0..solver::ITERATIONS {
				if constrained {
					lane.broadcast_mut(world, |world| world.constraints.solve(&mut world.solver));
				}
				for batch in 0..shared_batches {
					let _ = lane.each_ref_keyed(world, batch, |world, stage| world.solve_share(stage));
				}
				if rest {
					lane.broadcast_mut(world, |world| world.solver.solve_rest());
				}
			}
			lane.broadcast_mut(world, |world| {
				world.solver.end_passes();
				world.constraints.post_solve();
			});
			let _ = lane.each_ref(world, |world, lane| world.finish_share(lane, step, transforms_tx));
		});
		// Between steps the bodies are read as a whole.
		self.bodies.join();
		if let Err(panic) = executed {
			std::panic::resume_unwind(panic);
		}
	}

	/// Registers created bodies and constraints, and applies authored transforms to the bodies they target.
	fn adopt_updates(&mut self, transforms_rx: &mut impl Listener<TransformationUpdate>) {
		while let Some(message) = self.body_listener.read() {
			let handle = message.handle();
			let body = message.into_data();
			self.create_body(handle, body);
		}
		// Like bodies, creation messages are upserts, so one handle always owns at most one constraint.
		while let Some(message) = self.constraint_listener.read() {
			let handle = message.handle();
			self.constraints.insert(handle, message.into_data());
		}
		while let Some(message) = transforms_rx.read() {
			if let Some(index) = self.handles_to_bodies.get(&message.handle()).copied() {
				let body = &mut self.bodies.get_mut()[index];
				body.position = message.transform().get_position();
				body.orientation = message.transform().get_orientation();
				body.scale = message.transform().scale();
			}
		}
	}

	/// Starts the step for the bodies in one lane's share of the body slots: applies gravity, gives the solver what
	/// the step's contacts need of each body, and keeps the bounds each sweeps through in that lane's storage.
	fn start_share(&self, lane: &ConcreteLane<'_>, dt: MediaTime) {
		let mut share = self.shares.own(lane);
		let mut taken = self.solver.take_bodies(lane);
		let seconds = dt.as_seconds_f32();
		for (slot, body) in self.bodies.own(lane).items() {
			if body.body_type == BodyTypes::Dynamic {
				body.apply_linear_impulse(self.gravity * body.mass() * seconds);
			}
			taken.take(slot, body);
			share.bounds.push((slot, body.swept_bounds(seconds)));
		}
	}

	/// Gathers the bounds that [`Self::start_share`] kept, and leaves their order to be repaired as `parts` parts,
	/// with [`SweepBounds::repair`]. From here until the contacts are batched, every lane reads every body.
	///
	/// Shares are consecutive ranges of the body slots, so taking them in lane order gives the bounds in slot order
	/// whatever the number of lanes.
	fn stage_shares(&mut self, parts: usize) {
		let bounds = self.shares.iter_mut().flat_map(|share| share.bounds.drain(..));
		self.bounds.stage(bounds, parts);
		self.bodies.join();
	}

	/// Finds the contacts in the share of the swept bounds that one of `lanes` lanes has, and keeps them in that
	/// lane's storage.
	///
	/// Bodies in a dense part of a pile, or with shapes that cost more to test, take longer than others. The shares
	/// are sized by how long each took in the last step, so that no lane keeps the others waiting. The contacts come
	/// out in the same order whatever the shares are.
	fn detect_share_contacts(&self, lane: &ConcreteLane<'_>, lanes: usize, dt: MediaTime) {
		let mut share = self.shares.own(lane);
		let share = &mut *share;
		// The part holds what an earlier step found.
		share.contacts.clear();
		let range = self.detection.range(lane.idx(), lanes, self.bounds.count());
		let positions = range.len();
		let started = Instant::now();
		detect_contacts(
			&self.bounds,
			self.bodies.all(),
			range,
			dt.as_seconds_f32(),
			&mut share.contacts,
			&mut share.scratch,
		);
		share.detection = (positions, started.elapsed().as_nanos() as u64);
	}

	/// Gathers the contacts found in every share and sorts them into batches. Returns how many batches lanes should
	/// share, and whether any contacts are left for one lane to prepare and solve. From here on, each of `lanes`
	/// lanes has its own bodies again.
	fn batch_share_contacts(&mut self, lanes: usize) -> (u32, bool) {
		// Shares are consecutive ranges of the swept bounds, so taking them in lane order keeps the order one scan
		// finds. Each share gets the storage its contacts had in an earlier step.
		for (lane, share) in self.shares.iter_mut().take(lanes).enumerate() {
			std::mem::swap(self.contacts.part_mut(lane), &mut share.contacts);
		}
		self.contacts.clear_from(lanes);
		// A lane that did not run this step leaves nothing, which changes no share.
		let taken = self.shares.iter_mut().map(|share| std::mem::take(&mut share.detection));
		self.detection.record(taken);
		self.contacts.seal();
		self.solver.begin(&self.contacts, lanes, !self.constraints.is_empty());
		self.bodies.split(lanes);
		// Batches fill from the first one, so the large ones lead.
		let shared = (0..self.solver.batches())
			.take_while(|batch| self.solver.batch_contacts(*batch) >= SHARED_BATCH_CONTACTS)
			.count();
		(shared as u32, self.solver.has_rest(shared))
	}

	/// Prepares one lane's share of the groups of the stage's batch and keeps them in that lane's storage, after the
	/// groups of the batches before it.
	fn prepare_share(&self, stage: &Stage<'_, '_>, dt: MediaTime) {
		let mut share = self.shares.own(stage.lane());
		self.solver
			.prepare_batch(&self.contacts, dt.as_seconds_f32(), stage, &mut share.groups);
		let end = share.groups.len();
		share.batch_ends.push(end);
	}

	/// Gets the constraints ready for the step, once [`Self::batch_share_contacts`] has begun the solver's.
	fn pre_solve_constraints(&mut self) {
		// Constraints name their bodies by handle, so they hold whichever body has the handle in this step.
		let (bodies, slots) = (&*self.bodies.get_mut(), &self.handles_to_bodies);
		let body = |handle| {
			let index = *slots.get(&handle)?;
			Some((index.index(), bodies.get(index)?))
		};
		self.constraints.pre_solve(body, &mut self.solver);
	}

	/// Prepares the contacts that follow the first `shared_batches` batches, which one lane then passes over in turn.
	fn prepare_rest(&mut self, shared_batches: u32, dt: MediaTime) {
		self.solver
			.prepare_rest(&self.contacts, dt.as_seconds_f32(), shared_batches as usize);
	}

	/// Passes once over the groups one lane prepared of the stage's batch.
	fn solve_share(&self, stage: &Stage<'_, '_>) {
		let share = self.shares.own(stage.lane());
		let batch = stage.key() as usize;
		let start = batch.checked_sub(1).map_or(0, |previous| share.batch_ends[previous]);
		self.solver.solve(stage, &share.groups[start..share.batch_ends[batch]]);
	}

	/// Ends the step for one lane's share of the work that follows the passes over the contacts.
	///
	/// The lane keeps the impulses of the contacts it prepared for the next step to start from. It then gives the
	/// bodies in its share of the body slots the velocities the passes left them with, moves the dynamic ones to
	/// where they are after `dt`, and publishes their transforms to `transforms_tx`, in slot order.
	fn finish_share(&self, lane: &ConcreteLane<'_>, dt: MediaTime, transforms_tx: &impl Channel<TransformationUpdate>) {
		let mut share = self.shares.own(lane);
		self.solver.keep_pushes(&share.groups);
		share.groups.clear();
		share.batch_ends.clear();
		// One lane prepared and solved the contacts that are in no shared batch, so one lane keeps their impulses.
		if lane.idx() == 0 {
			self.solver.keep_rest_pushes();
		}

		let solved = self.solver.solved();
		for (slot, body) in self.bodies.own(lane).items() {
			solved.give(slot, body);
			if body.body_type == BodyTypes::Dynamic {
				body.update(dt);
				let transform = Transform::new(body.position, body.scale, body.orientation);
				share.transforms.push(TransformationUpdate::new(body.handle, transform));
			}
		}
		// One run for the whole share, so that lanes do not contend for a place in the channel with every body.
		transforms_tx.send_many(share.transforms.drain(..));
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
		let index = self.bodies.get_mut().push(PhysicsBody {
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

	/// Removes the body and the constraint that were created with the handle of every pending deletion message.
	pub fn process_pending_deletions(&mut self) {
		while let Some(message) = self.delete_listener.read() {
			let handle = message.into_handle();
			self.remove_body(handle);
			self.constraints.remove(handle);
		}
	}

	/// Removes and returns the physics body for `handle`.
	pub fn remove_body(&mut self, handle: Handle) -> Option<PhysicsBody> {
		self.handles_to_bodies
			.remove(&handle)
			.and_then(|index| self.bodies.get_mut().remove(index))
	}
}

/// Adds to `found` the contacts between each body in `range` of the swept `bounds` and the bodies after it.
///
/// Ranges that cover every scan position find each contact once, in the same order whatever the ranges are.
/// `scratch` is working storage that only this call may use while it runs.
fn detect_contacts(
	bounds: &SweepBounds,
	bodies: &StableVec<PhysicsBody>,
	range: Range<usize>,
	dt: f32,
	found: &mut ContactPart,
	scratch: &mut ContactScratch,
) {
	bounds.overlaps(range, |[first, second]| {
		let (Some(a), Some(b)) = (bodies.get_slot(first), bodies.get_slot(second)) else {
			return;
		};
		// Two bodies without mass cannot exchange an impulse, so their contact would change nothing.
		if a.inv_mass + b.inv_mass != 0.0 {
			// A body without mass is never changed by a contact.
			let bodies = || [(a, first), (b, second)].map(|(body, slot)| (slot, body.inv_mass != 0.0));
			found.find(|contacts| intersect((a, first), (b, second), dt, contacts, scratch), bodies);
		}
	});
}

/// The `Share` struct holds what one lane produces from its share of a stage until one lane collects it.
///
/// The storage is kept between steps so lanes do not allocate.
#[derive(Default)]
struct Share {
	/// The contacts found in a share of the swept bounds.
	contacts: ContactPart,
	/// Working storage for finding those contacts.
	scratch: ContactScratch,
	/// How many scan positions of the swept bounds that share had, and how long it took, in nanoseconds.
	detection: (usize, u64),
	/// The bounds that the bodies in a share of the body slots sweep through during the step, with the slot of each.
	bounds: Vec<(usize, AABB)>,
	/// Where the dynamic bodies in a share of the body slots end the step, between working it out and publishing it.
	transforms: Vec<TransformationUpdate>,
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
		physics::{collider::Shapes, dynabit::contact::Contact},
	};

	fn test_handle() -> Handle {
		let factory = Factory::<()>::new();
		factory.create(())
	}

	fn make_world() -> World {
		let body_factory = Factory::<Body>::new();
		let delete_channel = DefaultChannel::new();
		World::new(
			body_factory.listener(),
			Factory::<Constraint>::new().listener(),
			delete_channel.listener(),
		)
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
		let bounds = world.bodies.all().indexed_iter();
		world.bounds.rebuild(
			bounds.map(|(slot, body)| (slot, body.swept_bounds(dt))),
			&mut Alley::with_parallelism(1),
		);
		let mut found = ContactPart::default();
		detect_contacts(
			&world.bounds,
			world.bodies.all(),
			0..world.bounds.count(),
			dt,
			&mut found,
			&mut ContactScratch::default(),
		);
		found.into_contacts()
	}

	/// Advances `world` by `dt` `times` times on one lane, publishing to `transforms`.
	fn step(world: &mut World, dt: MediaTime, times: usize, transforms: &DefaultChannel<TransformationUpdate>) {
		let mut alley = Alley::with_parallelism(1);
		// Nothing authors transforms for the bodies of a test.
		let mut authored = DefaultChannel::new().listener();
		for _ in 0..times {
			world.update(Time::new(dt, dt), &mut authored, transforms, &mut alley);
		}
	}

	/// Steps a world of `bodies` for `seconds` at sixty steps a second and returns it.
	fn stepped(bodies: Vec<PhysicsBody>, seconds: f32) -> World {
		let mut world = make_world();
		*world.bodies.get_mut() = bodies.into_iter().collect();
		let steps = (seconds * 60.0).round() as usize;
		step(
			&mut world,
			MediaTime::from_seconds_f32(1.0 / 60.0),
			steps,
			&DefaultChannel::new(),
		);
		world
	}

	/// Returns how far the first two bodies of `world` overlap, or zero when they do not touch.
	fn overlap(world: &World) -> f32 {
		let mut contacts = Vec::new();
		intersect(
			(world.bodies.all().get_slot(0).expect("first test body"), 0),
			(world.bodies.all().get_slot(1).expect("second test body"), 1),
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

		let character = world.bodies.all().get_slot(1).unwrap();
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

		let character = world.bodies.all().get_slot(2).unwrap();
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

		let (first, second) = (
			world.bodies.all().get_slot(1).unwrap(),
			world.bodies.all().get_slot(2).unwrap(),
		);
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
		let ball = world.bodies.all().get_slot(2).unwrap();
		assert!(
			(ball.position.x() - (3.0 - CHARACTER_RADIUS - 0.5)).abs() <= 0.02,
			"the ball stopped at {}",
			ball.position.x()
		);
		assert!(ball.linear_velocity.x().abs() <= 0.01);
		assert_eq!(
			world.bodies.all().get_slot(1).unwrap().position,
			math::Point::new(3.0, 1.0, 0.0)
		);
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

		assert_eq!(world.bodies.all().len(), 1);
		let index = world.handles_to_bodies[&handle];
		assert_eq!(world.bodies.all()[index].position, math::Point::origin());
		assert_eq!(world.bodies.all()[index].linear_velocity, replacement_velocity);
		assert!(matches!(
			&world.bodies.all()[index].collision_shape,
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
		world.bodies.get_mut().push(body);
		let transforms = DefaultChannel::new();
		let mut transforms_rx = transforms.listener();

		step(&mut world, MediaTime::ZERO, 1, &transforms);

		let update = transforms_rx.read().expect("dynamic body transform update");
		assert_eq!(update.handle(), handle);
		assert_eq!(update.transform().scale(), expected_scale);
	}

	#[test]
	fn detects_each_pair_once() {
		let mut world = make_world();
		*world.bodies.get_mut() = [
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
		assert!((ground_first.bodies.all().get_slot(1).unwrap().position.y() - 1.5).abs() <= 0.01);
		assert!((sphere_first.bodies.all().get_slot(0).unwrap().position.y() - 1.5).abs() <= 0.01);
	}

	#[test]
	fn bodies_outside_contacts_reach_the_end_of_the_step() {
		let mut world = make_world();
		world.gravity = Vector::zero();
		let velocity = Vector::new(0.0, 0.0, 2.0);
		*world.bodies.get_mut() = [
			make_ground_body(),
			make_dynamic_sphere_body(math::Point::new(0.0, 1.4, 0.0), Vector::zero(), 0.5),
			make_dynamic_sphere_body(math::Point::new(20.0, 0.0, 0.0), velocity, 0.5),
		]
		.into_iter()
		.collect();
		step(&mut world, MediaTime::from_seconds_f32(0.5), 1, &DefaultChannel::new());

		let free_body = world.bodies.all().get_slot(2).expect("free test body");
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

	/// Steps a world of [`crowded_bodies`] and returns every transform it published, step after step. The transforms
	/// of one step are sorted, because lanes publish their shares of a step in the order they finish.
	fn published_positions(
		mut step: impl FnMut(&mut World, Time, &mut DefaultListener<TransformationUpdate>, &DefaultChannel<TransformationUpdate>),
	) -> Vec<(math::Point, Orientation)> {
		let mut world = make_world();
		*world.bodies.get_mut() = crowded_bodies().into_iter().collect();
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
			let first = positions.len();
			while let Some(update) = published_rx.read() {
				positions.push((update.transform().get_position(), update.transform().get_orientation()));
			}
			// No two bodies of the crowd are at the same place.
			let place = |(position, _): &(math::Point, Orientation)| position.to_array().map(f32::to_bits);
			positions[first..].sort_unstable_by_key(place);
		}
		positions
	}

	#[test]
	fn a_step_publishes_the_same_transforms_on_any_number_of_lanes() {
		let mut alley = Alley::with_parallelism(4);
		// Every step runs on the lanes it is given here, whatever the machine running the test would choose.
		let mut on = |lanes: usize| {
			published_positions(|world, time, authored, published| {
				world.set_step_lanes(StepLanes::new([(0, lanes)]));
				world.update(time, authored, published, &mut alley);
			})
		};
		let on_one = on(1);

		assert_eq!(on_one.len(), 30 * CROWD_SIDE * CROWD_SIDE);
		for lanes in [2, 3, 4] {
			assert_eq!(on(lanes), on_one, "on {lanes} lanes");
		}
	}

	#[test]
	fn fast_spheres_that_would_pass_through_each_other_meet_and_stop() {
		let mut world = make_world();
		world.gravity = Vector::zero();
		*world.bodies.get_mut() = [
			make_dynamic_sphere_body(math::Point::new(-2.0, 0.0, 0.0), Vector::new(4.0, 0.0, 0.0), 0.5),
			make_dynamic_sphere_body(math::Point::new(2.0, 0.0, 0.0), Vector::new(-4.0, 0.0, 0.0), 0.5),
		]
		.into_iter()
		.collect();
		let transforms = DefaultChannel::new();
		let one_second = MediaTime::from_seconds_f32(1.0);

		// The spheres would close their three-meter gap in under half the step and end it past each other. Their
		// contact slows them so that they end it touching.
		step(&mut world, one_second, 1, &transforms);
		let (first, second) = (
			world.bodies.all().get_slot(0).unwrap(),
			world.bodies.all().get_slot(1).unwrap(),
		);
		assert!((first.position - math::Point::new(-0.5, 0.0, 0.0)).length() <= 1e-3);
		assert!((second.position - math::Point::new(0.5, 0.0, 0.0)).length() <= 1e-3);

		// Once they touch, equal masses without elasticity cancel each other's approach.
		step(&mut world, one_second, 1, &transforms);
		let (first, second) = (
			world.bodies.all().get_slot(0).unwrap(),
			world.bodies.all().get_slot(1).unwrap(),
		);
		assert!(first.linear_velocity.length() <= 1e-3);
		assert!(second.linear_velocity.length() <= 1e-3);
	}

	#[test]
	fn fast_bodies_stop_at_a_thin_wall_that_they_would_pass_through() {
		// A wall a tenth of a meter thick, with the side the bodies come at where x is zero.
		let wall = || {
			let mut wall = make_ground_body();
			wall.collision_shape = Shapes::Cube {
				size: Vector::new(0.05, 4.0, 4.0),
			};
			wall.position = math::Point::new(0.05, 0.0, 0.0);
			wall
		};
		// Each step carries a body 1.67 meters, which is more than the body and the wall are thick together.
		let (start, velocity) = (math::Point::new(-3.0, 0.0, 0.0), Vector::new(100.0, 0.0, 0.0));
		let mut turning_box = make_dynamic_box_body(start, velocity);
		turning_box.rotates = true;
		let bodies = [
			("sphere", make_dynamic_sphere_body(start, velocity, 0.5)),
			("box", make_dynamic_box_body(start, velocity)),
			("box that contacts turn", turning_box),
			("capsule", make_dynamic_character_body(start, velocity)),
		];

		for (name, body) in bodies {
			// Every one of the bodies reaches half a meter, or the capsule's radius, toward the wall.
			let reach = match body.collision_shape {
				Shapes::Capsule { radius, .. } => radius,
				_ => 0.5,
			};
			let mut world = make_world();
			world.gravity = Vector::zero();
			*world.bodies.get_mut() = [wall(), body].into_iter().collect();
			step(
				&mut world,
				MediaTime::from_seconds_f32(1.0 / 60.0),
				10,
				&DefaultChannel::new(),
			);

			let body = world.bodies.all().get_slot(1).expect("the fast body");
			assert!((body.position.x() + reach).abs() <= 0.01, "{name} is at {:?}", body.position);
			assert!(
				body.linear_velocity.length() <= 0.01,
				"{name} moves at {:?}",
				body.linear_velocity
			);
		}
	}

	#[test]
	fn a_body_that_takes_over_a_slot_does_not_inherit_its_contacts() {
		let mut world = make_world();
		let transforms = DefaultChannel::new();
		let sixtieth = MediaTime::from_seconds_f32(1.0 / 60.0);
		// Boxes that do not rotate, so that the light one stands still under fifty times its weight within the
		// thirty steps it gets.
		let one_meter_box = || Shapes::cube(Vector::new(0.5, 0.5, 0.5));
		let dynamic = |shape| Body::new(BodyTypes::Dynamic, shape).with_rotation(false);
		let place = |world: &mut World, handle: Handle, body: Body, height: f32| {
			world.create_body(handle, body);
			let slot = world.handles_to_bodies[&handle];
			world.bodies.get_mut()[slot].position = math::Point::new(0.0, height, 0.0);
			slot
		};

		// A light box rests on the ground under a heavy one, until the contact between them carries the heavy one.
		let (bottom, top) = (test_handle(), test_handle());
		let ground = Body::new(BodyTypes::Static, Shapes::cube(Vector::new(4.0, 1.0, 4.0)));
		place(&mut world, test_handle(), ground, 0.0);
		let bottom_slot = place(&mut world, bottom, dynamic(one_meter_box()), 1.5);
		place(&mut world, top, dynamic(one_meter_box()).with_mass(50.0), 2.5);
		step(&mut world, sixtieth, 30, &transforms);

		// A light box takes the heavy one's place, and its slot, resting on the box below as the heavy one did.
		let rest = world.bodies.all()[bottom_slot].position.y() + 0.997;
		let top_slot = place(&mut world, top, dynamic(one_meter_box()), rest);
		step(&mut world, sixtieth, 1, &transforms);

		// Started from the heavy box's contact, the two light boxes would be thrown apart.
		assert!(world.bodies.all()[bottom_slot].linear_velocity.length() <= 0.05);
		assert!(world.bodies.all()[top_slot].linear_velocity.length() <= 0.05);
	}

	#[test]
	fn overlapping_bodies_that_are_parting_are_left_to_part() {
		let mut world = make_world();
		world.gravity = Vector::zero();
		*world.bodies.get_mut() = [
			make_dynamic_sphere_body(math::Point::origin(), Vector::new(-1.0, 0.0, 0.0), 1.0),
			make_dynamic_sphere_body(math::Point::new(1.5, 0.0, 0.0), Vector::new(1.0, 0.0, 0.0), 1.0),
		]
		.into_iter()
		.collect();

		step(&mut world, MediaTime::from_seconds_f32(1.0 / 60.0), 1, &DefaultChannel::new());

		// The contact pushes the spheres apart a little faster and never pulls them back together.
		let (first, second) = (
			world.bodies.all().get_slot(0).unwrap(),
			world.bodies.all().get_slot(1).unwrap(),
		);
		assert!(second.linear_velocity.x() - first.linear_velocity.x() >= 2.0);
	}

	#[test]
	fn a_stack_of_boxes_stays_stacked() {
		let mut ground = make_ground_body();
		ground.friction = 0.5;
		let boxes = (0..6).map(|layer| make_dynamic_box_body(math::Point::new(0.0, 1.5 + layer as f32, 0.0), Vector::zero()));
		let world = stepped(std::iter::once(ground).chain(boxes).collect(), 5.0);

		for layer in 0..6 {
			let position = world.bodies.all().get_slot(layer + 1).unwrap().position;
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
		let body = world.bodies.all().get_slot(1).unwrap();
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
			let body = world.bodies.all().get_slot(layer + 1).unwrap();
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
			let body = world.bodies.all().get_slot(1).unwrap();
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

		let body = world.bodies.all().get_slot(1).unwrap();
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
			let body = world.bodies.all().get_slot(layer + 1).unwrap();
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
		let body = world.bodies.all().get_slot(1).unwrap();
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
			let character = world.bodies.all().get_slot(1).unwrap();
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

		let sphere = world.bodies.all().get_slot(1).unwrap();
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

		let body = world.bodies.all().get_slot(1).unwrap();
		assert!(uprightness(body) >= 0.9999, "the hull lies {} from level", uprightness(body));
		assert!(
			(body.position.y() - 1.5).abs() <= 0.01,
			"the hull rests at {}",
			body.position.y()
		);
		assert!(body.linear_velocity.length() <= 0.02 && body.angular_velocity.length() <= 0.02);
	}

	/// The `Constrained` struct holds a world with what its test creates and deletes constraints through, as an
	/// application would.
	struct Constrained {
		world: World,
		constraints: Factory<Constraint>,
		deletes: DefaultChannel<DeleteMessage>,
		/// What the world's steps run on.
		alley: Alley,
		/// Nothing authors transforms for the bodies of a test, or reads the ones a step publishes.
		authored: DefaultListener<TransformationUpdate>,
		published: DefaultChannel<TransformationUpdate>,
	}

	impl Constrained {
		/// Returns a world without bodies, whose steps run on `lanes` lanes, whatever the machine running the test
		/// would choose.
		fn on(lanes: usize) -> Self {
			let constraints = Factory::<Constraint>::new();
			let deletes = DefaultChannel::new();
			let mut world = World::new(Factory::<Body>::new().listener(), constraints.listener(), deletes.listener());
			world.set_step_lanes(StepLanes::new([(0, lanes)]));
			Self {
				world,
				constraints,
				deletes,
				alley: Alley::with_parallelism(lanes),
				authored: DefaultChannel::new().listener(),
				published: DefaultChannel::new(),
			}
		}

		/// Adds `body` to the world and returns the handle that constraints name it by.
		fn add(&mut self, body: PhysicsBody) -> Handle {
			let handle = body.handle;
			let index = self.world.bodies.get_mut().push(body);
			self.world.handles_to_bodies.insert(handle, index);
			handle
		}

		/// Creates a rod of `length` meters between the origins of two bodies, and returns its handle. The world
		/// takes it up in its next step.
		fn rod(&self, first: Handle, second: Handle, length: f32) -> Handle {
			self.constraints.create(Constraint::distance(first, second, length))
		}

		/// Deletes what was created with `handle`.
		fn delete(&mut self, handle: Handle) {
			self.deletes.send(DeleteMessage::new(handle));
			self.world.process_pending_deletions();
		}

		/// Advances the world by `steps` sixtieths of a second.
		fn run(&mut self, steps: usize) {
			let dt = MediaTime::from_seconds_f32(1.0 / 60.0);
			for _ in 0..steps {
				self.world
					.update(Time::new(dt, dt), &mut self.authored, &self.published, &mut self.alley);
			}
		}

		/// Returns the body that was added with `handle`.
		fn body(&self, handle: Handle) -> &PhysicsBody {
			&self.world.bodies.all()[self.world.handles_to_bodies[&handle]]
		}
	}

	/// Returns a small body at `position` that nothing moves, for constraints to hang other bodies from.
	fn make_fixed_point(position: math::Point) -> PhysicsBody {
		let mut body = make_ground_body();
		body.collision_shape = Shapes::Sphere { radius: 0.05 };
		body.position = position;
		body
	}

	/// Returns a dynamic ball at `position` that is small enough to touch nothing in these tests.
	fn make_bob(position: math::Point) -> PhysicsBody {
		make_dynamic_sphere_body(position, Vector::zero(), 0.1)
	}

	#[test]
	fn a_constraint_holds_its_bodies_from_when_both_exist_until_it_is_deleted() {
		let mut scene = Constrained::on(1);
		let (point, bob) = (
			make_fixed_point(math::Point::origin()),
			make_bob(math::Point::new(0.0, -1.0, 0.0)),
		);
		// The constraint is created before its bodies, and does nothing until they are there.
		let rod = scene.rod(point.handle, bob.handle, 1.0);
		scene.run(1);
		let (point, bob) = (scene.add(point), scene.add(bob));

		scene.run(60);
		let hanging = scene.body(bob);
		assert!((hanging.position - math::Point::new(0.0, -1.0, 0.0)).length() <= 0.01);
		assert!(hanging.linear_velocity.length() <= 0.01);
		assert_eq!(scene.body(point).position, math::Point::origin());

		scene.delete(rod);
		scene.run(30);
		// Half a second of free fall.
		assert!(scene.body(bob).position.y() < -2.5);
	}

	#[test]
	fn a_pendulum_swings_at_the_length_of_its_constraint() {
		let mut scene = Constrained::on(1);
		let point = scene.add(make_fixed_point(math::Point::origin()));
		let bob = scene.add(make_bob(math::Point::new(2.0, 0.0, 0.0)));
		scene.rod(point, bob, 2.0);

		let (mut furthest_off, mut lowest, mut fastest) = (0.0f32, 0.0f32, 0.0f32);
		for _ in 0..120 {
			scene.run(1);
			let bob = scene.body(bob);
			furthest_off = furthest_off.max(((bob.position - math::Point::origin()).length() - 2.0).abs());
			lowest = lowest.min(bob.position.y());
			fastest = fastest.max(bob.linear_velocity.length());
		}

		assert!(furthest_off <= 0.02, "the bob was {furthest_off} off the length");
		// It falls two meters to the lowest point of its swing, which it passes at the speed that fall gives it.
		assert!((lowest + 2.0).abs() <= 0.02, "the bob got down to {lowest}");
		assert!((fastest - 8.0).abs() <= 0.4, "the bob got as fast as {fastest}");
	}

	#[test]
	fn a_constraint_between_two_free_bodies_shares_their_momentum_along_it() {
		let mut scene = Constrained::on(1);
		scene.world.gravity = Vector::zero();
		// The light body leaves the heavy one, which is twice its mass, and the rod between them pulls it along.
		let mut light = make_bob(math::Point::origin());
		light.linear_velocity = Vector::new(-3.0, 1.0, 0.0);
		let mut heavy = make_bob(math::Point::new(1.0, 0.0, 0.0));
		heavy.inv_mass = 0.5;
		heavy.linear_velocity = Vector::new(0.0, 1.0, 0.0);
		let (light, heavy) = (scene.add(light), scene.add(heavy));
		scene.rod(light, heavy, 1.0);

		scene.run(60);

		let (light, heavy) = (scene.body(light), scene.body(heavy));
		for body in [light, heavy] {
			assert!((body.linear_velocity - Vector::new(-1.0, 1.0, 0.0)).length() <= 1e-3);
		}
		assert!(((heavy.position - light.position).length() - 1.0).abs() <= 1e-3);
	}

	#[test]
	fn a_box_hung_by_a_corner_turns_about_it() {
		let mut scene = Constrained::on(1);
		let (above, corner) = (math::Point::new(0.5, 1.5, 0.5), math::Point::new(0.5, 0.5, 0.5));
		// The rod goes straight up from the corner, so the box's weight, which is off to the side of it, turns the box.
		let point = scene.add(make_fixed_point(above));
		let hung = scene.add(make_rotating_box_body(math::Point::origin()));
		scene
			.constraints
			.create(Constraint::distance(point, hung, 1.0).with_anchors(math::Point::origin(), corner));

		let (mut furthest_off, mut least_upright) = (0.0f32, 1.0f32);
		for _ in 0..120 {
			scene.run(1);
			let hung = scene.body(hung);
			furthest_off = furthest_off.max(((hung.to_world(corner) - above).length() - 1.0).abs());
			least_upright = least_upright.min(uprightness(hung));
		}

		assert!(furthest_off <= 0.02, "the corner was {furthest_off} off the length");
		assert!(least_upright < 0.9, "the box stayed {least_upright} upright");
	}

	#[test]
	fn a_chain_of_constraints_hangs_from_its_first_link() {
		const LINKS: usize = 6;
		let mut scene = Constrained::on(1);
		let mut last = scene.add(make_fixed_point(math::Point::origin()));
		for link in 1..=LINKS {
			let above = last;
			last = scene.add(make_bob(math::Point::new(0.0, link as f32 * -0.5, 0.0)));
			scene.rod(above, last, 0.5);
		}

		scene.run(120);

		let last = scene.body(last);
		assert!((last.position - math::Point::new(0.0, LINKS as f32 * -0.5, 0.0)).length() <= 0.02);
		assert!(last.linear_velocity.length() <= 0.02);
	}

	#[test]
	fn a_constraint_does_not_push_its_body_into_what_the_body_rests_on() {
		let mut scene = Constrained::on(1);
		// The ground's top is one unit up. The ball rests on it, with the few millimeters of overlap a resting body
		// keeps, half a meter under a point that a rod twice as long holds it from. So the rod pushes it straight at
		// the ground.
		scene.add(make_ground_body());
		let point = scene.add(make_fixed_point(math::Point::new(0.0, 1.6, 0.0)));
		let ball = scene.add(make_bob(math::Point::new(0.0, 1.095, 0.0)));
		scene.rod(point, ball, 1.0);

		let mut deepest = 0.0f32;
		for _ in 0..60 {
			scene.run(1);
			deepest = deepest.max(1.1 - scene.body(ball).position.y());
		}

		assert!(deepest <= 0.01, "the ball sank {deepest} into the ground");
	}

	#[test]
	fn a_constraint_holds_its_bodies_the_same_on_any_number_of_lanes() {
		// A pendulum swings over a crowd that has enough contacts for lanes to share batches of them.
		let swung = |lanes: usize| {
			let mut scene = Constrained::on(lanes);
			for body in crowded_bodies() {
				scene.add(body);
			}
			let point = scene.add(make_fixed_point(math::Point::new(0.0, 6.0, 0.0)));
			let bob = scene.add(make_bob(math::Point::new(2.0, 6.0, 0.0)));
			scene.rod(point, bob, 2.0);
			scene.run(30);
			scene.body(bob).position
		};
		let on_one = swung(1);

		let rod = on_one - math::Point::new(0.0, 6.0, 0.0);
		assert!(
			(rod.length() - 2.0).abs() <= 0.02 && rod.y() < -1.0,
			"the bob is at {on_one:?}"
		);
		assert_eq!(swung(4), on_one);
	}

	#[test]
	fn deleting_a_constraint_lets_only_its_own_bodies_go() {
		let mut scene = Constrained::on(1);
		// Three bobs, each a meter under a point of its own.
		let hung: [(Handle, Handle); 3] = std::array::from_fn(|place| {
			let x = place as f32 * 2.0;
			let point = scene.add(make_fixed_point(math::Point::new(x, 0.0, 0.0)));
			let bob = scene.add(make_bob(math::Point::new(x, -1.0, 0.0)));
			(scene.rod(point, bob, 1.0), bob)
		});
		let hangs = |scene: &Constrained, bob: Handle| (scene.body(bob).position.y() + 1.0).abs() <= 0.01;
		// The world takes up what was created when it steps.
		scene.run(1);

		scene.delete(hung[0].0);
		scene.run(30);
		assert!(!hangs(&scene, hung[0].1) && hangs(&scene, hung[1].1) && hangs(&scene, hung[2].1));

		scene.delete(hung[2].0);
		scene.run(30);
		assert!(hangs(&scene, hung[1].1) && !hangs(&scene, hung[2].1));
	}

	#[test]
	fn a_constraint_created_again_with_its_handle_replaces_the_one_before() {
		let mut scene = Constrained::on(1);
		let point = scene.add(make_fixed_point(math::Point::origin()));
		let bob = scene.add(make_bob(math::Point::new(0.0, -1.0, 0.0)));
		let rod = scene.rod(point, bob, 1.0);
		scene.run(10);

		scene.constraints.derive(rod, Constraint::distance(point, bob, 1.5));
		scene.run(60);

		// The bob is let down to the new length, and hangs there.
		assert!((scene.body(bob).position.y() + 1.5).abs() <= 0.01);
		assert!(scene.body(bob).linear_velocity.length() <= 0.05);
	}
}
