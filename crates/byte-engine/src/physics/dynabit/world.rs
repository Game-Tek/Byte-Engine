use std::{ops::Range, time::Instant};

use math::{AABB, Scale, Vector, collision::ray_aabb_intersection};
use utils::{StableVec, StableVecHandle, hash::HashMap};

use crate::{
	application::Time,
	core::{
		alley::{Alley, ConcreteLane, Lane as _, LaneOwned, LaneRanges, Stage},
		channel::Channel,
		factory::{CreateMessage, Factory, Handle},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
	},
	gameplay::transform::{Transform, TransformationUpdate},
	physics::{
		RayHit, RayQuery, RayQueryResponse,
		body::{Body, BodyTypes},
		constraint::Constraint,
		dynabit::{
			body::{PhysicsBody, Rest, intersect, takes_search},
			constraint::Constraints,
			contact::{Contact, ContactPart, Contacts, Mobility},
			recycle::{self, Manifold, ManifoldPart},
			sleep::{Sleep, WAKE_SPEED},
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

/// How far apart, in meters, the bounds of a sleeping body may be from a body that moves for the sleeping body to
/// wake, or from where a body was moved from or to, created, or removed.
///
/// Bodies that lie on each other can be a little apart. A body that falls away from under a sleeping body is
/// still this near when it has fallen for two steps, which is when it is fast enough to wake it.
const WAKE_REACH: f32 = 0.05;

/// The `World` struct owns Dynabit simulation state and synchronizes it with entity handles.
pub struct World {
	/// The bodies, by slot. Lanes change them side by side, each the bodies in its own range of the slots.
	bodies: LaneRanges<StableVec<PhysicsBody>>,
	gravity: Vector,
	body_listener: DefaultListener<CreateMessage<Body>>,
	constraint_listener: DefaultListener<CreateMessage<Constraint>>,
	/// Deletions of any entity. The world removes the body and the constraint that were created with the handle.
	delete_listener: DefaultListener<DeleteMessage>,
	/// The ray queries to answer after the next step.
	ray_queries: DefaultListener<CreateMessage<RayQuery>>,
	/// Where the answers to ray queries are created.
	ray_responses: Factory<RayQueryResponse>,
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
	/// Which bodies sleep, and how long the others have been at rest.
	sleep: Sleep,
	/// Whether bodies that have come to rest go to sleep.
	sleeps: bool,
	/// The bounds of every body without mass before and after its author moved it, and of those that were added or
	/// removed, since the last step. Bodies that sleep near them wake as the next step begins.
	disturbed: Vec<AABB>,
	/// The slots of sleeping bodies that are about to be woken. Empty between uses, and kept so that waking does
	/// not allocate.
	roused: Vec<u32>,
	/// The slots of the bodies that were created since the last step, in order, while their transforms are adopted.
	/// Kept so that it does not allocate.
	created: Vec<u32>,
	/// The manifolds that the last step made or recycled, by the lane that kept them. Every lane reads them while it
	/// searches for contacts.
	manifolds: Vec<ManifoldPart>,
	/// The slots of every pair of bodies that a constraint holds, the lower slot first, in order, from the start of a
	/// step until the next one. The pairs' contacts are not recycled, because the constraint can press the bodies
	/// into each other however little they move, and the pairs make islands of the bodies that sleep together.
	held: Vec<[usize; 2]>,
}

impl World {
	/// Creates a Dynabit world that listens for the bodies, constraints, and ray queries that are created, and for
	/// the handles that are deleted. The world answers each ray query by creating a response in `ray_responses`.
	///
	/// Next, create [`Body`] and [`Constraint`] values through the factories the listeners come from, and call
	/// [`Self::update`] every step.
	pub fn new(
		body_listener: DefaultListener<CreateMessage<Body>>,
		constraint_listener: DefaultListener<CreateMessage<Constraint>>,
		delete_listener: DefaultListener<DeleteMessage>,
		ray_queries: DefaultListener<CreateMessage<RayQuery>>,
		ray_responses: Factory<RayQueryResponse>,
	) -> Self {
		Self {
			bodies: LaneRanges::new(StableVec::new()),
			gravity: Vector::new(0.0, -16.0, 0.0),
			body_listener,
			constraint_listener,
			delete_listener,
			ray_queries,
			ray_responses,
			handles_to_bodies: HashMap::with_capacity_and_hasher(1024, Default::default()),
			constraints: Constraints::default(),
			bounds: SweepBounds::default(),
			shares: LaneOwned::default(),
			step_lanes: StepLanes::for_machine(),
			detection: WorkShares::default(),
			contacts: Contacts::default(),
			solver: ContactSolver::default(),
			sleep: Sleep::default(),
			sleeps: true,
			disturbed: Vec::new(),
			roused: Vec::new(),
			created: Vec::new(),
			manifolds: Vec::new(),
			held: Vec::new(),
		}
	}

	/// Replaces how many lanes [`Self::update`] runs a step on, which starts as
	/// [`StepLanes::for_machine`].
	///
	/// Use this when measurements on a machine show that other lane counts step faster there.
	pub fn set_step_lanes(&mut self, step_lanes: StepLanes) {
		self.step_lanes = step_lanes;
	}

	/// Replaces whether bodies that have come to rest go to sleep, which they do to start with. With `false`, every
	/// sleeping body wakes and every step simulates every body.
	///
	/// A step does nothing for a sleeping body: it does not move it and publishes no transform for it. A world
	/// whose bodies all sleep takes no time to step.
	///
	/// A body sleeps once it has stayed where it is for half a second, whatever the bodies that touch it do. A body
	/// counts as staying while it moves less than two and a half centimeters and turns less than a twentieth of a
	/// radian in that time, so a body that moves slower than that stops. Bodies that a [`Constraint`] holds
	/// together sleep as one island, once all of them have stayed.
	///
	/// To the bodies that are awake, a sleeping body is as fixed as one without mass: they lie on it and lean on it
	/// without moving it. It wakes in the step in which a body comes within five centimeters of it that moves faster
	/// than half a meter a second, or that has moved more than a body at rest may since the current half second
	/// began. So it is knocked away by a body that hits it, and falls once what it lies on moves away. A body that
	/// pushes into it slower than that is stopped by it, as by a wall.
	///
	/// A sleeping body also wakes when it is pushed with [`Self::apply_impulse`], given a transform that moves it,
	/// or taken hold of or let go by a constraint, and when a body near it is given a transform that moves it,
	/// created, or removed.
	///
	/// Keep bodies awake to measure what a step costs, or to compare a simulation step by step with another.
	pub fn set_sleeping(&mut self, sleeps: bool) {
		self.sleeps = sleeps;
		if !sleeps {
			for slot in 0..self.bodies.get_mut().slots_len() {
				self.wake(slot);
			}
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
		// A push moves a sleeping body, which wakes what it comes near as it moves.
		self.wake(index.index());
		let Some(body) = self.bodies.get_mut().get_mut(index) else {
			return false;
		};

		body.apply_linear_impulse(impulse);
		true
	}

	/// Wakes the body in `slot` and the island it sleeps with, between two steps.
	fn wake(&mut self, slot: usize) {
		self.sleep.wake(self.bodies.get_mut(), slot, 0, |_, _| ());
	}

	/// Wakes the bodies of the constraints that hold the body that was created with `handle`. Call it when that
	/// body appears, or is one without mass that moves or goes away, which the bodies it is held to sleep through
	/// otherwise.
	fn wake_held_to(&mut self, handle: Handle) {
		for bodies in self.constraints.bodies().filter(|bodies| bodies.contains(&handle)) {
			for index in bodies.iter().filter_map(|body| self.handles_to_bodies.get(body)) {
				self.sleep.wake(self.bodies.get_mut(), index.index(), 0, |_, _| ());
			}
		}
	}

	/// Wakes the bodies that sleep near where a body was moved from or to, appeared, or went away since the last
	/// step. They may lie on it, or be in its way.
	fn wake_disturbed(&mut self) {
		if self.sleep.asleep() != 0 && !self.disturbed.is_empty() {
			let near = |bounds: AABB| {
				let (low, high) = (bounds.min().to_array(), bounds.max().to_array());
				self.disturbed.iter().any(|disturbed| {
					let (min, max) = (disturbed.min().to_array(), disturbed.max().to_array());
					(0..3).all(|axis| low[axis] <= max[axis] + WAKE_REACH && min[axis] <= high[axis] + WAKE_REACH)
				})
			};
			let bodies = self.bodies.get_mut();
			let asleep = bodies
				.indexed_iter()
				.filter(|(_, body)| body.is_asleep() && near(body.bounds()));
			self.roused.extend(asleep.map(|(slot, _)| slot as u32));
			for slot in self.roused.drain(..) {
				self.sleep.wake(bodies, slot as usize, 0, |_, _| ());
			}
		}
		self.disturbed.clear();
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
	///    the constraints ready. Where a body that moves comes near a sleeping one, that lane first wakes the
	///    sleeping body's island, and every lane finds the contacts of the bodies that woke, in a second round. A
	///    pair whose contacts take a search over its shapes gets the contacts it had in the last step instead, moved
	///    with its bodies, for as long as they hardly move relative to each other.
	/// 4. The lanes prepare the contacts and pass over them several times, sharing every batch that is large
	///    enough, one batch to a stage. One lane takes the small ones. Before each pass, one lane solves the
	///    constraints in turn, so that the contacts have the last word on where bodies may be. After the last pass
	///    it keeps what the constraints start the next step from.
	/// 5. Every lane keeps the impulses of its contacts for the next step, gives its share of the bodies their new
	///    velocities, moves them, and publishes their transforms.
	///
	/// A step leaves out the bodies that sleep, and a step of a world in which no body with mass is awake does
	/// nothing. After a step, the bodies that have come to rest go to sleep. See [`Self::set_sleeping`].
	///
	/// Contacts are prepared and solved in the same order whatever the number of lanes, so the simulation does not
	/// depend on it. The order transforms are published in does: each lane publishes the bodies of its share in slot
	/// order, and the lanes' runs follow each other as they finish.
	///
	/// After the step, even one that does nothing, the world answers the [`RayQuery`] values created since the last
	/// update, in the order they were created, against the bodies where the step left them.
	pub fn update(
		&mut self,
		time: Time,
		transforms_rx: &mut impl Listener<TransformationUpdate>,
		transforms_tx: &(impl Channel<TransformationUpdate> + Sync),
		alley: &mut Alley,
	) {
		// Listeners belong to the calling thread, so messages are adopted before the lanes start.
		self.adopt_updates(transforms_rx);
		self.wake_disturbed();
		self.step(time, transforms_tx, alley);
		self.answer_ray_queries();
	}

	/// Runs the step that [`Self::update`] describes, once the world has adopted what was created and moved.
	fn step(&mut self, time: Time, transforms_tx: &(impl Channel<TransformationUpdate> + Sync), alley: &mut Alley) {
		let step = time.delta();
		let bodies = self.bodies.get_mut();
		// Nothing moves in a world whose bodies all sleep or have no mass. The search ends at the first body that
		// is awake, so it only takes long when it saves the step.
		if !bodies.iter().any(|body| body.active_since() == 0) {
			return;
		}
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
			// The second round finds the contacts of the bodies that the first one woke. Those did not begin the step
			// moving, so they wake no others.
			let mut round = 0;
			let (shared_batches, rest) = loop {
				let _ = lane.each_ref(world, |world, lane| world.detect_share_contacts(lane, lanes, step, round));
				let batches = lane.broadcast_mut(world, |world| {
					let woke = world.gather_share_contacts(lanes, round, step);
					if woke && round == 0 {
						return None;
					}
					let batches = world.batch_contacts(lanes, (round as usize + 1) * lanes);
					world.pre_solve_constraints(step);
					Some(batches)
				});
				match batches {
					Some(batches) => break batches,
					None => round += 1,
				}
			};
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
		// The manifolds each lane kept are the ones the next step recycles, and their storage is the next to fill.
		self.manifolds
			.resize_with(self.shares.iter_mut().count(), ManifoldPart::default);
		for (lane, (share, last)) in self.shares.iter_mut().zip(&mut self.manifolds).enumerate() {
			if lane < lanes {
				std::mem::swap(&mut share.manifolds, last);
			} else {
				// A lane that did not run kept nothing in this step.
				last.clear();
			}
			share.manifolds.clear();
		}
		if self.sleep.watch(step.as_seconds_f32()) {
			self.settle();
		}
	}

	/// Answers every ray query created since the last call, against the bodies where they are now.
	fn answer_ray_queries(&mut self) {
		let bodies = self.bodies.get_mut();
		while let Some(message) = self.ray_queries.read() {
			let hit = first_hit(bodies, message.data());
			self.ray_responses.create(RayQueryResponse::new(message.handle(), hit));
		}
	}

	/// Ends a watch for bodies at rest: puts the bodies to sleep that stayed where they were through it, with the
	/// islands they make, and begins the next watch.
	fn settle(&mut self) {
		// The step that ends the watch listed the pairs that constraints hold.
		let solver = &mut self.solver;
		self.sleep
			.settle(self.bodies.get_mut(), self.sleeps, self.held.iter().copied(), |slot, body| {
				solver.fix(slot, body)
			});
	}

	/// Registers created bodies and constraints, and applies authored transforms to the bodies they target.
	fn adopt_updates(&mut self, transforms_rx: &mut impl Listener<TransformationUpdate>) {
		self.created.clear();
		while let Some(message) = self.body_listener.read() {
			let handle = message.handle();
			let body = message.into_data();
			self.create_body(handle, body);
		}
		self.created.sort_unstable();
		// Like bodies, creation messages are upserts, so one handle always owns at most one constraint.
		while let Some(message) = self.constraint_listener.read() {
			let handle = message.handle();
			let constraint = message.into_data();
			// The constraint pulls on its bodies from now on, and the one it replaces no longer holds its own.
			let held = constraint.bodies();
			let replaced = self.constraints.insert(handle, constraint);
			self.wake_bodies(held);
			if let Some(replaced) = replaced {
				self.wake_bodies(replaced.bodies());
			}
		}
		while let Some(message) = transforms_rx.read() {
			let handle = message.handle();
			let Some(index) = self.handles_to_bodies.get(&handle).copied() else {
				continue;
			};
			let body = &mut self.bodies.get_mut()[index];
			let transform = message.transform();
			let placed = (transform.get_position(), transform.get_orientation(), transform.scale());
			// A transform that says where the body already is moves nothing, so it wakes nothing.
			if placed == (body.position, body.orientation, body.scale) {
				continue;
			}
			// Only a body that sleeps cares where others are moved.
			let before = (self.sleep.asleep() != 0).then(|| body.bounds());
			(body.position, body.orientation, body.scale) = placed;
			let Some(before) = before else {
				continue;
			};
			// What lay on the body where it was has to fall, and what is in its way where it is now has to move. A
			// body that was only just created was nowhere before.
			let after = body.bounds();
			let created = self.created.binary_search(&(index.index() as u32)).is_ok();
			self.disturb(handle, (!created).then_some(before).into_iter().chain([after]));
			self.wake(index.index());
		}
	}

	/// Wakes the bodies that were created with `handles`, with the islands they sleep with.
	fn wake_bodies(&mut self, handles: [Handle; 2]) {
		for index in handles
			.map(|handle| self.handles_to_bodies.get(&handle).copied())
			.into_iter()
			.flatten()
		{
			self.wake(index.index());
		}
	}

	/// Notes that the body that was created with `handle` appeared in, moved between, or went away from `bounds`,
	/// so that the next step begins by waking the bodies that sleep near them. Wakes the bodies that a constraint
	/// holds to that body right away.
	fn disturb(&mut self, handle: Handle, bounds: impl IntoIterator<Item = AABB>) {
		if self.sleep.asleep() != 0 {
			self.disturbed.extend(bounds);
			self.wake_held_to(handle);
		}
	}

	/// Starts the step for the bodies in one lane's share of the body slots: applies gravity, gives the solver what
	/// the step's contacts need of each body, and keeps the bounds each sweeps through in that lane's storage.
	fn start_share(&self, lane: &ConcreteLane<'_>, dt: MediaTime) {
		let mut share = self.shares.own(lane);
		let mut taken = self.solver.take_bodies(lane);
		let seconds = dt.as_seconds_f32();
		// Only a body that sleeps cares whether the bodies near it move.
		let sleepers = self.sleep.asleep() != 0;
		for (slot, body) in self.bodies.own(lane).items() {
			let Rest::Awake { strayed, .. } = body.rest else {
				// A sleeping body stays as it is, and the solver keeps it as it was when it fell asleep. Its bounds,
				// out to where a body that moves wakes it, are how bodies that are awake find it.
				let (bounds, reach) = (body.bounds(), Vector::new(WAKE_REACH, WAKE_REACH, WAKE_REACH));
				share
					.bounds
					.push((slot, AABB::new(bounds.min() - reach, bounds.max() + reach)));
				continue;
			};
			if sleepers {
				// How fast the body moves before gravity pulls it, which a body at rest only meets with what it lies
				// on, tells whether it wakes the sleeping bodies near it.
				let moving = strayed || body.speed() > WAKE_SPEED;
				if let Rest::Awake { stirs, .. } = &mut body.rest {
					*stirs = moving;
				}
			}
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
		self.held.clear();
		let slot = |handle| Some(self.handles_to_bodies.get(&handle)?.index());
		let held = self.constraints.bodies().filter_map(|[first, second]| {
			let (first, second) = (slot(first)?, slot(second)?);
			Some([first.min(second), first.max(second)])
		});
		self.held.extend(held);
		self.held.sort_unstable();
	}

	/// Finds the contacts that `round` of the step's search is for in the share of the swept bounds that one of
	/// `lanes` lanes has, and keeps them in that lane's storage, with the sleeping bodies that the search wakes.
	///
	/// Bodies in a dense part of a pile, or with shapes that cost more to test, take longer than others. The shares
	/// are sized by how long each took in the last step, so that no lane keeps the others waiting. The contacts come
	/// out in the same order whatever the shares are.
	fn detect_share_contacts(&self, lane: &ConcreteLane<'_>, lanes: usize, dt: MediaTime, round: u32) {
		let mut share = self.shares.own(lane);
		let share = &mut *share;
		// The part holds what an earlier step or round found.
		share.contacts.clear();
		let range = self.detection.range(lane.idx(), lanes, self.bounds.count());
		let positions = range.len();
		let started = Instant::now();
		let recycled = Recycled {
			manifolds: &self.manifolds,
			lane: lane.idx(),
			held: &self.held,
		};
		detect_contacts(
			&self.bounds,
			self.bodies.all(),
			range,
			dt.as_seconds_f32(),
			round,
			recycled,
			share,
		);
		// A later round only tests the bodies that woke, which says little about how long a share takes.
		if round == 0 {
			share.detection = (positions, started.elapsed().as_nanos() as u64);
		}
	}

	/// Gathers the contacts that `round` of the step's search found in every share, and wakes the islands of the
	/// sleeping bodies that the search found a body that moves near. Returns whether any woke, which have contacts
	/// for the next round to find.
	///
	/// A body that wakes starts the step as the bodies that were awake did: gravity pulls it, the solver takes it as
	/// it is then, and the contacts that were found with it change it.
	fn gather_share_contacts(&mut self, lanes: usize, round: u32, dt: MediaTime) -> bool {
		// Shares are consecutive ranges of the swept bounds, so taking them in lane order keeps the order one scan
		// finds, round after round. Each share gets the storage its contacts had in an earlier step.
		let first_part = round as usize * lanes;
		for (lane, share) in self.shares.iter_mut().take(lanes).enumerate() {
			std::mem::swap(self.contacts.part_mut(first_part + lane), &mut share.contacts);
			self.roused.append(&mut share.roused);
		}
		let woke = !self.roused.is_empty();
		let (bodies, pull) = (self.bodies.get_mut(), self.gravity * dt.as_seconds_f32());
		for slot in self.roused.drain(..) {
			self.sleep.wake(bodies, slot as usize, round + 1, |slot, body| {
				body.apply_linear_impulse(pull * body.mass());
				self.solver.retake(slot, body);
			});
		}
		if woke {
			let bodies = &*bodies;
			let awake = |slot| {
				bodies
					.get_slot(slot)
					.is_some_and(|body| body.active_since() != PhysicsBody::NEVER_ACTIVE)
			};
			self.contacts.release((round as usize + 1) * lanes, awake);
		}
		woke
	}

	/// Sorts the contacts that were gathered into the first `parts` parts into batches. Returns how many batches
	/// lanes should share, and whether any contacts are left for one lane to prepare and solve. From here on, each
	/// of `lanes` lanes has its own bodies again.
	fn batch_contacts(&mut self, lanes: usize, parts: usize) -> (u32, bool) {
		self.contacts.clear_from(parts);
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

	/// Gets the constraints ready for the step, once [`Self::batch_contacts`] has begun the solver's.
	fn pre_solve_constraints(&mut self, dt: MediaTime) {
		// Constraints name their bodies by handle, so they hold whichever body has the handle in this step.
		let (bodies, slots) = (&*self.bodies.get_mut(), &self.handles_to_bodies);
		let body = |handle| {
			let index = *slots.get(&handle)?;
			Some((index.index(), bodies.get(index)?))
		};
		self.constraints.pre_solve(body, &mut self.solver, dt.as_seconds_f32());
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
			// Nothing moved a sleeping body, and what the solver has of it is from before it fell asleep.
			let Rest::Awake { strayed, .. } = body.rest else {
				continue;
			};
			solved.give(slot, body);
			if body.body_type == BodyTypes::Dynamic {
				body.update(dt);
				let transform = Transform::new(body.position, body.scale, body.orientation);
				share.transforms.push(TransformationUpdate::new(body.handle, transform));
				// While bodies may not sleep, none is watched.
				let moved = strayed || !self.sleeps || self.sleep.strays(slot, body);
				if let Rest::Awake { strayed, since, .. } = &mut body.rest {
					// The next step finds the body awake from its first round on.
					(*strayed, *since) = (moved, 0);
				}
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
			rest: Rest::default(),
		});
		// The slot may have held another body, whose contacts the solver still remembers.
		self.solver.forget(index.index());
		self.handles_to_bodies.insert(handle, index);
		self.created.push(index.index() as u32);
		// A constraint that waited for the body acts from now on. The transform that places the body wakes what
		// sleeps where it is.
		if self.sleep.asleep() != 0 {
			self.wake_held_to(handle);
		}
	}

	/// Removes the body and the constraint that were created with the handle of every pending deletion message.
	pub fn process_pending_deletions(&mut self) {
		while let Some(message) = self.delete_listener.read() {
			let handle = message.into_handle();
			self.remove_body(handle);
			// The bodies that the constraint held are free to move.
			if let Some(removed) = self.constraints.remove(handle) {
				self.wake_bodies(removed.bodies());
			}
		}
	}

	/// Removes and returns the physics body for `handle`.
	///
	/// The bodies that slept with it wake, and so do those that sleep near it, which may lie on it.
	pub fn remove_body(&mut self, handle: Handle) -> Option<PhysicsBody> {
		let index = *self.handles_to_bodies.get(&handle)?;
		if let Some(bounds) = self.bodies.get_mut().get(index).map(PhysicsBody::bounds) {
			self.disturb(handle, [bounds]);
			self.wake(index.index());
		}
		self.handles_to_bodies.remove(&handle);
		self.bodies.get_mut().remove(index)
	}
}

/// Adds to `found` the contacts that `round` of a step's search is for, between each body in `range` of the swept
/// `bounds` and the bodies after it. Adds to `roused` the slot of every sleeping body whose bounds reach a body that
/// stirs, as [`Rest::Awake`] tells, which may be there more than once.
///
/// A contact changes nothing unless one of its bodies is awake and has mass. The first round, zero, tests the pairs
/// with such a body. Each later round tests the pairs that only have one since the round before it woke bodies, so
/// every pair is tested once.
///
/// Ranges that cover every scan position find each contact once, in the same order whatever the ranges are.
///
/// A pair whose contacts take a search over its shapes gets the contacts that `recycled` keeps for it instead, when
/// they are close enough. `share` keeps the manifold of every such pair for the next step.
fn detect_contacts(
	bounds: &SweepBounds,
	bodies: &StableVec<PhysicsBody>,
	range: Range<usize>,
	dt: f32,
	round: u32,
	recycled: Recycled<'_>,
	share: &mut Share,
) {
	let Share {
		contacts: found,
		roused,
		scratch,
		manifolds: kept,
		..
	} = share;
	bounds.overlaps(range, |[first, second]| {
		// The body in the lower slot comes first, so that a pair's contacts are the same from step to step whichever
		// way the scan finds it.
		let [first, second] = if first < second { [first, second] } else { [second, first] };
		let (Some(a), Some(b)) = (bodies.get_slot(first), bodies.get_slot(second)) else {
			return;
		};
		if a.active_since().min(b.active_since()) != round {
			return;
		}
		// A body without mass is never changed by a contact, and a sleeping one only if it wakes.
		let mobility = |body: &PhysicsBody| match body.rest {
			Rest::Asleep { .. } => Mobility::Asleep,
			Rest::Awake { .. } if body.inv_mass != 0.0 => Mobility::Movable,
			Rest::Awake { .. } => Mobility::Fixed,
		};
		let mobilities = || [(first, mobility(a)), (second, mobility(b))];
		let find = |contacts: &mut Vec<Contact>| {
			if takes_search(a, b) && !recycle::outpaces(a, b, dt) {
				recycled.contacts((a, first), (b, second), dt, contacts, (kept, scratch));
			} else {
				intersect((a, first), (b, second), dt, contacts, scratch);
			}
		};
		found.find(find, mobilities);
		// A body that moves may be about to push into a sleeping body, or to leave it without what it lies on, so it
		// wakes it whether they touch yet or not.
		for (sleeper, slot, other) in [(a, first, b), (b, second, a)] {
			if sleeper.is_asleep() && matches!(other.rest, Rest::Awake { stirs: true, .. }) {
				roused.push(slot as u32);
			}
		}
	});
}

/// Returns where the ray of `query` first meets one of `bodies` within its length, sleeping ones included.
///
/// Every body is tested in turn. Each hit shortens how far the ray reaches, so the bodies whose bounds it no longer
/// reaches are passed over without casting against their shapes.
fn first_hit(bodies: &StableVec<PhysicsBody>, query: &RayQuery) -> Option<RayHit> {
	let ray = query.ray();
	let mut nearest = None;
	let mut reach = query.max_distance();
	for body in bodies.iter() {
		if !ray_aabb_intersection(&ray, &body.bounds()).is_some_and(|entry| entry <= reach) {
			continue;
		}
		if let Some((distance, normal)) = body.ray_cast(&ray, reach) {
			reach = distance;
			nearest = Some(RayHit::new(body.handle, &ray, distance, normal));
		}
	}
	nearest
}

/// The `Recycled` struct gives a search for contacts the manifolds of the last step that it may recycle.
#[derive(Clone, Copy)]
struct Recycled<'a> {
	manifolds: &'a [ManifoldPart],
	/// The lane that searches.
	lane: usize,
	/// The slots of the pairs that a constraint holds, the lower first, in order. Their contacts are found again in
	/// every step.
	held: &'a [[usize; 2]],
}

impl Recycled<'_> {
	/// Adds the contacts between `a`, in slot `i`, and `b`, in slot `j`, whose contacts take a search, to
	/// `contacts`: the ones of the last step's manifold of the pair, moved with the bodies, when they are close
	/// enough, and otherwise the ones a search finds. Keeps the pair's manifold in `kept` for the next step.
	// Made part of the search for contacts, where most pairs do not take this way, this made that search's steps
	// of every pile slower.
	#[inline(never)]
	fn contacts(
		&self,
		(a, i): (&PhysicsBody, usize),
		(b, j): (&PhysicsBody, usize),
		dt: f32,
		contacts: &mut Vec<Contact>,
		(kept, scratch): (&mut ManifoldPart, &mut ContactScratch),
	) {
		let manifold = self.manifold([i, j]);
		if let Some(manifold) = manifold.filter(|manifold| manifold.recycle((a, i), (b, j), contacts)) {
			return kept.keep([i, j], *manifold);
		}
		let before = contacts.len();
		intersect((a, i), (b, j), dt, contacts, scratch);
		// Bodies that only meet later in the step have contacts that change with how fast they move, so only
		// contacts where the bodies are now are kept. Bodies that move far enough in a step not to touch again where
		// they are move too far for them to be recycled.
		if contacts.get(before).is_some_and(|contact| contact.toi == 0.0) {
			kept.keep([i, j], Manifold::of(a, b, &contacts[before..]));
		}
	}

	/// Returns the manifold of the last step that the pair of bodies in `slots`, the lower first, may recycle.
	fn manifold(&self, slots: [usize; 2]) -> Option<&Manifold> {
		if self.held.binary_search(&slots).is_ok() {
			return None;
		}
		recycle::find(self.manifolds, self.lane, slots)
	}
}

/// The `Share` struct holds what one lane produces from its share of a stage until one lane collects it.
///
/// The storage is kept between steps so lanes do not allocate.
#[derive(Default)]
struct Share {
	/// The contacts found in a share of the swept bounds.
	contacts: ContactPart,
	/// The slots of the sleeping bodies that those contacts touch.
	roused: Vec<u32>,
	/// Working storage for finding those contacts.
	scratch: ContactScratch,
	/// The manifolds of the pairs whose contacts the lane found with a search or recycled in this step, for the next
	/// step to recycle.
	manifolds: ManifoldPart,
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

	use std::f32::consts::FRAC_PI_2;

	use math::Orientation;

	use super::*;
	use crate::{
		core::{channel::DefaultChannel, factory::Factory},
		physics::{LocalSpace, collider::Shapes, dynabit::contact::Contact},
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
			Factory::<RayQuery>::new().listener(),
			Factory::new(),
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
			rest: Rest::default(),
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
			rest: Rest::default(),
		}
	}

	/// Returns the contacts one step of `dt` seconds finds between the world's bodies.
	fn detected_contacts(world: &mut World, dt: f32) -> Vec<Contact> {
		let bounds = world.bodies.all().indexed_iter();
		world.bounds.rebuild(
			bounds.map(|(slot, body)| (slot, body.swept_bounds(dt))),
			&mut Alley::with_parallelism(1),
		);
		let mut share = Share::default();
		let recycled = Recycled {
			manifolds: &[],
			lane: 0,
			held: &[],
		};
		detect_contacts(
			&world.bounds,
			world.bodies.all(),
			0..world.bounds.count(),
			dt,
			0,
			recycled,
			&mut share,
		);
		share.contacts.into_contacts()
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

	#[test]
	fn ray_queries_are_answered_in_order_with_the_first_body_along_each() {
		let (queries, responses) = (Factory::<RayQuery>::new(), Factory::<RayQueryResponse>::new());
		let mut listener = responses.listener();
		let mut world = World::new(
			Factory::<Body>::new().listener(),
			Factory::<Constraint>::new().listener(),
			DefaultChannel::new().listener(),
			queries.listener(),
			responses,
		);
		// The ground's top is at height one. A static one-meter box hovers over it with its top at 3.5.
		let ground = make_ground_body();
		let mut hovering = make_ground_body();
		hovering.collision_shape = Shapes::cube(Vector::new(0.5, 0.5, 0.5));
		hovering.position = math::Point::new(2.0, 3.0, 0.0);
		let (ground_handle, hovering_handle) = (ground.handle, hovering.handle);
		*world.bodies.get_mut() = [ground, hovering].into_iter().collect();

		let down_from = |x: f32, to: f32| {
			RayQuery::segment(math::Point::new(x, 10.0, 0.0), math::Point::new(x, to, 0.0)).expect("the segment has a length")
		};
		let asked = [
			queries.create(down_from(2.0, -10.0)),
			queries.create(down_from(-2.0, -10.0)),
			queries.create(down_from(-2.0, 2.0)),
		];
		// Nothing moves in the step, which leaves the queries for after it.
		step(&mut world, MediaTime::from_seconds_f32(1.0 / 60.0), 1, &DefaultChannel::new());

		let answers = listener.to_vec();
		assert_eq!(answers.iter().map(|answer| answer.data().query()).collect::<Vec<_>>(), asked);
		assert!(answers.iter().all(|answer| !asked.contains(&answer.handle())));
		let hits = answers.iter().map(|answer| answer.data().hit()).collect::<Vec<_>>();
		let on_box = hits[0].expect("the ray down onto the box meets it");
		assert_eq!(on_box.body(), hovering_handle);
		assert!((on_box.distance() - 6.5).abs() <= 1e-4, "{on_box:?}");
		assert!(
			(on_box.point() - math::Point::new(2.0, 3.5, 0.0)).length() <= 1e-4,
			"{on_box:?}"
		);
		assert!(
			(on_box.normal().into_vector() - Vector::new(0.0, 1.0, 0.0)).length() <= 1e-4,
			"{on_box:?}"
		);
		let on_ground = hits[1].expect("the ray beside the box meets the ground");
		assert_eq!(on_ground.body(), ground_handle);
		assert!((on_ground.distance() - 9.0).abs() <= 1e-4, "{on_ground:?}");
		assert_eq!(hits[2], None, "the segment ends above the ground");

		// Every query is answered once.
		step(&mut world, MediaTime::from_seconds_f32(1.0 / 60.0), 1, &DefaultChannel::new());
		assert!(listener.read().is_none());
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
			let mut world = World::new(
				Factory::<Body>::new().listener(),
				constraints.listener(),
				deletes.listener(),
				Factory::<RayQuery>::new().listener(),
				Factory::new(),
			);
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
			self.join(Constraint::distance(first, second, length))
		}

		/// Creates `constraint` and returns its handle. The world takes it up in its next step.
		fn join(&self, constraint: Constraint) -> Handle {
			self.constraints.create(constraint)
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

		// A rod that swings this fast is left a few centimeters off its length by every step, and takes up a share.
		assert!(furthest_off <= 0.08, "the bob was {furthest_off} off the length");
		// It falls two meters to the lowest point of its swing, which it passes at the speed that fall gives it.
		assert!((lowest + 2.0).abs() <= 0.08, "the bob got down to {lowest}");
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

		assert!(furthest_off <= 0.06, "the corner was {furthest_off} off the length");
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
	fn a_chain_carries_a_body_much_heavier_than_its_links() {
		const LINKS: usize = 12;
		let mut scene = Constrained::on(1);
		let mut bodies = vec![scene.add(make_fixed_point(math::Point::origin()))];
		for link in 1..=LINKS {
			let mut bob = make_bob(math::Point::new(0.0, link as f32 * -0.25, 0.0));
			if link == LINKS {
				bob.inv_mass = 0.1;
			}
			bodies.push(scene.add(bob));
			scene.rod(bodies[link - 1], bodies[link], 0.25);
		}

		// Each link is pulled by all of the weight under it, which it only feels once the links under it have. So the
		// links stretch by tens of centimeters together unless they are solved several times over, each step
		// starting from the impulses of the one before.
		let mut furthest_off = 0.0f32;
		for _ in 0..180 {
			scene.run(1);
			let lengths = bodies
				.windows(2)
				.map(|link| (scene.body(link[1]).position - scene.body(link[0]).position).length());
			furthest_off = furthest_off.max(lengths.map(|length| (length - 0.25).abs()).sum());
		}

		assert!(
			furthest_off <= 0.09,
			"the links were {furthest_off} off their lengths together"
		);
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
			(rod.length() - 2.0).abs() <= 0.08 && rod.y() < -1.0,
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
		scene.run(180);

		// The bob is let down to the new length, a share of the way with every step, and hangs there.
		assert!((scene.body(bob).position.y() + 1.5).abs() <= 0.01);
		assert!(scene.body(bob).linear_velocity.length() <= 0.05);
	}

	/// Returns a dynamic bar that contacts and constraints turn, 0.7 meters long along its own x axis, with its middle
	/// at `position`.
	fn make_bar(position: math::Point) -> PhysicsBody {
		let mut body = make_rotating_box_body(position);
		body.collision_shape = Shapes::Cube {
			size: Vector::new(0.35, 0.05, 0.05),
		};
		body
	}

	/// The point of a bar, half a meter from its middle, that the hinges of these tests go through.
	fn bar_end(side: f32) -> math::Point<LocalSpace> {
		math::Point::new(0.5 * side, 0.0, 0.0)
	}

	/// Returns how far, in radians, `body` is turned about the z axis from how it was made. Only for a body that
	/// is turned about that axis alone.
	fn turn_about_z(body: &PhysicsBody) -> f32 {
		body.orientation.to_rotation_vector::<math::WorldSpace>().z()
	}

	#[test]
	fn a_hinge_lets_its_body_turn_about_its_pin_only() {
		let mut scene = Constrained::on(1);
		let pivot = math::Point::new(0.0, 5.0, 0.0);
		let post = scene.add(make_fixed_point(pivot));
		let mut bar = make_bar(pivot + Vector::new(0.5, 0.0, 0.0));
		// A knock that sends a free bar off to the side, spinning about its length and tumbling.
		bar.linear_velocity = Vector::new(0.0, 0.0, 3.0);
		bar.angular_velocity = Vector::new(2.0, 2.0, 0.0);
		let bar = scene.add(bar);
		scene.join(
			Constraint::hinge(post, bar, math::UnitVector::z_axis(), None).with_anchors(math::Point::origin(), bar_end(-1.0)),
		);

		let mut lowest = f32::MAX;
		for step in 0..180 {
			scene.run(1);
			let bar = scene.body(bar);
			let off = (bar.to_world(bar_end(-1.0)) - pivot).length();
			// A bar that swings this fast leaves the hinge behind by what a step's share of the way back does not
			// take up.
			assert!(off <= 0.05, "the bar's end is {off} from the hinge in step {step}");
			let pin = bar.orientation.rotate_vector(Vector::<math::WorldSpace>::new(0.0, 0.0, 1.0));
			assert!(pin.z() >= 0.999, "the bar's pin leans {} in step {step}", pin.z());
			lowest = lowest.min(bar.position.y());
		}
		// Gravity swings the bar down about the pin, as nothing stops it.
		assert!(lowest <= pivot.y() - 0.45, "the bar only came down to {lowest}");
	}

	#[test]
	fn a_hinge_stops_at_either_end_of_its_limit() {
		// Gravity turns a bar that sticks out to the side of its hinge down. That is back about a pin that points
		// one way, and on about a pin that points the other way.
		for (toward, stop) in [(1.0, -0.5), (-1.0, 0.5)] {
			let mut scene = Constrained::on(1);
			let pivot = math::Point::new(0.0, 5.0, 0.0);
			let post = scene.add(make_fixed_point(pivot));
			let bar = scene.add(make_bar(pivot + Vector::new(0.5, 0.0, 0.0)));
			let axis = Vector::<LocalSpace>::new(0.0, 0.0, toward).normalized().unwrap();
			let limit = math::Radians::new(-0.5)..=math::Radians::new(0.5);
			scene.join(Constraint::hinge(post, bar, axis, Some(limit)).with_anchors(math::Point::origin(), bar_end(-1.0)));

			scene.run(240);

			let bar = scene.body(bar);
			let angle = turn_about_z(bar) * toward;
			assert!((angle - stop).abs() <= 0.03, "the bar rests at {angle}, not {stop}");
			assert!(bar.angular_velocity.length() <= 0.05);
			assert!((bar.to_world(bar_end(-1.0)) - pivot).length() <= 0.03);
		}
	}

	#[test]
	fn a_hinge_keeps_its_bodies_turned_as_they_were_when_it_first_held_them() {
		let mut scene = Constrained::on(1);
		let pivot = math::Point::new(0.0, 5.0, 0.0);
		let post = scene.add(make_fixed_point(pivot));
		// The bar starts turned about another direction than the pin's, and the hinge may not turn at all.
		let turned = Orientation::try_from_rotation_vector(Vector::<math::WorldSpace>::new(0.0, 0.8, 0.0)).unwrap();
		let mut bar = make_bar(pivot + turned.rotate_vector(Vector::new(0.5, 0.0, 0.0)));
		bar.orientation = turned;
		let bar = scene.add(bar);
		let locked = math::Radians::new(0.0)..=math::Radians::new(0.0);
		scene.join(
			Constraint::hinge(post, bar, math::UnitVector::z_axis(), Some(locked))
				.with_anchors(math::Point::origin(), bar_end(-1.0)),
		);

		scene.run(240);

		let bar = scene.body(bar);
		// Two orientations are a turn of twice the angle whose cosine this is apart.
		let alike = bar.orientation.dot(turned).abs();
		assert!(alike >= 0.9998, "the bar turned to {alike} of how it started");
		assert!((bar.to_world(bar_end(-1.0)) - pivot).length() <= 0.03);
	}

	#[test]
	fn a_limb_of_hinged_bars_bends_no_further_than_the_limits_of_its_hinges() {
		let mut scene = Constrained::on(1);
		let pivot = math::Point::new(0.0, 5.0, 0.0);
		let post = scene.add(make_fixed_point(pivot));
		// Three bars in a row that stick out to the side, each hinged to the one before it.
		let limit = || math::Radians::new(-0.3)..=math::Radians::new(0.3);
		let mut bars = Vec::new();
		let (mut before, mut end) = (post, math::Point::origin());
		for bar in 0..3 {
			let handle = scene.add(make_bar(pivot + Vector::new(0.5 + bar as f32, 0.0, 0.0)));
			scene.join(
				Constraint::hinge(before, handle, math::UnitVector::z_axis(), Some(limit())).with_anchors(end, bar_end(-1.0)),
			);
			bars.push(handle);
			(before, end) = (handle, bar_end(1.0));
		}

		scene.run(300);

		// Gravity bends every hinge down as far as it goes, so each bar hangs that much lower than the one before.
		let (mut before, mut joint) = (0.0, pivot);
		for (bar, &handle) in bars.iter().enumerate() {
			let body = scene.body(handle);
			let bend = turn_about_z(body) - before;
			assert!((bend + 0.3).abs() <= 0.04, "hinge {bar} is bent {bend}");
			let off = (body.to_world(bar_end(-1.0)) - joint).length();
			assert!(off <= 0.03, "hinge {bar} is {off} apart");
			assert!(
				body.linear_velocity.length() <= 0.05,
				"bar {bar} moves at {}",
				body.linear_velocity.length()
			);
			(before, joint) = (turn_about_z(body), body.to_world(bar_end(1.0)));
		}
	}

	#[test]
	fn a_limb_of_hinged_bars_that_falls_on_the_ground_comes_to_rest_in_one_piece() {
		let mut scene = Constrained::on(1);
		let mut ground = make_ground_body();
		ground.friction = 0.5;
		scene.add(ground);
		// Three bars in a row, each hinged to the one before it, that fall leaning, so that one end lands first and
		// the ground folds the limb against its hinges.
		let lean = Orientation::try_from_rotation_vector(Vector::<math::WorldSpace>::new(0.0, 0.0, 0.9)).unwrap();
		let foot = math::Point::new(-1.0, 1.6, 0.0);
		let limit = || math::Radians::new(-0.5)..=math::Radians::new(0.5);
		let mut bars: Vec<Handle> = Vec::new();
		for bar in 0..3 {
			let mut body = make_bar(foot + lean.rotate_vector(Vector::new(0.5 + bar as f32, 0.0, 0.0)));
			body.orientation = lean;
			let handle = scene.add(body);
			if let Some(&before) = bars.last() {
				scene.join(
					Constraint::hinge(before, handle, math::UnitVector::z_axis(), Some(limit()))
						.with_anchors(bar_end(1.0), bar_end(-1.0)),
				);
			}
			bars.push(handle);
		}

		scene.run(360);

		for (bar, pair) in bars.windows(2).enumerate() {
			let (before, after) = (scene.body(pair[0]), scene.body(pair[1]));
			let off = (after.to_world(bar_end(-1.0)) - before.to_world(bar_end(1.0))).length();
			assert!(off <= 0.03, "hinge {bar} is {off} apart");
			let bend = turn_about_z(after) - turn_about_z(before);
			assert!(bend.abs() <= 0.53, "hinge {bar} is bent {bend}");
		}
		for (bar, &handle) in bars.iter().enumerate() {
			let body = scene.body(handle);
			// The ground's top is a meter up, and a bar that lies on it has its middle 5 cm above that.
			assert!(
				(body.position.y() - 1.05).abs() <= 0.02,
				"bar {bar} rests at {}",
				body.position.y()
			);
			assert!(
				body.linear_velocity.length() <= 0.05,
				"bar {bar} moves at {}",
				body.linear_velocity.length()
			);
		}
	}

	/// Returns a world with a bar that a joint holds by one end to a point that nothing moves, with the bar's handle
	/// and that point. The bar starts turned by `turned`, and with the `linear` and `angular` velocities. `joint`
	/// returns the joint between the point and the bar, in that sequence, about an axis along the bar's length.
	fn make_jointed_bar(
		turned: Orientation,
		linear: Vector,
		angular: Vector,
		joint: impl FnOnce(Handle, Handle, math::UnitVector<LocalSpace>) -> Constraint,
	) -> (Constrained, Handle, math::Point) {
		let mut scene = Constrained::on(1);
		let pivot = math::Point::new(0.0, 5.0, 0.0);
		let post = scene.add(make_fixed_point(pivot));
		let length = turned.rotate_vector(Vector::<math::WorldSpace>::new(1.0, 0.0, 0.0));
		let mut bar = make_bar(pivot + length * 0.5);
		(bar.orientation, bar.linear_velocity, bar.angular_velocity) = (turned, linear, angular);
		let bar = scene.add(bar);
		// The point is not turned, so its directions are the world's.
		let axis = Vector::<LocalSpace>::from_array(length.to_array()).normalized().unwrap();
		scene.join(joint(post, bar, axis).with_anchors(math::Point::origin(), bar_end(-1.0)));
		(scene, bar, pivot)
	}

	/// Returns how far, in radians, a bar has turned about its length since it was turned by `turned`.
	fn twist(bar: &PhysicsBody, turned: Orientation) -> f32 {
		// The turn since then is one about the bar's length then, followed by one that leans it over. A unit
		// quaternion holds the first of them as its part along that length and its scalar part.
		let length = turned.rotate_vector(Vector::<math::WorldSpace>::new(1.0, 0.0, 0.0));
		let [x, y, z, w] = bar.orientation.compose(turned.inverse()).to_array();
		2.0 * (Vector::new(x, y, z).dot(length) * w.signum()).atan2(w.abs())
	}

	/// Returns how a bar is turned that hangs down from the end that these tests hold it by.
	fn hanging() -> Orientation {
		Orientation::try_from_rotation_vector(Vector::<math::WorldSpace>::new(0.0, 0.0, -FRAC_PI_2)).unwrap()
	}

	/// Returns how far, in radians, the length of a bar leans away from where it pointed when the bar was turned
	/// by `turned`.
	fn lean(bar: &PhysicsBody, turned: Orientation) -> f32 {
		let length = |turned: Orientation| turned.rotate_vector(Vector::<math::WorldSpace>::new(1.0, 0.0, 0.0));
		length(bar.orientation).dot(length(turned)).clamp(-1.0, 1.0).acos()
	}

	#[test]
	fn a_constant_velocity_joint_lets_its_body_lean_any_way_but_not_twist() {
		// A bar that hangs down, and a knock that swings it out and spins it about its length.
		let hanging = hanging();
		let (knock, spin) = (Vector::new(1.5, 0.0, 1.5), Vector::new(0.0, 6.0, 0.0));
		let joint = |post, bar, axis| Constraint::constant_velocity(post, bar, axis, None);
		let (mut scene, bar, pivot) = make_jointed_bar(hanging, knock, spin, joint);

		let mut furthest = [0.0f32; 2];
		for step in 0..180 {
			scene.run(1);
			let bar = scene.body(bar);
			let off = (bar.to_world(bar_end(-1.0)) - pivot).length();
			assert!(off <= 0.05, "the bar's end is {off} from the joint in step {step}");
			let twist = twist(bar, hanging);
			assert!(twist.abs() <= 0.03, "the bar is twisted {twist} in step {step}");
			furthest = [
				furthest[0].max(bar.position.x().abs()),
				furthest[1].max(bar.position.z().abs()),
			];
		}
		// The knock swings the bar out both ways, as nothing stops either.
		assert!(
			furthest[0] >= 0.15 && furthest[1] >= 0.15,
			"the bar only swung {furthest:?} out"
		);
	}

	#[test]
	fn a_constant_velocity_joint_stops_its_body_at_its_limit() {
		let level = Orientation::identity();
		let joint = |post, bar, axis| Constraint::constant_velocity(post, bar, axis, Some(math::Radians::new(0.5)));
		let (mut scene, bar, pivot) = make_jointed_bar(level, Vector::zero(), Vector::zero(), joint);

		scene.run(240);

		// Gravity leans a bar that sticks out to the side down as far as the joint lets it, where it rests.
		let bar = scene.body(bar);
		assert!(
			(lean(bar, level) - 0.5).abs() <= 0.03,
			"the bar rests leaning {}",
			lean(bar, level)
		);
		assert!(bar.position.y() < pivot.y() && bar.position.z().abs() <= 0.01);
		assert!(bar.angular_velocity.length() <= 0.05);
		assert!((bar.to_world(bar_end(-1.0)) - pivot).length() <= 0.03);
	}

	#[test]
	fn a_constant_velocity_joint_stops_its_body_as_far_to_every_side() {
		// A knock that swings a bar that sticks out to the side further aside while gravity swings it down, so that
		// it comes to the limit somewhere between the two and is carried on around it.
		let level = Orientation::identity();
		let joint = |post, bar, axis| Constraint::constant_velocity(post, bar, axis, Some(math::Radians::new(0.5)));
		let (mut scene, bar, _) = make_jointed_bar(level, Vector::new(0.0, 0.0, 3.0), Vector::zero(), joint);

		let (mut furthest, mut furthest_aside) = (0.0f32, 0.0f32);
		for step in 0..300 {
			scene.run(1);
			let bar = scene.body(bar);
			// A bar that is carried around the limit this fast is past it by what a step's share of the way back
			// does not take up.
			assert!(lean(bar, level) <= 0.55, "the bar leans {} in step {step}", lean(bar, level));
			furthest = furthest.max(lean(bar, level));
			furthest_aside = furthest_aside.max(bar.position.z().abs());
		}
		assert!(furthest >= 0.47, "the bar only leaned {furthest}");
		// Half a meter out along a bar that leans as far as it may straight to the side is 24 cm aside.
		assert!(furthest_aside >= 0.15, "the bar only went {furthest_aside} aside");
	}

	#[test]
	fn a_ball_joint_lets_its_body_turn_every_way() {
		// A bar that hangs down, and a knock that swings it out and spins it about its length.
		let hanging = hanging();
		let (knock, spin) = (Vector::new(1.5, 0.0, 1.5), Vector::new(0.0, 6.0, 0.0));
		let joint = |post, bar, axis| Constraint::ball(post, bar, axis, None, None);
		let (mut scene, bar, pivot) = make_jointed_bar(hanging, knock, spin, joint);

		let mut furthest = [0.0f32; 2];
		for step in 0..180 {
			scene.run(1);
			let bar = scene.body(bar);
			let off = (bar.to_world(bar_end(-1.0)) - pivot).length();
			assert!(off <= 0.05, "the bar's end is {off} from the joint in step {step}");
			furthest = [
				furthest[0].max(bar.position.x().abs()),
				furthest[1].max(bar.position.z().abs()),
			];
		}
		// The knock swings the bar out both ways and it goes on spinning, as nothing stops any of it.
		assert!(
			furthest[0] >= 0.15 && furthest[1] >= 0.15,
			"the bar only swung {furthest:?} out"
		);
		let bar = scene.body(bar);
		let length = bar.orientation.rotate_vector(Vector::<math::WorldSpace>::new(1.0, 0.0, 0.0));
		let spin = bar.angular_velocity.dot(length).abs();
		assert!(spin >= 5.0, "the bar only spins at {spin}");
	}

	#[test]
	fn a_ball_joint_stops_its_body_at_its_limits() {
		// A bar that sticks out to the side and spins about its length, one way and the other. Gravity leans it
		// down as far as the joint lets it, and the spin turns it as far as the joint lets it that way.
		for (spin, stop) in [(6.0, 0.6), (-6.0, -0.4)] {
			let level = Orientation::identity();
			let joint = |post, bar, axis| {
				let twist = math::Radians::new(-0.4)..=math::Radians::new(0.6);
				Constraint::ball(post, bar, axis, Some(math::Radians::new(0.5)), Some(twist))
			};
			let (mut scene, bar, pivot) = make_jointed_bar(level, Vector::zero(), Vector::new(spin, 0.0, 0.0), joint);

			// Nothing holds the bar at the end of the limit that the spin takes it to, so that is only as far as it
			// gets.
			let mut furthest = 0.0f32;
			for _ in 0..240 {
				scene.run(1);
				let twist = twist(scene.body(bar), level);
				furthest = if twist.abs() > furthest.abs() { twist } else { furthest };
			}
			assert!(
				(furthest - stop).abs() <= 0.03,
				"the bar turned as far as {furthest}, not {stop}"
			);

			let bar = scene.body(bar);
			assert!(
				(lean(bar, level) - 0.5).abs() <= 0.03,
				"the bar rests leaning {}",
				lean(bar, level)
			);
			assert!((bar.to_world(bar_end(-1.0)) - pivot).length() <= 0.03);
		}
	}

	/// Returns a dynamic box of `mass` kilograms that contacts and constraints turn, with its middle at `position`
	/// and sides twice as long as `half`.
	fn make_part(position: math::Point, half: [f32; 3], mass: f32) -> PhysicsBody {
		let mut body = make_rotating_box_body(position);
		body.collision_shape = Shapes::Cube {
			size: Vector::from_array(half),
		};
		body.inv_mass = 1.0 / mass;
		body
	}

	#[test]
	fn a_ragdoll_that_falls_over_comes_to_rest_on_the_ground_in_one_piece() {
		let mut scene = Constrained::on(1);
		let mut ground = make_ground_body();
		ground.friction = 0.5;
		scene.add(ground);

		// A figure that stands on the ground with its arms out to the sides, 1.9 meters tall. Its parts leave gaps
		// at the joints, wide enough that two jointed parts do not touch however far the joint lets them turn.
		let feet = 1.02;
		let place = |x: f32, y: f32| math::Point::new(x, feet + y, 0.0);
		let radians = math::Radians::new;
		let (x, y) = (
			Vector::<LocalSpace>::new(1.0, 0.0, 0.0),
			Vector::<LocalSpace>::new(0.0, 1.0, 0.0),
		);
		let axis = |along: Vector<LocalSpace>| along.normalized().unwrap();

		let mut torso = make_part(place(0.0, 1.3), [0.2, 0.3, 0.1], 10.0);
		// A push that topples the figure forward and to the side.
		torso.linear_velocity = Vector::new(0.5, 0.0, 1.5);
		let torso = scene.add(torso);
		let mut parts = vec![torso];
		// Each joint, as the two parts it joins and where it is.
		let mut joints: Vec<(Handle, Handle, math::Point)> = Vec::new();
		let mut join = |scene: &mut Constrained, at: math::Point, joint: Constraint| {
			let [first, second] = joint.bodies();
			let anchor = |handle| {
				math::Point::<LocalSpace>::origin() + Vector::from_array((at - scene.body(handle).position).to_array())
			};
			scene.join(joint.with_anchors(anchor(first), anchor(second)));
			joints.push((first, second, at));
		};

		let head = scene.add(make_part(place(0.0, 1.8), [0.1, 0.1, 0.1], 4.0));
		parts.push(head);
		let neck = Constraint::ball(torso, head, axis(y), Some(radians(0.4)), Some(radians(-0.5)..=radians(0.5)));
		join(&mut scene, place(0.0, 1.65), neck);

		for side in [-1.0, 1.0] {
			let upper_arm = scene.add(make_part(place(side * 0.5, 1.5), [0.15, 0.05, 0.05], 2.0));
			let forearm = scene.add(make_part(place(side * 0.9, 1.5), [0.15, 0.05, 0.05], 2.0));
			let thigh = scene.add(make_part(place(side * 0.1, 0.7), [0.05, 0.2, 0.05], 2.0));
			let shin = scene.add(make_part(place(side * 0.1, 0.2), [0.05, 0.2, 0.05], 2.0));
			parts.extend([upper_arm, forearm, thigh, shin]);

			let bend = || Some(radians(0.0)..=radians(1.4));
			let shoulder = Constraint::ball(
				torso,
				upper_arm,
				axis(x * side),
				Some(radians(1.2)),
				Some(radians(-0.6)..=radians(0.6)),
			);
			join(&mut scene, place(side * 0.275, 1.5), shoulder);
			join(
				&mut scene,
				place(side * 0.7, 1.5),
				Constraint::hinge(upper_arm, forearm, axis(y), bend()),
			);
			let hip = Constraint::constant_velocity(torso, thigh, axis(-y), Some(radians(1.0)));
			join(&mut scene, place(side * 0.1, 0.95), hip);
			join(
				&mut scene,
				place(side * 0.1, 0.45),
				Constraint::hinge(thigh, shin, axis(x), bend()),
			);
		}

		// Where each joint is from the middle of each of its two parts, which have not turned yet.
		let reaches: Vec<[Vector; 2]> = (joints.iter())
			.map(|&(first, second, at)| [first, second].map(|handle| at - scene.body(handle).position))
			.collect();
		// Returns how far apart the two parts of the joint that is furthest apart have it.
		let furthest_apart = |scene: &Constrained| {
			let apart = joints.iter().zip(&reaches).map(|(&(first, second, _), reaches)| {
				let [first, second] = [(first, reaches[0]), (second, reaches[1])].map(|(handle, reach)| {
					let body = scene.body(handle);
					body.position + body.orientation.rotate_vector(reach)
				});
				(second - first).length()
			});
			apart.fold(0.0, f32::max)
		};

		let mut furthest_ever = 0.0f32;
		for _ in 0..480 {
			scene.run(1);
			furthest_ever = furthest_ever.max(furthest_apart(&scene));
		}

		// The knees give the most, when the legs take the weight of the figure, which is eleven times theirs.
		assert!(furthest_ever <= 0.1, "a joint came {furthest_ever} apart");
		assert!(
			furthest_apart(&scene) <= 0.03,
			"a joint rests {} apart",
			furthest_apart(&scene)
		);
		for (part, &handle) in parts.iter().enumerate() {
			let body = scene.body(handle);
			// The ground's top is a meter up, and the thinnest part lies with its middle 5 cm above that.
			let height = body.position.y();
			assert!((1.03..=1.25).contains(&height), "part {part} rests at {height}");
			let speed = body.linear_velocity.length();
			assert!(speed <= 0.05, "part {part} moves at {speed}");
		}
	}

	#[test]
	fn a_joint_with_resistance_gives_way_until_the_resistance_holds_it() {
		// Gravity of 16 leans a bar of a kilogram that sticks out to the side down, with 8 newton meters at first.
		// A resistance that grows with the angle has taken up what the bar gains by leaning where five times the
		// angle squared, for a resistance of ten newton meters a radian, is 8 times the sine of the angle. The bar
		// stays there, as the resistance is more than gravity's pull by then. A stiffer joint stops sooner.
		for (stiffness, stop) in [(10.0, 1.21), (40.0, 0.39)] {
			let level = Orientation::identity();
			let joint = |post, bar, axis| {
				Constraint::ball(post, bar, axis, None, None).with_resistance(move |turned| stiffness * turned.value())
			};
			let (mut scene, bar, pivot) = make_jointed_bar(level, Vector::zero(), Vector::zero(), joint);

			scene.run(240);

			let bar = scene.body(bar);
			assert!(
				(lean(bar, level) - stop).abs() <= 0.05,
				"the bar rests leaning {}",
				lean(bar, level)
			);
			assert!(bar.angular_velocity.length() <= 0.05);
			assert!((bar.to_world(bar_end(-1.0)) - pivot).length() <= 0.03);
		}
	}

	#[test]
	fn a_joint_with_resistance_lets_its_body_turn_back() {
		// A bar that hangs down and is knocked into a swing, with and without a joint that resists.
		let furthest_and_last = |resistance: f32| {
			let hanging = hanging();
			let joint = |post, bar, axis| {
				Constraint::ball(post, bar, axis, None, None).with_resistance(move |turned| resistance * turned.value())
			};
			let (mut scene, bar, _) = make_jointed_bar(hanging, Vector::new(2.0, 0.0, 0.0), Vector::zero(), joint);
			let mut furthest = 0.0f32;
			for _ in 0..600 {
				scene.run(1);
				furthest = furthest.max(lean(scene.body(bar), hanging));
			}
			(furthest, lean(scene.body(bar), hanging))
		};

		let (free, _) = furthest_and_last(0.0);
		let (furthest, last) = furthest_and_last(5.0);

		// The resistance stops the bar short of where it swings to freely, and does not hold it there: gravity
		// brings the bar back, to swing less far every time.
		assert!(furthest <= free - 0.1, "the bar swung out {furthest}, and {free} freely");
		assert!(last <= 0.05, "the bar ends leaning {last}");
	}

	/// The `Resting` struct holds a world on level ground that its test fills and disturbs as an application would,
	/// through the world's factories and channels, and tells what the world's steps publish.
	struct Resting {
		world: World,
		bodies: Factory<Body>,
		constraints: Factory<Constraint>,
		deletes: DefaultChannel<DeleteMessage>,
		authoring: DefaultChannel<TransformationUpdate>,
		authored: DefaultListener<TransformationUpdate>,
		published: DefaultChannel<TransformationUpdate>,
		reader: DefaultListener<TransformationUpdate>,
		alley: Alley,
		/// The ground, whose top is at height zero.
		ground: Handle,
	}

	impl Resting {
		/// Returns a world with ground, whose steps run on `lanes` lanes.
		fn on(lanes: usize) -> Self {
			let bodies = Factory::<Body>::new();
			let constraints = Factory::<Constraint>::new();
			let deletes = DefaultChannel::new();
			let authoring = DefaultChannel::new();
			let published = DefaultChannel::new();
			let mut world = World::new(
				bodies.listener(),
				constraints.listener(),
				deletes.listener(),
				Factory::<RayQuery>::new().listener(),
				Factory::new(),
			);
			world.set_step_lanes(StepLanes::new([(0, lanes)]));
			let mut scene = Self {
				world,
				authored: authoring.listener(),
				reader: published.listener(),
				bodies,
				constraints,
				deletes,
				authoring,
				published,
				alley: Alley::with_parallelism(lanes),
				ground: test_handle(),
			};
			let ground = Body::new(BodyTypes::Static, Shapes::cube(Vector::new(30.0, 1.0, 30.0)));
			scene.ground = scene.spawn(ground, math::Point::new(0.0, -1.0, 0.0));
			scene
		}

		/// Creates `body` at `position` and returns its handle. The world takes it up in its next step.
		fn spawn(&self, body: Body, position: math::Point) -> Handle {
			let handle = test_handle();
			self.bodies.derive(handle, body);
			self.place(handle, position);
			handle
		}

		/// Creates a one-meter box of one kilogram with its middle at `position`.
		fn spawn_box(&self, position: math::Point) -> Handle {
			self.spawn(
				Body::new(BodyTypes::Dynamic, Shapes::cube(Vector::new(0.5, 0.5, 0.5))),
				position,
			)
		}

		/// Moves the body that was created with `handle` to `position`, as its author.
		fn place(&self, handle: Handle, position: math::Point) {
			self.authoring
				.send(TransformationUpdate::new(handle, Transform::from_position(position)));
		}

		/// Deletes what was created with `handle`.
		fn delete(&mut self, handle: Handle) {
			self.deletes.send(DeleteMessage::new(handle));
			self.world.process_pending_deletions();
		}

		/// Advances the world by `steps` sixtieths of a second. Returns the handles of the bodies that the last of
		/// those steps published, which are the bodies that it moved.
		fn run(&mut self, steps: usize) -> Vec<Handle> {
			let dt = MediaTime::from_seconds_f32(1.0 / 60.0);
			let mut moved = Vec::new();
			for _ in 0..steps {
				self.world
					.update(Time::new(dt, dt), &mut self.authored, &self.published, &mut self.alley);
				moved.clear();
				while let Some(update) = self.reader.read() {
					moved.push(update.handle());
				}
			}
			moved
		}

		/// Advances the world until a step moves no body, which is when every body that can come to rest sleeps.
		/// Fails the test when that takes longer than `seconds`.
		fn run_to_rest(&mut self, seconds: usize) {
			let rested = (0..seconds * 60).any(|_| self.run(1).is_empty());
			assert!(rested, "the bodies are still simulated after {seconds} seconds");
		}

		/// Returns the body that was created with `handle`.
		fn body(&self, handle: Handle) -> &PhysicsBody {
			&self.world.bodies.all()[self.world.handles_to_bodies[&handle]]
		}

		/// Returns how high the origin of the body that was created with `handle` is.
		fn height(&self, handle: Handle) -> f32 {
			self.body(handle).position.y()
		}
	}

	#[test]
	fn a_body_that_has_come_to_rest_sleeps_where_it_lies() {
		let mut scene = Resting::on(1);
		let dropped = scene.spawn_box(math::Point::new(0.0, 3.0, 0.0));

		// While it falls and settles, every step moves the box.
		assert_eq!(scene.run(30), [dropped]);
		scene.run_to_rest(4);

		let at_rest = scene.body(dropped).position;
		assert!((at_rest.y() - 0.5).abs() <= 0.01, "the box rests at {}", at_rest.y());
		assert_eq!(scene.body(dropped).linear_velocity.length(), 0.0);
		// Nothing moves it from then on, and gravity does not pull it into the ground.
		assert!(scene.run(600).is_empty());
		assert_eq!(scene.body(dropped).position, at_rest);
	}

	#[test]
	fn a_body_that_keeps_moving_stays_awake() {
		let mut scene = Resting::on(1);
		// A ball that rolls slowly, which nothing stops on level ground.
		let ball = Body::new(BodyTypes::Dynamic, Shapes::sphere(0.5)).with_velocity(Vector::new(0.3, 0.0, 0.0));
		let ball = scene.spawn(ball, math::Point::new(-10.0, 0.5, 0.0));

		for second in 1..=10 {
			assert_eq!(scene.run(60), [ball], "after {second} seconds");
		}
		assert!(scene.body(ball).position.x() >= -8.0);
	}

	#[test]
	fn a_body_that_swings_stays_awake_however_slow_it_is_where_it_turns() {
		let mut scene = Resting::on(1);
		let post = scene.spawn(
			Body::new(BodyTypes::Static, Shapes::sphere(0.05)),
			math::Point::new(0.0, 6.0, 0.0),
		);
		let bob = scene.spawn(
			Body::new(BodyTypes::Dynamic, Shapes::sphere(0.1)),
			math::Point::new(1.0, 6.0, 0.0),
		);
		scene.constraints.create(Constraint::distance(post, bob, 1.0));

		// The bob stands still for a moment at each end of every swing.
		for second in 1..=20 {
			assert_eq!(scene.run(60), [bob], "after {second} seconds");
		}
	}

	#[test]
	fn a_body_that_lands_on_a_sleeping_body_pushes_it_no_further_than_one_that_is_awake() {
		// Returns how low a box on the ground is pushed by a box that lands on it at eight meters a second.
		let lowest = |sleeps: bool| {
			let mut scene = Resting::on(1);
			scene.world.set_sleeping(sleeps);
			let lower = scene.spawn_box(math::Point::new(0.0, 0.5, 0.0));
			scene.run(240);
			assert_eq!(scene.run(1).is_empty(), sleeps);

			let upper = scene.spawn_box(math::Point::new(0.0, 3.0, 0.0));
			let mut lowest = f32::MAX;
			for _ in 0..120 {
				scene.run(1);
				lowest = lowest.min(scene.height(lower));
			}
			assert!(
				(scene.height(upper) - 1.5).abs() <= 0.02,
				"the upper box is at {}",
				scene.height(upper)
			);
			lowest
		};

		// The sleeping box wakes in the step that the other one reaches it in, with the ground under it, so it
		// gives way as little as a box that never slept. Left without the ground for that step, it would sink
		// several centimeters.
		let (awake, asleep) = (lowest(false), lowest(true));
		assert!(awake >= 0.5 - 0.02, "the box that was awake was pushed down to {awake}");
		assert!(
			asleep >= awake - 0.002,
			"the sleeping box was pushed down to {asleep}, the other to {awake}"
		);
	}

	#[test]
	fn a_body_that_hits_a_sleeping_body_knocks_it_away() {
		let mut scene = Resting::on(1);
		let target = scene.spawn_box(math::Point::new(0.0, 0.5, 0.0));
		scene.run_to_rest(4);

		let ball = Body::new(BodyTypes::Dynamic, Shapes::sphere(0.5)).with_velocity(Vector::new(10.0, 0.0, 0.0));
		scene.spawn(ball, math::Point::new(-4.0, 0.5, 0.0));
		scene.run(60);

		// The box takes up the ball's momentum, as it would had it been awake.
		assert!(
			scene.body(target).position.x() >= 1.0,
			"the box is at {}",
			scene.body(target).position.x()
		);
	}

	#[test]
	fn pushing_a_sleeping_body_wakes_what_lies_on_it() {
		let mut scene = Resting::on(1);
		let stack = [0.5, 1.5, 2.5].map(|height| scene.spawn_box(math::Point::new(0.0, height, 0.0)));
		scene.run_to_rest(6);

		// The lowest box is knocked out from under the others.
		assert!(scene.world.apply_impulse(stack[0], Vector::new(30.0, 0.0, 0.0)));
		scene.run(90);

		assert!(scene.body(stack[0]).position.x() >= 2.0);
		assert!(
			scene.height(stack[2]) <= 2.0,
			"the top box is still at {}",
			scene.height(stack[2])
		);
	}

	#[test]
	fn deleting_a_sleeping_body_lets_what_lies_on_it_fall() {
		let mut scene = Resting::on(1);
		let stack = [0.5, 1.5, 2.5].map(|height| scene.spawn_box(math::Point::new(0.0, height, 0.0)));
		scene.run_to_rest(6);

		scene.delete(stack[0]);
		scene.run(120);

		assert!((scene.height(stack[1]) - 0.5).abs() <= 0.02 && (scene.height(stack[2]) - 1.5).abs() <= 0.03);
	}

	#[test]
	fn sleeping_bodies_fall_when_what_they_lie_on_is_moved_or_deleted() {
		// A shelf without mass, with a box asleep on it, two meters above the ground.
		let shelved = || {
			let mut scene = Resting::on(1);
			let shelf = Body::new(BodyTypes::Static, Shapes::cube(Vector::new(1.0, 0.1, 1.0)));
			let shelf = scene.spawn(shelf, math::Point::new(0.0, 1.9, 0.0));
			let lying = scene.spawn_box(math::Point::new(0.0, 2.5, 0.0));
			scene.run_to_rest(4);
			assert!((scene.height(lying) - 2.5).abs() <= 0.01);
			(scene, shelf, lying)
		};

		let (mut scene, shelf, lying) = shelved();
		scene.delete(shelf);
		scene.run(120);
		assert!(
			(scene.height(lying) - 0.5).abs() <= 0.02,
			"the box is at {}",
			scene.height(lying)
		);

		let (mut scene, shelf, lying) = shelved();
		scene.place(shelf, math::Point::new(5.0, 1.9, 0.0));
		scene.run(120);
		assert!(
			(scene.height(lying) - 0.5).abs() <= 0.02,
			"the box is at {}",
			scene.height(lying)
		);
	}

	#[test]
	fn a_body_without_mass_that_is_moved_into_a_sleeping_body_pushes_it_away() {
		let mut scene = Resting::on(1);
		let lying = scene.spawn_box(math::Point::new(0.0, 0.5, 0.0));
		scene.run_to_rest(4);

		// A wall is put where it reaches a little into the box from the side.
		let wall = Body::new(BodyTypes::Static, Shapes::cube(Vector::new(0.5, 2.0, 2.0)));
		scene.spawn(wall, math::Point::new(-0.9, 2.0, 0.0));
		scene.run(120);

		assert!(
			scene.body(lying).position.x() >= 0.09,
			"the box is at {}",
			scene.body(lying).position.x()
		);
	}

	#[test]
	fn a_sleeping_body_that_its_author_moves_falls_from_where_it_is_put() {
		let mut scene = Resting::on(1);
		let stack = [0.5, 1.5].map(|height| scene.spawn_box(math::Point::new(0.0, height, 0.0)));
		scene.run_to_rest(6);

		scene.place(stack[0], math::Point::new(4.0, 3.0, 0.0));
		scene.run(120);

		// The box that was moved lands where it was put, and the one that lay on it lands where it was.
		assert!((scene.height(stack[0]) - 0.5).abs() <= 0.02 && (scene.body(stack[0]).position.x() - 4.0).abs() <= 0.02);
		assert!(
			(scene.height(stack[1]) - 0.5).abs() <= 0.02,
			"the upper box is at {}",
			scene.height(stack[1])
		);
	}

	#[test]
	fn a_transform_that_moves_nothing_wakes_nothing() {
		let mut scene = Resting::on(1);
		let lying = scene.spawn_box(math::Point::new(0.0, 0.5, 0.0));
		scene.run_to_rest(4);

		// An author that repeats where a body is, as one that writes every transform every tick does.
		let (body, ground) = (scene.body(lying), scene.ground);
		let at_rest = Transform::new(body.position, body.scale, body.orientation);
		for _ in 0..10 {
			scene.authoring.send(TransformationUpdate::new(lying, at_rest.clone()));
			scene.place(ground, math::Point::new(0.0, -1.0, 0.0));
			assert!(scene.run(1).is_empty());
		}
	}

	#[test]
	fn constraints_wake_the_sleeping_bodies_that_they_take_hold_of_or_let_go() {
		let mut scene = Resting::on(1);
		let post = scene.spawn(
			Body::new(BodyTypes::Static, Shapes::sphere(0.05)),
			math::Point::new(0.0, 6.0, 0.0),
		);
		let bob = scene.spawn(
			Body::new(BodyTypes::Dynamic, Shapes::sphere(0.1)),
			math::Point::new(0.0, 5.0, 0.0),
		);
		let rod = scene.constraints.create(Constraint::distance(post, bob, 1.0));
		let lying = scene.spawn_box(math::Point::new(3.0, 0.5, 0.0));
		// The bob hangs straight down from the start, so it and the box both come to rest.
		scene.run_to_rest(4);
		assert!((scene.height(bob) - 5.0).abs() <= 0.01);

		// A rod that is shorter than the box is far lifts the box off the ground.
		let lift = scene.constraints.create(Constraint::distance(post, lying, 5.0));
		scene.run(120);
		assert!(scene.height(lying) >= 0.7, "the box is at {}", scene.height(lying));

		// Without their rods, both fall.
		scene.delete(lift);
		scene.delete(rod);
		scene.run(180);
		assert!(scene.height(bob) <= 0.2 && scene.height(lying) <= 0.6);
	}

	#[test]
	fn a_body_does_not_sleep_while_a_body_that_touches_it_moves() {
		let mut scene = Resting::on(1);
		// A long plank that lies on the ground, with a ball that rolls along the top of it.
		let plank = Body::new(BodyTypes::Dynamic, Shapes::cube(Vector::new(6.0, 0.25, 1.0))).with_mass(50.0);
		let plank = scene.spawn(plank, math::Point::new(0.0, 0.25, 0.0));
		let ball = Body::new(BodyTypes::Dynamic, Shapes::sphere(0.25)).with_velocity(Vector::new(2.0, 0.0, 0.0));
		let ball = scene.spawn(ball, math::Point::new(-5.5, 0.75, 0.0));

		// The plank hardly moves under the ball, but it is simulated for as long as the ball is on it.
		for _ in 0..4 {
			let moved = scene.run(60);
			assert!(moved.contains(&plank) && moved.contains(&ball));
		}
		assert!((scene.body(plank).position.x()).abs() <= 0.5);

		// Once the ball has rolled off its end, the plank sleeps, though the ball rolls on.
		scene.run(300);
		assert!(scene.body(ball).position.x() >= 7.0);
		assert_eq!(scene.run(1), [ball]);
	}

	#[test]
	fn bodies_that_a_moving_body_has_left_sleep_while_it_moves_on_the_others() {
		let mut scene = Resting::on(1);
		// A row of boxes that touch side by side, with a ball that rolls along their tops.
		let row: Vec<Handle> = (0..12)
			.map(|column| scene.spawn_box(math::Point::new(column as f32 - 5.5, 0.5, 0.0)))
			.collect();
		scene.run_to_rest(4);
		let ball = Body::new(BodyTypes::Dynamic, Shapes::sphere(0.25)).with_velocity(Vector::new(2.0, 0.0, 0.0));
		let ball = scene.spawn(ball, math::Point::new(-5.5, 1.25, 0.0));

		// Once friction makes the ball roll, it is slower than it starts. After three seconds it is over the middle
		// of the row, and the first boxes, which it left more than a second ago, sleep.
		let moved = scene.run(180);
		assert!(
			moved.contains(&ball),
			"the ball is at rest at {}",
			scene.body(ball).position.x()
		);
		assert!(
			(-2.5..=2.5).contains(&scene.body(ball).position.x()),
			"the ball is at {}",
			scene.body(ball).position.x()
		);
		assert!(
			row[..3].iter().all(|lying| !moved.contains(lying)),
			"the boxes the ball left are simulated"
		);
	}

	#[test]
	fn a_world_that_keeps_its_bodies_awake_simulates_them_all() {
		let mut scene = Resting::on(1);
		let lying = scene.spawn_box(math::Point::new(0.0, 0.5, 0.0));
		scene.run_to_rest(4);

		scene.world.set_sleeping(false);
		for _ in 0..3 {
			assert_eq!(scene.run(60), [lying]);
		}
		assert!((scene.height(lying) - 0.5).abs() <= 0.01);

		scene.world.set_sleeping(true);
		scene.run_to_rest(2);
	}

	/// Returns where every body of a pile is once the pile has gone to sleep and a ball has been shot into it, in
	/// a world whose steps run on `lanes` lanes.
	fn woken_pile(lanes: usize) -> Vec<math::Point> {
		let mut scene = Resting::on(lanes);
		// Enough boxes side by side for the lanes to share their contacts with the ground, with a second layer on
		// some of them.
		let mut boxes = Vec::new();
		for index in 0..CROWD_SIDE * CROWD_SIDE {
			let (column, row) = ((index % CROWD_SIDE) as f32, (index / CROWD_SIDE) as f32);
			let position = math::Point::new(column * 1.02 - 8.0, 0.5, row * 1.02 - 8.0);
			boxes.push(scene.spawn_box(position));
			if index % 5 == 0 {
				boxes.push(scene.spawn_box(position + Vector::new(0.0, 1.0, 0.0)));
			}
		}
		scene.run_to_rest(10);

		let ball = Body::new(BodyTypes::Dynamic, Shapes::sphere(0.5))
			.with_mass(20.0)
			.with_velocity(Vector::new(15.0, 0.0, 4.0));
		boxes.push(scene.spawn(ball, math::Point::new(-12.0, 0.5, -3.0)));
		scene.run(90);
		boxes.iter().map(|handle| scene.body(*handle).position).collect()
	}

	#[test]
	fn sleeping_bodies_wake_the_same_on_any_number_of_lanes() {
		let on_one = woken_pile(1);
		// The ball went through the pile, and what it hit moved.
		assert!(
			on_one
				.iter()
				.filter(|position| position.y() > 0.6 || position.y() < 0.4)
				.count() >= 10
		);

		for lanes in [2, 4] {
			assert_eq!(woken_pile(lanes), on_one, "on {lanes} lanes");
		}
	}

	#[test]
	fn boxes_stacked_on_a_slope_that_friction_holds_them_on_stay_where_they_are() {
		// Ground that is tilted by half of what the boxes slide on it and on each other from.
		let tilt = Orientation::try_from_rotation_vector(Vector::<math::WorldSpace>::new(0.0, 0.0, -0.125)).unwrap();
		let mut ground = make_ground_body();
		ground.friction = 0.5;
		ground.orientation = tilt;
		ground.rotates = true;
		let up = tilt.rotate_vector(Vector::<math::WorldSpace>::new(0.0, 1.0, 0.0));
		let boxes = (0..2).map(|layer| {
			let mut body = make_rotating_box_body(math::Point::origin() + up * (1.5 + layer as f32));
			body.orientation = tilt;
			body
		});
		let mut world = make_world();
		// Boxes that sleep would stay where they are whatever friction did.
		world.set_sleeping(false);
		*world.bodies.get_mut() = std::iter::once(ground).chain(boxes).collect();
		let dt = MediaTime::from_seconds_f32(1.0 / 60.0);
		step(&mut world, dt, 120, &DefaultChannel::new());
		let settled: Vec<math::Point> = world.bodies.all().iter().map(|body| body.position).collect();
		step(&mut world, dt, 600, &DefaultChannel::new());

		// The upper box is only held by friction that the lower one passes on to the ground. Friction that started
		// every step from nothing let both slide more than ten centimeters in these ten seconds.
		for (layer, (body, settled)) in world.bodies.all().iter().zip(settled).enumerate().skip(1) {
			let slid = (body.position - settled).length();
			assert!(slid <= 0.01, "box {layer} slid {slid} in ten seconds");
		}
	}
}
