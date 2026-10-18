//! Per-skeleton graph playback, pose evaluation, and root-motion extraction.

use super::*;

mod stream;

/// The `RuntimeMotion` struct plays one graph state, keeping a blend's clips in step through one shared phase.
///
/// A clip state is a motion with one sample. A blend state weighs its samples by the requested blend position, and
/// plays for their durations weighed the same way, so a mix of a short and a long cycle lasts in between.
#[derive(Clone, Copy)]
struct RuntimeMotion<'graph> {
	state: AnimationStateId,
	/// The state's clips as the graph binding resolved them, shared with every player of the graph.
	samples: &'graph [ClipBinding],
	playback: AnimationPlayback,
	/// The samples that contribute and how much, summing to one.
	weights: BlendWeights,
	/// Progress through the clips, from 0 at their start to 1 at their end.
	phase: f32,
	/// The phase before the latest advance, which a blend's root motion measures from.
	previous_phase: f32,
}

impl<'graph> RuntimeMotion<'graph> {
	/// Plays a state's bound clips, weighed at `value`, which [`BlendPositions::recall`] gives for a blend.
	fn new(state: AnimationStateId, motion: &StateMotion, value: BlendValue, samples: &'graph [ClipBinding]) -> Self {
		let weights = motion.weights(value, BlendWeights::single(0));
		Self {
			state,
			samples,
			playback: motion.playback(),
			weights,
			phase: 0.0,
			previous_phase: 0.0,
		}
	}

	/// Returns how long one cycle lasts: the clips' durations weighed by how much each plays.
	fn duration(&self) -> f32 {
		if let [sample] = self.samples {
			return sample.duration;
		}
		self.weights
			.iter()
			.map(|(sample, weight)| self.samples[sample].duration * weight)
			.sum()
	}

	/// Returns the phase at which a state's clip reaches `point`, or `None` when a frame point names a clip without a
	/// frame rate.
	fn phase_of(&self, point: ClipPoint) -> Option<f32> {
		let sample = &self.samples[point.clip];
		let seconds = match point.time {
			ClipTime::Seconds(seconds) => seconds,
			ClipTime::Frame(frame) => frame as f32 / sample.frame_rate.or_else(|| warn_no_frame_rate(sample))?,
		};
		if seconds > sample.duration {
			warn_past_clip_end(sample, seconds);
		}
		Some(if sample.duration > 0.0 {
			(seconds / sample.duration).clamp(0.0, 1.0)
		} else {
			0.0
		})
	}

	/// Returns the points of the named clip that weighs the most, the first named among equals, so a route that names
	/// frames on several clips of a blend follows the clip that plays the most.
	fn points_of_heaviest<'points>(&self, points: &'points [ClipPoint]) -> impl Iterator<Item = &'points ClipPoint> {
		let heaviest = self.heaviest(points);
		points.iter().filter(move |point| Some(point.clip) == heaviest)
	}

	/// Returns the named clip of `points` that weighs the most, the first named among equals.
	fn heaviest(&self, points: &[ClipPoint]) -> Option<usize> {
		points
			.iter()
			.map(|point| point.clip)
			.reduce(|best, clip| if self.weight_of(clip) > self.weight_of(best) { clip } else { best })
	}

	/// Returns how much the clip at `clip` contributes, zero when it doesn't play.
	fn weight_of(&self, clip: usize) -> f32 {
		self.weights
			.iter()
			.find(|&(sample, _)| sample == clip)
			.map_or(0.0, |(_, weight)| weight)
	}

	/// Returns how long playback takes from now to reach the earliest of `points`, or `None` when it reaches none, like
	/// a one-shot already past them.
	fn time_until(&self, points: &[ClipPoint]) -> Option<MediaTime> {
		self.next_wait(points).map(|(time, _)| time)
	}

	/// Returns how long playback takes from now to reach the earliest of `points`, with that point.
	fn next_wait(&self, points: &[ClipPoint]) -> Option<(MediaTime, ClipPoint)> {
		let (ahead, point) = self
			.points_of_heaviest(points)
			.filter_map(|point| {
				let ahead = self.phase_of(*point)? - self.phase;
				let ahead = match self.playback {
					AnimationPlayback::Loop => Some(ahead.rem_euclid(1.0)),
					AnimationPlayback::Once => (ahead >= 0.0).then_some(ahead),
				}?;
				Some((ahead, *point))
			})
			.min_by(|(left, _), (right, _)| left.total_cmp(right))?;
		Some((MediaTime::from_seconds_f32(ahead * self.duration()), point))
	}

	fn is_finished(&self) -> bool {
		self.playback == AnimationPlayback::Once && self.phase >= 1.0
	}

	/// Returns the part of `delta` a one-shot motion leaves unused once it reaches its end, or `None` while it keeps
	/// playing.
	fn leftover_after(&self, delta: MediaTime) -> Option<MediaTime> {
		let duration = self.duration();
		if self.playback != AnimationPlayback::Once || duration <= 0.0 {
			return None;
		}
		let remaining = f64::from((1.0 - self.phase) * duration);
		let delta = f64::from(delta.as_seconds_f32());
		(delta >= remaining).then(|| MediaTime::from_seconds_f64((delta - remaining).max(0.0)))
	}

	/// Moves playback forward by `delta` and returns how many times a looping motion wrapped.
	fn advance(&mut self, delta: MediaTime) -> usize {
		self.previous_phase = self.phase;
		let duration = self.duration();
		if duration <= 0.0 {
			self.phase = 0.0;
			return 0;
		}

		let advanced = self.phase + delta.as_seconds_f32() / duration;
		match self.playback {
			AnimationPlayback::Loop => {
				self.phase = advanced.rem_euclid(1.0);
				advanced.floor().max(0.0) as usize
			}
			AnimationPlayback::Once => {
				self.phase = advanced.min(1.0);
				0
			}
		}
	}
}

/// Reports a frame point on a clip that does not record its frame rate, which the point then never reaches.
#[cold]
#[inline(never)]
fn warn_no_frame_rate(sample: &ClipBinding) -> Option<f32> {
	log::warn!(
		"Animation route point counts frames of a clip without a frame rate. The most likely cause is '{}' coming from a format that stores seconds only, like glTF; use `at_time` instead.",
		sample.resource_id
	);
	None
}

/// Returns the transform that moves to `current` as `earlier` moves to `later`, so a velocity measured from it to
/// `current` is the one measured between the other two.
fn shifted_sample(earlier: LocalTransform, later: LocalTransform, current: &LocalTransform) -> LocalTransform {
	LocalTransform {
		translation: current.translation - (later.translation - earlier.translation),
		rotation: earlier.rotation.compose(later.rotation.inverse()).compose(current.rotation),
		scale: current.scale - (later.scale - earlier.scale),
	}
}

/// Reports a route point past the end of its clip, which the point then clamps to.
#[cold]
#[inline(never)]
fn warn_past_clip_end(sample: &ClipBinding, seconds: f32) {
	log::warn!(
		"Animation route point lies past the end of its clip. The most likely cause is a frame counted beyond '{}', which lasts {:.3} s, not {seconds:.3} s.",
		sample.resource_id,
		sample.duration
	);
}

/// Resolves a state's resident clips against the canonical skeleton `target`, for [`GraphBinding::state`].
fn bind_clips(motion: &StateMotion, pool: &AnimationPool, target: &Skeleton) -> Box<[ClipBinding]> {
	motion
		.clips()
		.iter()
		.map(|clip| {
			let resident = pool
				.resident(clip.resource_id())
				.expect("a started motion's clips must be resident");
			ClipBinding {
				resource_id: clip.resource_id.clone(),
				targets: TrackTargets::by_name(resident.packed, resident.skeleton, target),
				duration: resident.packed.duration(),
				frame_rate: resident.packed.frame_rate(),
			}
		})
		.collect()
}

/// The `Evaluation` struct lends a player what one evaluation needs besides its own state, so players on several
/// lanes can share one pool: the pool to read resident clips from, the list of clips the player needs but the pool
/// doesn't hold yet, and the evaluating thread's scratch poses.
///
/// The pool requests the listed clips once evaluation ends, with [`AnimationPool::request`].
struct Evaluation<'pool, 'graph, 'local> {
	pool: &'pool AnimationPool,
	missing: &'local mut Vec<&'graph str>,
	scratch: &'local mut PoseScratch,
}

/// The `PoseScratch` struct holds the poses one evaluation fills and reads within a frame and no player keeps
/// between frames: a blend's per-clip samples and a hand-over's spans.
///
/// Each thread evaluates one player at a time, so it lends its own scratch to each; lanes are persistent threads, so
/// the buffers last across steps. They only grow, and each use takes as many transforms as its skeleton has nodes, so a
/// thread that plays skeletons of several sizes refills nothing.
#[derive(Default)]
struct PoseScratch {
	/// One pose per clip a blend samples this frame, mixed into the active or destination pose.
	blend_poses: [Vec<LocalTransform>; BlendWeights::MAX],
	loop_start: Vec<LocalTransform>,
	loop_end: Vec<LocalTransform>,
}

impl PoseScratch {
	/// Grows every pose to hold a skeleton of `node_count` nodes.
	fn fit(&mut self, node_count: usize) {
		for pose in self.blend_poses.iter_mut().chain([&mut self.loop_start, &mut self.loop_end]) {
			if pose.len() < node_count {
				pose.resize(node_count, LocalTransform::identity());
			}
		}
	}
}

thread_local! {
	/// The scratch poses of the players this thread evaluates.
	static POSE_SCRATCH: std::cell::RefCell<PoseScratch> = std::cell::RefCell::default();
}

impl<'graph> Evaluation<'_, 'graph, '_> {
	/// Returns whether a clip is resident, listing it for the pool to request when it isn't.
	fn resident(&mut self, resource_id: &'graph str) -> bool {
		let resident = self.pool.is_resident(resource_id);
		if !resident {
			self.missing.push(resource_id);
		}
		resident
	}

	/// Returns whether every clip a state plays is resident, listing the ones that aren't.
	fn motion_ready(&mut self, motion: &'graph StateMotion) -> bool {
		// Every clip is checked, so one call lists every clip that is missing.
		let missing = motion
			.clips()
			.iter()
			.filter(|clip| !self.resident(clip.resource_id()))
			.count();
		missing == 0
	}

	/// Lists the clips of the states directly reachable from `state`, through its own routes and its space's, that
	/// aren't resident, without traversing the graph further.
	fn prefetch_neighbors(&mut self, graph: &'graph AnimationGraph, id: AnimationStateId) {
		let state = graph.state(id);
		let neighbors = state
			.transitions
			.iter()
			.chain(graph.space_routes(id))
			.map(|transition| transition.target)
			.chain(state.completion);
		for neighbor in neighbors {
			self.motion_ready(&graph.state(neighbor).motion);
		}
	}
}

struct ActiveTransition<'graph> {
	source: RuntimeMotion<'graph>,
	destination: RuntimeMotion<'graph>,
	duration: MediaTime,
	elapsed: MediaTime,
	begun: bool,
	/// Why the blend started, which the player's data stream reports.
	origin: RouteOrigin,
}

/// The `PendingPlayerTransition` struct holds a selected route while its target loads or its source plays on to a
/// wait point.
struct PendingPlayerTransition<'graph> {
	/// The state the client requested, which [`AnimationGraphPlayer::waiting_for`] reports.
	requested: AnimationStateId,
	target: AnimationStateId,
	/// How long to blend from the source; `None` starts the target with nothing to blend from.
	duration: Option<MediaTime>,
	wait: &'graph [ClipPoint],
	entry: &'graph [ClipPoint],
	/// Where the route came from, which the player's data stream reports.
	origin: RouteOrigin,
}

/// The `RootMotionTranslation` struct selects node-local translation axes for root motion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootMotionTranslation(u8);

impl RootMotionTranslation {
	/// Keeps all translation in the visual pose.
	pub const NONE: Self = Self(0);
	/// Extracts translation along the node's local X axis.
	pub const X: Self = Self(1 << 0);
	/// Extracts translation along the node's local Y axis.
	pub const Y: Self = Self(1 << 1);
	/// Extracts translation along the node's local Z axis.
	pub const Z: Self = Self(1 << 2);
	/// Extracts translation along every node-local axis.
	pub const XYZ: Self = Self(Self::X.0 | Self::Y.0 | Self::Z.0);

	/// Combines translation axes for clips that move along more than one local axis.
	pub const fn union(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}

	/// Takes `selected` along the axes this selection contains and `unselected` along the others.
	fn select(self, selected: Vector<ParentSpace>, unselected: Vector<ParentSpace>) -> Vector<ParentSpace> {
		let axis = |axis: Self, selected: f32, unselected: f32| if self.0 & axis.0 != 0 { selected } else { unselected };
		Vector::new(
			axis(Self::X, selected.x(), unselected.x()),
			axis(Self::Y, selected.y(), unselected.y()),
			axis(Self::Z, selected.z(), unselected.z()),
		)
	}
}

/// The `RootMotionRotation` enum selects whether node-local rotation drives the owning object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootMotionRotation {
	/// Keeps rotation in the visual pose.
	None,
	/// Extracts the node's full rotation.
	Full,
}

/// The `RootMotionSettings` struct defines which motion a canonical-skeleton node contributes to its owning object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootMotionSettings<'a> {
	/// Names the unique node that contains the authored motion.
	pub node_name: &'a str,
	/// Selects node-local translation axes to extract.
	pub translation: RootMotionTranslation,
	/// Selects whether to extract node-local rotation.
	pub rotation: RootMotionRotation,
}

impl<'a> RootMotionSettings<'a> {
	/// Selects all translation and rotation from a dedicated authored root node.
	pub const fn full(node_name: &'a str) -> Self {
		Self {
			node_name,
			translation: RootMotionTranslation::XYZ,
			rotation: RootMotionRotation::Full,
		}
	}
}

#[derive(Clone, Copy)]
struct RootMotionTarget {
	node: usize,
	reference: LocalTransform,
	translation: RootMotionTranslation,
	rotation: RootMotionRotation,
}

/// The `AnimationGraphPose` struct borrows the latest player pose and frame root motion.
///
/// Apply [`Self::root_motion`] to the owning object, then submit [`Self::global_pose`] to rendering during the
/// current tick.
pub struct AnimationGraphPose<'a> {
	/// The canonical skeleton that defines the pose's node and matrix order.
	pub skeleton: &'a Skeleton,
	/// Blendable local transforms with extracted root motion removed when configured.
	pub local_pose: &'a [LocalTransform],
	/// Renderer-facing global skeleton transforms, one affine matrix per node, as `UpdatePose` carries them.
	pub global_pose: &'a [AffineMatrix],
	/// This frame's root translation and rotation delta.
	pub root_motion: RootMotionDelta,
	/// The state playing after this advance, as [`AnimationGraphPlayer::state`] reports it. Use it to react to a route
	/// taken this frame, like taking over a turn clip's rotation, while still holding the pose.
	pub state: Option<AnimationStateId>,
}

/// The `AnimationGraphPlayer` struct retains local playback while the pool resolves all graph data asynchronously.
///
/// Create players with [`AnimationPool::create_player`]. Call [`AnimationPool::update`] once per tick, then either
/// call [`Self::advance`] with the client-selected state, or set each player's state with [`Self::request`] and
/// advance them all together with [`AnimationPool::advance_players`]. The initial clip supplies the canonical
/// skeleton used by every graph clip.
pub struct AnimationGraphPlayer<'graph> {
	graph: &'graph AnimationGraph,
	/// Retained until the initial clip supplies the canonical skeleton that resolves the node name.
	root_motion: Option<RootMotionSettings<'graph>>,
	runtime: PlayerRuntime<'graph>,
	/// The latest request, which holds until another replaces it.
	requested: AnimationRequest,
	/// The clips the latest evaluation needed but the pool didn't hold, for the pool to request.
	missing: Vec<&'graph str>,
	/// Where the player reports its state after each advance; see [`Self::set_stream`].
	stream: Option<crate::inspector::DataStream>,
}

/// The `PlayerRuntime` enum follows a player from waiting for its initial clip to playing, or to never playing.
enum PlayerRuntime<'graph> {
	/// Waiting for the initial clip, whose skeleton becomes the canonical skeleton.
	Loading,
	Ready(ReadyAnimationGraphPlayer<'graph>),
	/// The player's settings don't resolve against the canonical skeleton.
	Rejected {
		error: AnimationGraphPlayerError,
		/// Whether an advance returned the error yet; it returns it once.
		reported: bool,
	},
}

/// Most players a lane takes on: fewer players than this per lane don't pay for waking the lanes.
const PLAYERS_PER_LANE: usize = 16;

/// Returns how many lanes advancing `players` players runs on, never more than the machine's fastest cores have
/// hardware threads, because the step waits for its slowest lane.
fn player_lanes(players: usize) -> usize {
	static FAST_LANES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
	let fast_lanes = *FAST_LANES.get_or_init(crate::core::machine::performance_parallelism);
	(players / PLAYERS_PER_LANE).clamp(1, fast_lanes)
}

impl AnimationPool {
	/// Creates a local player and queues the initial clip that supplies its canonical skeleton.
	///
	/// Continue calling [`Self::update`] once per tick, then pass this pool to
	/// [`AnimationGraphPlayer::advance`] so requested clips can load through the
	/// same asynchronous worker.
	pub fn create_player<'graph>(
		&mut self,
		graph: &'graph AnimationGraph,
		root_motion: Option<RootMotionSettings<'graph>>,
	) -> AnimationGraphPlayer<'graph> {
		for clip in graph.state(graph.initial_state()).motion.clips() {
			self.request(clip.resource_id());
		}
		AnimationGraphPlayer {
			graph,
			root_motion,
			runtime: PlayerRuntime::Loading,
			requested: graph.initial_state().into(),
			missing: Vec::new(),
			stream: None,
		}
	}

	/// Advances every player by `delta` toward its latest [request](AnimationGraphPlayer::request), then requests
	/// the clips they found missing.
	///
	/// Players only read the pool while they evaluate, so large crowds spread over `alley`'s lanes, each lane taking
	/// a balanced share of the players; small ones stay on the calling thread. Read each player's result with
	/// [`AnimationGraphPlayer::pose`] next. Returns the error of the first player rejected in this advance, such as a
	/// root motion node its canonical skeleton lacks, after advancing the others.
	pub fn advance_players(
		&mut self,
		players: &mut [AnimationGraphPlayer<'_>],
		delta: MediaTime,
		alley: &mut crate::core::alley::Alley,
	) -> Result<(), AnimationGraphPlayerError> {
		if delta < MediaTime::ZERO {
			return Err(AnimationGraphPlayerError::NegativeDelta);
		}
		let pool = &*self;
		let unsettled = AtomicBool::new(false);
		alley.for_each_mut(players, player_lanes(players.len()), |_, players| {
			let lane_unsettled = POSE_SCRATCH.with_borrow_mut(|scratch| {
				players
					.iter_mut()
					.fold(false, |unsettled, player| player.evaluate(delta, pool, scratch) | unsettled)
			});
			if lane_unsettled {
				unsettled.store(true, Ordering::Relaxed);
			}
		});
		// Loads, admissions, and reports change the pool, so they wait until no player reads it. Most steps have none,
		// so the players aren't visited again.
		if !unsettled.into_inner() {
			return Ok(());
		}
		let mut first_error = None;
		for player in players {
			if let Some(error) = self.settle(player) {
				first_error.get_or_insert(error);
			}
		}
		first_error.map_or(Ok(()), Err)
	}

	/// Requests the clips `player`'s latest evaluation listed as missing, and returns its rejection the first time.
	fn settle(&mut self, player: &mut AnimationGraphPlayer<'_>) -> Option<AnimationGraphPlayerError> {
		for resource_id in player.missing.drain(..) {
			self.request(resource_id);
		}
		match &mut player.runtime {
			PlayerRuntime::Rejected { error, reported } if !*reported => {
				*reported = true;
				Some(error.clone())
			}
			_ => None,
		}
	}
}

impl<'graph> AnimationGraphPlayer<'graph> {
	/// Returns the currently playing destination state after the initial clip has loaded.
	pub fn state(&self) -> Option<AnimationStateId> {
		self.ready().and_then(ReadyAnimationGraphPlayer::state)
	}

	/// Returns the requested state whose route waits for the playing state to reach one of its wait points, like
	/// [`AnimationGraphTransitionBuilder::at_frame`].
	///
	/// Use it to hold game logic tied to the route, like a jump's lift-off, until the animation takes the route.
	/// Requesting another state cancels the wait.
	pub fn waiting_for(&self) -> Option<AnimationStateId> {
		self.ready().and_then(ReadyAnimationGraphPlayer::waiting_for)
	}

	/// Returns how long the playing state plays on before its route to `target` starts: zero for a route that does not
	/// wait, and the time to the nearest of its wait points for one that does, like
	/// [`AnimationGraphTransitionBuilder::at_frames`].
	///
	/// Use it to choose, among states that serve one purpose, the one whose route starts soonest, like a stop off
	/// either foot. Returns `None` when the playing state has no route to `target`, a one-shot has played past every
	/// wait point, the player is blending, or the initial clip has not loaded.
	pub fn time_until_route(&self, target: AnimationStateId) -> Option<MediaTime> {
		self.ready()?.time_until_route(target)
	}

	/// Replaces the state this player plays toward, which holds until the next request.
	///
	/// `requested` is a state ID, or a blend state with its position from [`LinearStateId::at`] or
	/// [`PlanarStateId::at`]. An [inertialized](AnimationRequest::inertialized) request blends a playing blend state
	/// to a position that changes its weights; a request held after that blend changes nothing more.
	pub fn request(&mut self, requested: impl Into<AnimationRequest>) -> Result<(), AnimationGraphPlayerError> {
		let requested = requested.into();
		if !self.graph.contains(requested.state) {
			return Err(AnimationGraphPlayerError::StateFromDifferentGraph);
		}
		self.requested = requested;
		Ok(())
	}

	/// Returns the latest advance's pose, or [`None`] while the initial clip loads asynchronously.
	///
	/// Keep the renderable's existing pose until a pose arrives.
	pub fn pose(&self) -> Option<AnimationGraphPose<'_>> {
		self.ready().map(ReadyAnimationGraphPlayer::pose)
	}

	/// Requests `requested`, advances by `delta`, and returns the resulting pose, like [`Self::request`],
	/// [`AnimationPool::advance_players`] for this player alone, and [`Self::pose`].
	///
	/// Returns [`None`] while the initial clip loads asynchronously. Keep the renderable's existing pose until a pose
	/// arrives. Returns the player's rejection, such as a root motion node its canonical skeleton lacks, in the advance
	/// that finds it.
	pub fn advance(
		&mut self,
		delta: MediaTime,
		requested: impl Into<AnimationRequest>,
		pool: &mut AnimationPool,
	) -> Result<Option<AnimationGraphPose<'_>>, AnimationGraphPlayerError> {
		if delta < MediaTime::ZERO {
			return Err(AnimationGraphPlayerError::NegativeDelta);
		}
		self.request(requested)?;
		POSE_SCRATCH.with_borrow_mut(|scratch| self.evaluate(delta, pool, scratch));
		if let Some(error) = pool.settle(self) {
			return Err(error);
		}
		Ok(self.pose())
	}

	/// Returns the player once it plays.
	fn ready(&self) -> Option<&ReadyAnimationGraphPlayer<'graph>> {
		match &self.runtime {
			PlayerRuntime::Ready(ready) => Some(ready),
			PlayerRuntime::Loading | PlayerRuntime::Rejected { .. } => None,
		}
	}

	/// Starts playing once the initial clip is resident, then plays toward the latest request.
	///
	/// It only reads `pool`, listing the clips it needs but the pool lacks in [`Self::missing`], then writes a record
	/// to its stream when it has one. Returns whether the pool has work to settle for the player: clips to request, or
	/// a rejection to report.
	fn evaluate(&mut self, delta: MediaTime, pool: &AnimationPool, scratch: &mut PoseScratch) -> bool {
		let unsettled = self.advance_runtime(delta, pool, scratch);
		if self.stream.is_some() {
			self.write_stream();
		}
		unsettled
	}

	/// Starts or advances the player's runtime, and returns whether the pool has work to settle for it.
	fn advance_runtime(&mut self, delta: MediaTime, pool: &AnimationPool, scratch: &mut PoseScratch) -> bool {
		if matches!(self.runtime, PlayerRuntime::Loading) {
			self.start(pool);
		}
		match &mut self.runtime {
			PlayerRuntime::Ready(runtime) => {
				scratch.fit(runtime.pose.target.nodes.len());
				let mut evaluation = Evaluation {
					pool,
					missing: &mut self.missing,
					scratch,
				};
				runtime.advance(delta, self.requested, &mut evaluation);
			}
			PlayerRuntime::Rejected { reported, .. } => return !*reported,
			PlayerRuntime::Loading => {}
		}
		!self.missing.is_empty()
	}

	/// Leaves [`PlayerRuntime::Loading`] once the initial clip is resident: its skeleton becomes the canonical
	/// skeleton, which sizes the player's poses and resolves its root motion node.
	fn start(&mut self, pool: &AnimationPool) {
		let graph = self.graph;
		// The initial state's first clip supplies the canonical skeleton.
		let initial = graph.state(graph.initial_state()).motion.clips()[0].resource_id();
		if !pool.is_resident(initial) {
			self.missing.push(initial);
			return;
		}
		// Every player of the graph shares the canonical skeleton the first one took from the initial clip.
		let skeleton = Arc::clone(graph.binding.skeleton(|| {
			Arc::clone(
				pool.resident(initial)
					.expect("a resident initial clip stays resident while the pool is read")
					.skeleton,
			)
		}));
		self.runtime = match PlayerPose::new(skeleton, self.root_motion) {
			// The canonical skeleton sizes every retained evaluation buffer, and the player starts in the initial state.
			Ok(pose) => PlayerRuntime::Ready(ReadyAnimationGraphPlayer {
				graph,
				playback: Playback::AwaitingInitial,
				pending: Some(PendingPlayerTransition {
					requested: graph.initial_state(),
					target: graph.initial_state(),
					duration: None,
					wait: &[],
					entry: &[],
					origin: RouteOrigin::Initial,
				}),
				positions: BlendPositions::new(&graph.spaces),
				last_delta: MediaTime::ZERO,
				root_motion: RootMotionDelta::IDENTITY,
				pose,
			}),
			Err(error) => PlayerRuntime::Rejected { error, reported: false },
		};
	}
}

/// The `ReadyAnimationGraphPlayer` struct retains playback after the initial clip supplies its skeleton.
struct ReadyAnimationGraphPlayer<'graph> {
	graph: &'graph AnimationGraph,
	playback: Playback<'graph>,
	pending: Option<PendingPlayerTransition<'graph>>,
	/// Each space's position, which its looping blends follow and its blends start at.
	positions: BlendPositions,
	/// The latest frame's duration, which a redirected blend measures the on-screen pose's velocity over.
	last_delta: MediaTime,
	/// The root motion of the latest advance.
	root_motion: RootMotionDelta,
	pose: PlayerPose,
}

/// Playback owns either one clip or both sides of a transition, never both representations.
enum Playback<'graph> {
	AwaitingInitial,
	Playing(RuntimeMotion<'graph>),
	Transitioning(ActiveTransition<'graph>),
}

/// The `PlayerPose` struct retains sampling buffers independently of playback transitions.
struct PlayerPose {
	target: Arc<Skeleton>,
	root_motion: Option<RootMotionTarget>,
	active_previous: Vec<LocalTransform>,
	active_current: Vec<LocalTransform>,
	destination_previous: Vec<LocalTransform>,
	destination_current: Vec<LocalTransform>,
	/// The nodes root motion measures and their transforms at the moments it measures between.
	root_chain: RootChain,
	local_pose: Vec<LocalTransform>,
	/// The pose shown the frame before `local_pose`, which a redirected blend takes its starting velocity from.
	local_previous: Vec<LocalTransform>,
	global_pose: Vec<AffineMatrix>,
	inertializer: PoseInertializer,
}

impl<'graph> ReadyAnimationGraphPlayer<'graph> {
	/// Returns the requested state whose route waits for the playing motion to reach a wait point.
	fn waiting_for(&self) -> Option<AnimationStateId> {
		self.pending
			.as_ref()
			.filter(|pending| !pending.wait.is_empty())
			.map(|pending| pending.requested)
	}

	/// Returns how long the playing motion plays on before its route to `target` starts.
	fn time_until_route(&self, target: AnimationStateId) -> Option<MediaTime> {
		let Playback::Playing(active) = &self.playback else {
			return None;
		};
		let route = self.graph.select_transition(active.state, target, active.is_finished())?;
		if route.wait.is_empty() {
			Some(MediaTime::ZERO)
		} else {
			active.time_until(route.wait)
		}
	}

	/// Returns the currently playing destination state.
	fn state(&self) -> Option<AnimationStateId> {
		match &self.playback {
			Playback::AwaitingInitial => None,
			Playback::Playing(active) => Some(active.state),
			Playback::Transitioning(transition) => Some(transition.destination.state),
		}
	}

	/// Reconciles playback toward `requested`, advances ready clips, and borrows the resulting pose.
	///
	/// Call [`AnimationPool::update`] once before advancing any players for the
	/// current tick. While a selected target loads, the player retains the source
	/// clip and the next request can cancel or retarget that pending route. Next,
	/// apply [`AnimationGraphPose::root_motion`] to the owning object and submit
	/// [`AnimationGraphPose::global_pose`] to the rendering system.
	fn advance(&mut self, delta: MediaTime, requested: AnimationRequest, evaluation: &mut Evaluation<'_, 'graph, '_>) {
		self.positions
			.remember(self.graph.state(requested.state).space, requested.value);
		self.refresh_pending_transition(requested.state);
		self.start_pending(evaluation);

		if let Playback::Playing(active) = &self.playback
			&& self.pending.is_none()
		{
			self.pending = self.selected_transition(active, requested.state);
			self.start_pending(evaluation);
		}
		self.redirect_blend(requested.state, evaluation);
		self.apply_blend_position(requested, evaluation);

		self.pose.begin_frame();
		let graph = self.graph;
		// The part of the frame left once the playing motion reaches a wait point or a one-shot's end.
		let mut leftover = None;
		let root_motion = match &mut self.playback {
			Playback::AwaitingInitial => {
				self.pose.write_rest_pose();
				RootMotionDelta::IDENTITY
			}
			Playback::Playing(active) => {
				if evaluation.motion_ready(&graph.state(active.state).motion) {
					// A route waiting for a moment of the clip starts there, so the motion plays only up to it.
					let waited = self
						.pending
						.as_ref()
						.and_then(|pending| active.time_until(pending.wait))
						.filter(|time| *time <= delta);
					let played = match waited {
						Some(time) => {
							leftover = Some(delta - time);
							time
						}
						None => {
							leftover = active.leftover_after(delta);
							delta
						}
					};
					self.pose.advance_active(active, played, evaluation)
				} else {
					self.pose.hold_frame();
					RootMotionDelta::IDENTITY
				}
			}
			Playback::Transitioning(transition) => {
				// Both sides are checked, so one evaluation lists every clip either lacks.
				let source_ready = evaluation.motion_ready(&graph.state(transition.source.state).motion);
				if evaluation.motion_ready(&graph.state(transition.destination.state).motion) && source_ready {
					let root_motion = self.pose.advance_transition(transition, delta, evaluation);
					if transition.elapsed == transition.duration {
						// Move the destination into sole ownership only when the transition completes.
						let Playback::Transitioning(transition) =
							std::mem::replace(&mut self.playback, Playback::AwaitingInitial)
						else {
							unreachable!("Only a transition can complete playback blending");
						};
						self.playback = Playback::Playing(transition.destination);
					}
					root_motion
				} else {
					self.pose.hold_frame();
					RootMotionDelta::IDENTITY
				}
			}
		};

		// A motion that reaches a wait point or a one-shot's end hands the rest of the frame to the next state, so
		// the route starts exactly there and a one-shot's last pose is never shown twice.
		let root_motion = match leftover {
			Some(leftover) => root_motion.then(self.hand_over(delta, leftover, requested, evaluation)),
			None => root_motion,
		};

		self.last_delta = delta;
		self.root_motion = root_motion;
	}

	/// Borrows the latest advance's pose and root motion.
	fn pose(&self) -> AnimationGraphPose<'_> {
		AnimationGraphPose {
			state: self.state(),
			skeleton: &self.pose.target,
			local_pose: &self.pose.local_pose,
			global_pose: &self.pose.global_pose,
			root_motion: self.root_motion,
		}
	}

	/// Redirects a blend in progress toward the requested state along an `.anytime` route.
	///
	/// The new blend starts from the pose on screen, with its velocity, so nothing pops. The interrupted blend's
	/// destination keeps playing as the new source, which carries root motion while the new destination fades in.
	// Rare: kept out of line so it does not weigh on the every-frame path.
	#[cold]
	#[inline(never)]
	fn redirect_blend(&mut self, requested: AnimationStateId, evaluation: &mut Evaluation<'_, 'graph, '_>) {
		let Playback::Transitioning(transition) = &self.playback else {
			return;
		};
		// The on-screen velocity needs one blended frame and a non-zero frame time.
		if !transition.begun || self.last_delta == MediaTime::ZERO {
			return;
		}
		let Some(route) = self.graph.select_interruption(transition.destination.state, requested) else {
			return;
		};
		// A route that waits for a moment of its source starts once the blend settles and the source plays on its own.
		if !route.wait.is_empty() || !evaluation.motion_ready(&self.graph.state(route.target).motion) {
			return;
		}
		let (target, duration, origin) = (route.target, route.duration, route.origin);
		let destination = self.start_motion(target, route.entry, evaluation.pool);
		let Playback::Transitioning(transition) = std::mem::replace(&mut self.playback, Playback::AwaitingInitial) else {
			unreachable!("only a blend in progress can be redirected");
		};
		// The interrupted blend's destination becomes the source, so its latest sample becomes the source's, which
		// its root motion measures from; otherwise the next frame would measure from the old source's pose.
		self.pose.active_current.copy_from_slice(&self.pose.destination_current);
		self.blend_from_display(transition.destination, destination, duration, origin, evaluation);
		self.pending = None;
		evaluation.prefetch_neighbors(self.graph, target);
	}

	/// Moves every looping blend of the requested state's space, wherever it plays, to the requested blend position.
	///
	/// A request that asks to be [inertialized](AnimationRequest::inertialized) blends a playing blend state to its
	/// new position from the pose on screen instead. Blends that play once keep the weights they started with, so a
	/// stop keeps the direction it set off in.
	fn apply_blend_position(&mut self, requested: AnimationRequest, evaluation: &mut Evaluation<'_, 'graph, '_>) {
		// Clip states and blends requested without a position keep their weights.
		if requested.value == BlendValue::Unset {
			return;
		}
		if requested.inertialization > MediaTime::ZERO
			&& self.blend_to_position(requested, requested.inertialization, evaluation)
		{
			return;
		}
		let graph = self.graph;
		let Some(space) = graph.state(requested.state).space else {
			return;
		};
		let follow = |runtime: &mut RuntimeMotion| {
			let state = graph.state(runtime.state);
			if state.space == Some(space) && runtime.playback == AnimationPlayback::Loop {
				runtime.weights = state.motion.weights(requested.value, runtime.weights);
			}
		};
		match &mut self.playback {
			Playback::AwaitingInitial => {}
			Playback::Playing(active) => follow(active),
			Playback::Transitioning(transition) => {
				follow(&mut transition.source);
				follow(&mut transition.destination);
			}
		}
	}

	/// Turns the playing blend state into a blend from itself at its current weights to itself at the requested
	/// position, starting from the pose on screen so the pose does not pop. Returns whether it started one.
	// Rare: kept out of line so it does not weigh on the every-frame path.
	#[cold]
	#[inline(never)]
	fn blend_to_position(
		&mut self,
		requested: AnimationRequest,
		duration: MediaTime,
		evaluation: &mut Evaluation<'_, 'graph, '_>,
	) -> bool {
		let Playback::Playing(active) = &self.playback else {
			return false;
		};
		// The on-screen velocity needs a non-zero frame time, and a blend played once keeps its weights.
		if active.state != requested.state
			|| active.playback != AnimationPlayback::Loop
			|| self.last_delta == MediaTime::ZERO
		{
			return false;
		}
		let mut destination = *active;
		destination.weights = self
			.graph
			.state(requested.state)
			.motion
			.weights(requested.value, destination.weights);
		// A held request asks for the weights already playing, which need no blend.
		if destination.weights == active.weights {
			return false;
		}
		let Playback::Playing(source) = std::mem::replace(&mut self.playback, Playback::AwaitingInitial) else {
			unreachable!("the playing motion was checked above");
		};
		// The playing motion's latest sample is already the active pose, which the source's root motion measures from.
		self.blend_from_display(source, destination, duration, RouteOrigin::Position, evaluation);
		true
	}

	/// Blends from `source` to `destination` over `duration`, starting from the pose on screen and its velocity.
	///
	/// The active pose must hold `source`'s latest sample.
	// Rare: kept out of line so it does not weigh on the every-frame path.
	#[cold]
	#[inline(never)]
	fn blend_from_display(
		&mut self,
		source: RuntimeMotion<'graph>,
		destination: RuntimeMotion<'graph>,
		duration: MediaTime,
		origin: RouteOrigin,
		evaluation: &mut Evaluation<'_, 'graph, '_>,
	) {
		self.pose.sample(&destination, destination.phase, evaluation, Sampled::Destination);
		self.pose.destination_previous.copy_from_slice(&self.pose.destination_current);
		self.pose.begin_from_display(self.last_delta, duration);
		self.playback = Playback::Transitioning(ActiveTransition {
			source,
			destination,
			duration,
			elapsed: MediaTime::ZERO,
			begun: true,
			origin,
		});
	}

	/// Starts the motion of `target`, a blend at its space's position, at the `entry` point of its heaviest named clip
	/// or at its beginning.
	fn start_motion(
		&self,
		target: AnimationStateId,
		entry: &[ClipPoint],
		pool: &AnimationPool,
	) -> RuntimeMotion<'graph> {
		let graph = self.graph;
		let state = graph.state(target);
		let samples = graph
			.binding
			.state(target.index, || bind_clips(&state.motion, pool, &self.pose.target));
		let mut runtime = RuntimeMotion::new(target, &state.motion, self.positions.recall(state.space), samples);
		let entry = runtime.points_of_heaviest(entry).next().copied();
		if let Some(phase) = entry.and_then(|entry| runtime.phase_of(entry)) {
			runtime.phase = phase;
			runtime.previous_phase = phase;
		}
		runtime
	}

	/// Takes the route the playing motion reached this frame, a waiting route at its wait point or a finished
	/// one-shot's completion, and plays `leftover` of the next state within the same frame.
	///
	/// A zero-length route cuts to the new clip; otherwise the frame begins inertializing from the motion's last
	/// pose. A clip still loading, or a zero-time frame that gives no velocities, starts on the next advance instead.
	fn hand_over(
		&mut self,
		delta: MediaTime,
		leftover: MediaTime,
		requested: AnimationRequest,
		evaluation: &mut Evaluation<'_, 'graph, '_>,
	) -> RootMotionDelta {
		let Playback::Playing(active) = &self.playback else {
			return RootMotionDelta::IDENTITY;
		};
		let route = match &self.pending {
			Some(pending) if !pending.wait.is_empty() => {
				pending.duration.map(|duration| (pending.target, duration, pending.entry, pending.origin))
			}
			_ => self
				.graph
				.select_transition(active.state, requested.state, true)
				.map(|route| (route.target, route.duration, route.entry, route.origin)),
		};
		let Some((target, blend, entry, origin)) = route else {
			return RootMotionDelta::IDENTITY;
		};
		if blend != MediaTime::ZERO && delta == MediaTime::ZERO {
			return RootMotionDelta::IDENTITY;
		}
		if !evaluation.motion_ready(&self.graph.state(target).motion) {
			return RootMotionDelta::IDENTITY;
		}
		let mut destination = self.start_motion(target, entry, evaluation.pool);

		let root_motion = if blend == MediaTime::ZERO {
			// Start from the motion's first frame so root motion covers only the leftover time.
			self.pose.sample(&destination, destination.phase, evaluation, Sampled::Active);
			self.pose.advance_active(&mut destination, leftover, evaluation)
		} else {
			self.pose.begin_handover_blend(&mut destination, delta, leftover, blend, evaluation)
		};

		self.playback = match std::mem::replace(&mut self.playback, Playback::AwaitingInitial) {
			Playback::Playing(source) if blend != MediaTime::ZERO => Playback::Transitioning(ActiveTransition {
				source,
				destination,
				duration: blend,
				elapsed: leftover.min(blend),
				begun: true,
				origin,
			}),
			_ => Playback::Playing(destination),
		};
		self.pending = None;
		evaluation.prefetch_neighbors(self.graph, target);
		root_motion
	}

	/// Starts a selected state once all of its clips are resident, preserving playback while they load.
	fn start_pending(&mut self, evaluation: &mut Evaluation<'_, 'graph, '_>) {
		let Some(pending) = self.pending.as_ref() else {
			return;
		};
		// A waiting route still loads its clips, but starts when the source reaches its wait point.
		if !evaluation.motion_ready(&self.graph.state(pending.target).motion) || !pending.wait.is_empty() {
			return;
		}
		let (target, origin) = (pending.target, pending.origin);
		let destination = self.start_motion(target, pending.entry, evaluation.pool);

		// A route of zero length cuts to its target, so only the target plays this frame.
		self.playback = if let Some(duration) = pending.duration.filter(|duration| *duration > MediaTime::ZERO) {
			let Playback::Playing(source) = std::mem::replace(&mut self.playback, Playback::AwaitingInitial) else {
				unreachable!("Only a playing clip can start a graph transition");
			};
			self.pose.sample(&destination, destination.phase, evaluation, Sampled::Destination);
			self.pose.destination_previous.copy_from_slice(&self.pose.destination_current);
			Playback::Transitioning(ActiveTransition {
				source,
				destination,
				duration,
				elapsed: MediaTime::ZERO,
				begun: false,
				origin,
			})
		} else {
			self.pose.sample(&destination, destination.phase, evaluation, Sampled::Active);
			self.pose.active_previous.copy_from_slice(&self.pose.active_current);
			Playback::Playing(destination)
		};
		self.pending = None;
		evaluation.prefetch_neighbors(self.graph, target);
	}

	/// Cancels or retargets a loading state when its source's selected edge changes.
	fn refresh_pending_transition(&mut self, requested: AnimationStateId) {
		if let Playback::Playing(active) = &self.playback
			&& self.pending.as_ref().is_some_and(|pending| pending.duration.is_some())
		{
			self.pending = self.selected_transition(active, requested);
		}
	}

	/// Resolves the active clip toward the persistent state requested by client code.
	fn selected_transition(
		&self,
		active: &RuntimeMotion,
		requested: AnimationStateId,
	) -> Option<PendingPlayerTransition<'graph>> {
		self.graph
			.select_transition(active.state, requested, active.is_finished())
			.map(|route| PendingPlayerTransition {
				requested,
				target: route.target,
				duration: Some(route.duration),
				wait: route.wait,
				entry: route.entry,
				origin: route.origin,
			})
	}
}

/// Names which retained pose buffer a sample writes.
#[derive(Clone, Copy)]
enum Sampled {
	Active,
	Destination,
}

impl PlayerPose {
	/// Allocates sampling buffers once from the canonical rest pose.
	fn new(target: Arc<Skeleton>, root_motion: Option<RootMotionSettings<'_>>) -> Result<Self, AnimationGraphPlayerError> {
		let root_motion = resolve_root_motion_target(&target, root_motion)?;
		let root_chain = RootChain::new(&target, root_motion);
		let node_count = target.nodes.len();
		let rest_pose: Vec<_> = target.nodes.iter().map(|node| node.rest_local).collect();
		let mut global_pose = Vec::with_capacity(node_count);
		write_global_pose(&target, &rest_pose, &mut global_pose)
			.expect("Canonical rest pose must match its skeleton node count");

		Ok(Self {
			target,
			root_motion,
			active_previous: rest_pose.clone(),
			active_current: rest_pose.clone(),
			destination_previous: rest_pose.clone(),
			destination_current: rest_pose.clone(),
			root_chain,
			local_previous: rest_pose.clone(),
			local_pose: rest_pose,
			global_pose,
			inertializer: PoseInertializer::new(node_count),
		})
	}

	/// Samples `motion` at `phase` into the active or destination pose.
	fn sample(&mut self, motion: &RuntimeMotion, phase: f32, evaluation: &mut Evaluation<'_, '_, '_>, into: Sampled) {
		let output = match into {
			Sampled::Active => &mut self.active_current,
			Sampled::Destination => &mut self.destination_current,
		};
		sample_motion(motion, phase, evaluation.pool, &mut evaluation.scratch.blend_poses, output);
	}

	/// Returns `motion`'s root delta since its previous phase. `poses` selects the retained poses it played between.
	fn root_delta(
		&mut self,
		motion: &RuntimeMotion,
		wrapped_loops: usize,
		pool: &AnimationPool,
		poses: Sampled,
	) -> RootMotionDelta {
		let (previous, current) = match poses {
			Sampled::Active => (&self.active_previous, &self.active_current),
			Sampled::Destination => (&self.destination_previous, &self.destination_current),
		};
		motion_root_delta(
			self.root_motion,
			&mut self.root_chain,
			motion,
			previous,
			current,
			wrapped_loops,
			pool,
		)
	}

	/// Samples the playing motion into the retained visual pose and extracts its root motion.
	fn advance_active(
		&mut self,
		active: &mut RuntimeMotion,
		delta: MediaTime,
		evaluation: &mut Evaluation<'_, '_, '_>,
	) -> RootMotionDelta {
		let wrapped_loops = active.advance(delta);
		std::mem::swap(&mut self.active_previous, &mut self.active_current);
		self.sample(active, active.phase, evaluation, Sampled::Active);
		let root_motion = self.root_delta(active, wrapped_loops, evaluation.pool, Sampled::Active);
		self.local_pose.copy_from_slice(&self.active_current);
		self.remove_root_motion_from_visual_pose();
		self.write_global_pose();
		root_motion
	}

	/// Blends both transition motions and retains the destination samples when blending completes.
	fn advance_transition(
		&mut self,
		transition: &mut ActiveTransition,
		delta: MediaTime,
		evaluation: &mut Evaluation<'_, '_, '_>,
	) -> RootMotionDelta {
		let source_wrapped_loops = transition.source.advance(delta);
		let destination_wrapped_loops = transition.destination.advance(delta);
		std::mem::swap(&mut self.active_previous, &mut self.active_current);
		std::mem::swap(&mut self.destination_previous, &mut self.destination_current);
		self.sample(&transition.source, transition.source.phase, evaluation, Sampled::Active);
		self.sample(
			&transition.destination,
			transition.destination.phase,
			evaluation,
			Sampled::Destination,
		);
		let pool = evaluation.pool;

		let source_root_motion = self.root_delta(&transition.source, source_wrapped_loops, pool, Sampled::Active);
		let destination_root_motion =
			self.root_delta(&transition.destination, destination_wrapped_loops, pool, Sampled::Destination);

		let duration = transition.duration;
		// How far this frame takes the blend, which starts from the poses this frame samples.
		let mut blended = delta;
		if duration == MediaTime::ZERO {
			self.local_pose.copy_from_slice(&self.destination_current);
		} else if delta == MediaTime::ZERO && !transition.begun {
			// Inertialization needs a non-zero sample interval to derive velocities.
			// Keep the source pose for this zero-time tick and begin next advance.
			self.local_pose.copy_from_slice(&self.active_current);
		} else {
			if !transition.begun {
				self.inertializer
					.begin(
						&self.active_previous,
						&self.active_current,
						&self.destination_previous,
						&self.destination_current,
						delta,
						duration,
					)
					.expect("player pose buffers must match the canonical skeleton");
				transition.begun = true;
				// The offsets are measured at the end of this frame, so the blend starts there: decaying them over
				// this frame too would move the pose by a frame of decay in a single step.
				blended = MediaTime::ZERO;
			}
			self.inertializer
				.apply(&self.destination_current, blended, &mut self.local_pose)
				.expect("player pose buffers must match the canonical skeleton");
		}

		transition.elapsed = (transition.elapsed + blended).min(duration);
		let transition_factor = if duration == MediaTime::ZERO {
			1.0
		} else {
			(transition.elapsed.as_seconds_f32() / duration.as_seconds_f32()).clamp(0.0, 1.0)
		};
		self.remove_root_motion_from_visual_pose();
		self.write_global_pose();

		if transition.elapsed == duration {
			self.active_previous.copy_from_slice(&self.destination_current);
			self.active_current.copy_from_slice(&self.destination_current);
		}
		// A finished one-shot holds its last pose and no longer moves, so the destination carries all root motion.
		if transition.source.is_finished() {
			destination_root_motion
		} else {
			source_root_motion.blend(destination_root_motion, transition_factor)
		}
	}

	/// Starts inertializing from the motion that just handed over, a finished one-shot or a source at its wait point,
	/// into `destination`, which then plays `leftover` of this frame.
	///
	/// The source's last two samples and two destination samples from where it starts on give both sides their velocity.
	fn begin_handover_blend(
		&mut self,
		destination: &mut RuntimeMotion,
		delta: MediaTime,
		leftover: MediaTime,
		blend: MediaTime,
		evaluation: &mut Evaluation<'_, '_, '_>,
	) -> RootMotionDelta {
		// The one-shot only moved until its end, so its last step spans the frame minus the leftover. Sampling the
		// destination over the same span keeps both velocities comparable.
		let interval = if delta > leftover { delta - leftover } else { delta };
		// The source stopped at the hand-over moment, so the offset must compare it with the destination at that same
		// moment, where it starts, not after it plays the leftover, or the destination's own travel would count as an
		// offset. Spans of a clip are measured within one cycle: looping clips need not be continuous across their wrap
		// in local space (the root travels), so a span across it would invent a huge jump. The starting pose is the one
		// the destination reaches less its travel over the leftover, which keeps it continuous with that pose even when
		// a loop wraps on the way, and the velocity span then moves onto it.
		let start = destination.phase;
		let duration = destination.duration();
		let within_cycle = |time: MediaTime| {
			if duration > 0.0 {
				let span = (time.as_seconds_f32() / duration).min(1.0);
				let later = (start + span).clamp(span, 1.0);
				(later - span, later)
			} else {
				(start, start)
			}
		};
		let wrapped_loops = destination.advance(leftover);
		self.sample(destination, destination.phase, evaluation, Sampled::Destination);
		let pool = evaluation.pool;
		let PoseScratch {
			blend_poses,
			loop_start,
			loop_end,
		} = &mut *evaluation.scratch;
		// The scratch spans hold as many transforms as this skeleton has nodes.
		let nodes = self.local_pose.len();
		let (loop_start, loop_end) = (&mut loop_start[..nodes], &mut loop_end[..nodes]);
		// The loop buffers only serve root motion, which is measured below, so they hold spans until then.
		let (earlier, later) = within_cycle(leftover);
		sample_motion(destination, earlier, pool, blend_poses, loop_start);
		sample_motion(destination, later, pool, blend_poses, loop_end);
		for ((earlier, later), current) in loop_start.iter_mut().zip(&*loop_end).zip(&self.destination_current) {
			*earlier = shifted_sample(*earlier, *later, current);
		}
		let (earlier, later) = within_cycle(interval);
		sample_motion(
			destination,
			earlier,
			pool,
			blend_poses,
			&mut self.destination_previous,
		);
		sample_motion(destination, later, pool, blend_poses, loop_end);
		for ((previous, later), starting) in self.destination_previous.iter_mut().zip(&*loop_end).zip(&*loop_start) {
			*previous = shifted_sample(*previous, *later, starting);
		}
		self.inertializer
			.begin(
				&self.active_previous,
				&self.active_current,
				&self.destination_previous,
				loop_start,
				interval,
				blend,
			)
			.expect("player pose buffers must match the canonical skeleton");
		// Root motion runs from where the destination starts, which replaces the earlier sample the inertializer no
		// longer needs.
		sample_motion(
			destination,
			start,
			pool,
			blend_poses,
			&mut self.destination_previous,
		);
		let root_motion = self.root_delta(destination, wrapped_loops, pool, Sampled::Destination);

		self.inertializer
			.apply(&self.destination_current, leftover, &mut self.local_pose)
			.expect("player pose buffers must match the canonical skeleton");
		self.remove_root_motion_from_visual_pose();
		self.write_global_pose();
		root_motion
	}

	/// Keeps the pose shown last frame as the previous one before this frame writes a new pose.
	fn begin_frame(&mut self) {
		std::mem::swap(&mut self.local_pose, &mut self.local_previous);
	}

	/// Shows last frame's pose again when no clip is ready to sample.
	fn hold_frame(&mut self) {
		self.local_pose.copy_from_slice(&self.local_previous);
	}

	/// Starts inertializing from the pose on screen, moving over `interval`, toward the sampled destination.
	fn begin_from_display(&mut self, interval: MediaTime, duration: MediaTime) {
		self.inertializer
			.begin(
				&self.local_previous,
				&self.local_pose,
				&self.destination_previous,
				&self.destination_current,
				interval,
				duration,
			)
			.expect("player pose buffers must match the canonical skeleton");
	}

	/// Restores the canonical pose while no clip is ready.
	fn write_rest_pose(&mut self) {
		for (output, node) in self.local_pose.iter_mut().zip(&self.target.nodes) {
			*output = node.rest_local;
		}
		self.write_global_pose();
	}

	/// Removes extracted motion from the visual pose so the owning object applies it once.
	fn remove_root_motion_from_visual_pose(&mut self) {
		let Some(root_motion) = self.root_motion else {
			return;
		};
		let local = &mut self.local_pose[root_motion.node];
		local.translation = root_motion
			.translation
			.select(root_motion.reference.translation, local.translation);
		if root_motion.rotation == RootMotionRotation::Full {
			local.rotation = root_motion.reference.rotation;
		}
	}

	/// Updates the borrowed global pose after local sampling.
	fn write_global_pose(&mut self) {
		write_global_pose(&self.target, &self.local_pose, &mut self.global_pose)
			.expect("player local pose always matches its canonical skeleton");
	}
}

/// The `AnimationGraphPlayerError` enum reports invalid player inputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnimationGraphPlayerError {
	/// Choose a root-motion node that exists in the canonical skeleton.
	RootMotionNodeNotFound {
		/// The requested root-motion node name.
		name: String,
	},
	/// Choose a root-motion node name that is unique in the canonical skeleton.
	DuplicateRootMotionNodeName {
		/// The ambiguous root-motion node name.
		name: String,
	},
	/// Request a state created by the same graph as this player.
	StateFromDifferentGraph,
	/// Pass a nonnegative frame duration to [`AnimationGraphPlayer::advance`].
	NegativeDelta,
}

impl fmt::Display for AnimationGraphPlayerError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::RootMotionNodeNotFound { name } => write!(
				formatter,
				"Animation root-motion node was not found. The most likely cause is that the canonical skeleton has no node named '{name}'."
			),
			Self::DuplicateRootMotionNodeName { name } => write!(
				formatter,
				"Animation root-motion node name is ambiguous. The most likely cause is that the canonical skeleton has more than one node named '{name}'."
			),
			Self::StateFromDifferentGraph => write!(
				formatter,
				"Animation state belongs to another graph. The most likely cause is passing a state ID from a different graph definition."
			),
			Self::NegativeDelta => write!(
				formatter,
				"Animation graph advance delta is negative. The most likely cause is passing a timeline offset instead of a frame duration."
			),
		}
	}
}

impl std::error::Error for AnimationGraphPlayerError {}

/// Resolves the stable root-motion name once so steady-state sampling retains a numeric node index.
fn resolve_root_motion_target(
	target: &Skeleton,
	root_motion: Option<RootMotionSettings<'_>>,
) -> Result<Option<RootMotionTarget>, AnimationGraphPlayerError> {
	let Some(settings) = root_motion else {
		return Ok(None);
	};
	let name = settings.node_name;
	let mut matches = target
		.nodes
		.iter()
		.enumerate()
		.filter(|(_, node)| node.name.as_deref() == Some(name));
	let Some((node, root)) = matches.next() else {
		return Err(AnimationGraphPlayerError::RootMotionNodeNotFound { name: name.into() });
	};
	if matches.next().is_some() {
		return Err(AnimationGraphPlayerError::DuplicateRootMotionNodeName { name: name.into() });
	}
	Ok(Some(RootMotionTarget {
		node,
		reference: root.rest_local,
		translation: settings.translation,
		rotation: settings.rotation,
	}))
}

/// Samples one clip of a motion at `phase` into canonical-skeleton local transforms.
fn sample_clip(sample: &ClipBinding, phase: f32, pool: &AnimationPool, output: &mut [LocalTransform]) {
	let resident = pool
		.resident(&sample.resource_id)
		.expect("a ready motion's clips must remain resident");
	resident.packed.sample_pose(phase * sample.duration, &sample.targets, output);
}

/// Samples the root chain's nodes of one clip of a motion at `phase` into `output`, one transform per chain node.
fn sample_chain(sample: &ClipBinding, phase: f32, pool: &AnimationPool, nodes: &[u32], output: &mut [LocalTransform]) {
	let resident = pool
		.resident(&sample.resource_id)
		.expect("a ready motion's clips must remain resident");
	resident
		.packed
		.sample_nodes(phase * sample.duration, &sample.targets, nodes, output);
}

/// Samples a motion at `phase`, mixing a blend's weighted clips through `blend_poses`.
fn sample_motion(
	motion: &RuntimeMotion,
	phase: f32,
	pool: &AnimationPool,
	blend_poses: &mut [Vec<LocalTransform>; BlendWeights::MAX],
	output: &mut [LocalTransform],
) {
	// One weighted clip, a clip state's or a blend's at one of its samples, samples straight into the output.
	let mut weighted = motion.weights.iter();
	if let (1, Some((sample, _))) = (weighted.len(), weighted.next()) {
		sample_clip(&motion.samples[sample], phase, pool, output);
		return;
	}

	let nodes = output.len();
	let mut weights = [0.0; BlendWeights::MAX];
	let mut count = 0;
	for (sample, weight) in motion.weights.iter() {
		sample_clip(&motion.samples[sample], phase, pool, &mut blend_poses[count][..nodes]);
		weights[count] = weight;
		count += 1;
	}
	let [first, second, third] = blend_poses;
	let poses = [&first[..nodes], &second[..nodes], &third[..nodes]];
	blend_local_poses(&poses[..count], &weights[..count], output).expect("blend weights are positive and sum to one");
}

/// The `RootChain` struct holds the nodes root motion measures, the root motion node and its ancestors, and their
/// transforms at the moments one root delta measures between.
///
/// A root delta composes only these nodes, so measuring one clip's travel samples them alone instead of whole poses.
struct RootChain {
	/// The root motion node, then each ancestor up to the skeleton's root. Empty without root motion.
	nodes: Box<[u32]>,
	previous: Box<[LocalTransform]>,
	current: Box<[LocalTransform]>,
	loop_start: Box<[LocalTransform]>,
	loop_end: Box<[LocalTransform]>,
}

impl RootChain {
	/// Collects the chain of `root_motion`'s node in `skeleton`, and sizes its buffers.
	fn new(skeleton: &Skeleton, root_motion: Option<RootMotionTarget>) -> Self {
		let nodes: Box<[u32]> = std::iter::successors(root_motion.map(|root_motion| root_motion.node as u32), |&node| {
			skeleton.nodes[node as usize].parent
		})
		.collect();
		let buffer = || vec![LocalTransform::identity(); nodes.len()].into_boxed_slice();
		Self {
			previous: buffer(),
			current: buffer(),
			loop_start: buffer(),
			loop_end: buffer(),
			nodes,
		}
	}
}

/// Calculates a motion's root delta since its previous phase.
///
/// A single clip measures between its retained `previous` and `current` poses. A blend measures each weighted clip
/// on its own and mixes the deltas by weight: the blended pose's root can jump when the weights change, but each
/// clip's own travel cannot. Either way, only the root chain's nodes are read or sampled.
fn motion_root_delta(
	root_motion: Option<RootMotionTarget>,
	chain: &mut RootChain,
	motion: &RuntimeMotion,
	previous: &[LocalTransform],
	current: &[LocalTransform],
	wrapped_loops: usize,
	pool: &AnimationPool,
) -> RootMotionDelta {
	let Some(root_motion) = root_motion else {
		return RootMotionDelta::IDENTITY;
	};
	if let [sample] = motion.samples {
		// A single clip's poses are already sampled, so the chain copies its links from them.
		for (link, &node) in chain.nodes.iter().enumerate() {
			chain.previous[link] = previous[node as usize];
			chain.current[link] = current[node as usize];
		}
		return clip_root_delta(root_motion, chain, sample, wrapped_loops, pool);
	}

	let mut mixed: Option<(RootMotionDelta, f32)> = None;
	for (sample, weight) in motion.weights.iter() {
		let sample = &motion.samples[sample];
		sample_chain(sample, motion.previous_phase, pool, &chain.nodes, &mut chain.previous);
		sample_chain(sample, motion.phase, pool, &chain.nodes, &mut chain.current);
		let delta = clip_root_delta(root_motion, chain, sample, wrapped_loops, pool);
		// Folding each delta in by its share of the weight so far gives the weighted mean.
		mixed = Some(match mixed {
			None => (delta, weight),
			Some((sum, total)) => (sum.blend(delta, weight / (total + weight)), total + weight),
		});
	}
	mixed.map_or(RootMotionDelta::IDENTITY, |(delta, _)| delta)
}

/// Calculates one clip's root delta between the chain's previous and current transforms, preserving forward progress
/// across loop boundaries.
///
/// A clip that wrapped samples its chain at both ends, to join the travel up to its end, any whole loops, and the
/// travel from its start.
fn clip_root_delta(
	root_motion: RootMotionTarget,
	chain: &mut RootChain,
	sample: &ClipBinding,
	wrapped_loops: usize,
	pool: &AnimationPool,
) -> RootMotionDelta {
	let previous = object_space_root(root_motion, &chain.previous);
	let current = object_space_root(root_motion, &chain.current);
	if wrapped_loops == 0 {
		return RootMotionDelta::between(previous, current);
	}

	// Sample the clip ends only for a loop crossing. This keeps the common
	// steady-state path to one clip sample while preserving forward root motion.
	sample_chain(sample, 1.0, pool, &chain.nodes, &mut chain.loop_end);
	sample_chain(sample, 0.0, pool, &chain.nodes, &mut chain.loop_start);
	let end = object_space_root(root_motion, &chain.loop_end);
	let start = object_space_root(root_motion, &chain.loop_start);

	let mut delta = RootMotionDelta::between(previous, end);
	let full_loop = RootMotionDelta::between(start, end);
	for _ in 1..wrapped_loops {
		delta = delta.then(full_loop);
	}
	delta.then(RootMotionDelta::between(start, current))
}

/// Returns the root motion node's transform in the owning object's space, with only its extracted channels.
///
/// `chain` holds the local transforms of the root motion node and each ancestor up to the skeleton's root. Replacing
/// the unselected channels with their reference values before composing lets the selected local axes inherit
/// authored parent scale and rotation, while pose-only hip motion cannot leak into the owning object.
fn object_space_root(root_motion: RootMotionTarget, chain: &[LocalTransform]) -> LocalTransform {
	let (node, ancestors) = chain
		.split_first()
		.expect("a root chain starts with the root motion node");
	ancestors
		.iter()
		.fold(extracted_root_transform(root_motion, *node), |result, ancestor| {
			compose_local_transform(*ancestor, result)
		})
}

/// Keeps selected root-motion channels and restores all other channels to the reference pose.
fn extracted_root_transform(root_motion: RootMotionTarget, mut transform: LocalTransform) -> LocalTransform {
	transform.translation = root_motion
		.translation
		.select(transform.translation, root_motion.reference.translation);
	if root_motion.rotation == RootMotionRotation::None {
		transform.rotation = root_motion.reference.rotation;
	}
	transform
}

/// Prepends `parent` to `child`, preserving the hierarchy's scale-rotate-translate order.
fn compose_local_transform(parent: LocalTransform, child: LocalTransform) -> LocalTransform {
	LocalTransform {
		translation: parent.translation + parent.rotation.rotate_vector(parent.scale * child.translation),
		rotation: parent.rotation.compose(child.rotation),
		scale: parent.scale * child.scale,
	}
}

#[cfg(test)]
mod tests {

	use resource_management::{
		Reference,
		resources::{
			animation::{Animation, NodeTrack, RotationCurve, TranslationCurve},
			skeleton::{LocalTransform, Skeleton},
		},
	};

	use super::super::tests::{packed_test_animation_bytes, test_animation, test_motion, test_motion_from};
	use super::*;
	use crate::{MediaTime, animation::test_node};

	fn ready(result: Result<Option<AnimationGraphPose<'_>>, AnimationGraphPlayerError>) -> AnimationGraphPose<'_> {
		result
			.expect("animation evaluation should succeed")
			.expect("resident initial clip should produce a pose")
	}

	#[test]
	fn player_prefetches_direct_neighbors_as_it_enters_graph_states() {
		let idle_animation = test_animation("idle", 0.0);
		let start_animation = test_animation("start", 1.0);
		let byte_budget = packed_test_animation_bytes("idle", 0.0)
			+ packed_test_animation_bytes("start", 1.0)
			+ packed_test_animation_bytes("walk", 2.0)
			+ packed_test_animation_bytes("backward", -1.0);
		let mut pool = AnimationPool::detached(byte_budget);
		pool.admit("idle.animation".into(), idle_animation);

		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let backward = builder.state("backward").with(AnimationClip::looping("backward.animation"));
		let _unrelated = builder.state("unrelated").with(AnimationClip::looping("unrelated.animation"));
		let start_walk = idle
			.to(walk)
			.with(AnimationClip::once("start.animation"))
			.when(MediaTime::ZERO);
		idle.to(backward).when(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, None);

		assert!(!pool.entries.contains_key("start.animation"));
		player
			.advance(MediaTime::ZERO, idle.id(), &mut pool)
			.expect("enters the initial idle state");

		assert!(pool.entries.contains_key("start.animation"));
		assert!(pool.entries.contains_key("backward.animation"));
		assert!(!pool.entries.contains_key("walk.animation"));
		assert!(!pool.entries.contains_key("unrelated.animation"));

		pool.admit("start.animation".into(), start_animation);
		player
			.advance(MediaTime::ZERO, walk.id(), &mut pool)
			.expect("starts the walk transition clip");

		assert_eq!(player.state(), Some(start_walk.id()));
		assert!(pool.entries.contains_key("walk.animation"));
		assert!(!pool.entries.contains_key("unrelated.animation"));
	}

	#[test]
	fn player_returns_root_motion_and_removes_it_from_the_visual_pose() {
		let idle = test_animation("idle", 1.0);
		let run = test_animation("run", 3.0);
		let byte_budget = packed_test_animation_bytes("idle", 1.0).saturating_add(packed_test_animation_bytes("run", 3.0));
		let mut pool = AnimationPool::detached(byte_budget);
		pool.admit("idle.animation".into(), idle);
		pool.admit("run.animation".into(), run);

		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let run = builder.state("run").with(AnimationClip::looping("run.animation"));
		idle.to(run).when(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("root")));

		let initial = ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));

		assert_eq!(initial.local_pose[0], LocalTransform::identity());
		let root_motion = ready(player.advance(MediaTime::from_millis(500), idle.id(), &mut pool)).root_motion;

		assert_eq!(root_motion.translation, math::Vector::new(0.5, 0.0, 0.0));
		assert_eq!(
			ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool)).local_pose[0].translation,
			math::Vector::zero()
		);

		let switched = ready(player.advance(MediaTime::ZERO, run.id(), &mut pool));

		assert_eq!(switched.root_motion.translation, math::Vector::zero());
		assert_eq!(player.state(), Some(run.id()));
		assert_eq!(
			ready(player.advance(MediaTime::from_millis(500), run.id(), &mut pool))
				.root_motion
				.translation,
			math::Vector::new(1.5, 0.0, 0.0)
		);
		assert_eq!(
			ready(player.advance(MediaTime::from_millis(750), run.id(), &mut pool))
				.root_motion
				.translation,
			math::Vector::new(2.25, 0.0, 0.0)
		);
	}

	#[test]
	fn players_sharing_a_graph_play_as_each_would_alone() {
		let clips = || {
			pool_with(vec![
				("idle.animation", test_animation("idle", 1.0)),
				("run.animation", test_animation("run", 3.0)),
			])
		};
		let graph = || {
			let builder = AnimationGraph::builder();
			let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
			let run = builder.state("run").with(AnimationClip::looping("run.animation"));
			idle.to(run).when(MediaTime::from_millis(200));
			run.to(idle).when(MediaTime::from_millis(200));
			(builder.build(idle).expect("graph should build"), idle.id(), run.id())
		};
		let (shared, idle, run) = graph();
		let (alone, alone_idle, alone_run) = graph();
		let mut shared_pool = clips();
		let mut alone_pool = clips();

		// The first player enters every state before the second exists, so the second plays from what it bound.
		let mut first = shared_pool.create_player(&shared, Some(RootMotionSettings::full("root")));
		for (delta, state) in [(0, run), (300, run), (100, idle), (300, idle)] {
			ready(first.advance(MediaTime::from_millis(delta), state, &mut shared_pool));
		}
		let mut second = shared_pool.create_player(&shared, Some(RootMotionSettings::full("root")));
		let mut solo = alone_pool.create_player(&alone, Some(RootMotionSettings::full("root")));

		for (delta, requested) in [(0, false), (300, false), (100, true), (150, true), (250, true), (100, false)] {
			let delta = MediaTime::from_millis(delta);
			let (state, alone_state) = if requested { (run, alone_run) } else { (idle, alone_idle) };
			// The first player keeps playing elsewhere in the graph between the other two's frames.
			ready(first.advance(MediaTime::from_millis(70), run, &mut shared_pool));
			let shared_pose = ready(second.advance(delta, state, &mut shared_pool));
			let solo_pose = ready(solo.advance(delta, alone_state, &mut alone_pool));

			assert_eq!(shared_pose.local_pose, solo_pose.local_pose);
			assert_eq!(shared_pose.global_pose, solo_pose.global_pose);
			assert_eq!(shared_pose.root_motion, solo_pose.root_motion);
		}
	}

	#[test]
	fn players_advanced_together_match_each_player_advanced_alone() {
		let mut pool = pool_with(vec![
			("idle.animation", test_animation("idle", 1.0)),
			("run.animation", test_animation("run", 3.0)),
		]);
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let run = builder.state("run").with(AnimationClip::looping("run.animation"));
		idle.to(run).anytime(MediaTime::from_millis(200));
		run.to(idle).anytime(MediaTime::from_millis(200));
		let graph = builder.build(idle).expect("graph should build");
		// Enough players that a crowd spreads over more than one lane wherever the machine has the lanes.
		let count = 3 * PLAYERS_PER_LANE;
		let mut crowd: Vec<_> = (0..count)
			.map(|_| pool.create_player(&graph, Some(RootMotionSettings::full("root"))))
			.collect();
		let mut alone: Vec<_> = (0..count)
			.map(|_| pool.create_player(&graph, Some(RootMotionSettings::full("root"))))
			.collect();
		let mut alley = crate::core::alley::Alley::with_parallelism(4);

		for (frame, delta) in [0, 16, 16, 33, 8, 16, 50, 16, 16, 16].into_iter().enumerate() {
			let delta = MediaTime::from_millis(delta);
			// Each player switches between the states on its own schedule, so the crowd holds every phase of a blend.
			let request = |player: usize| if (frame + player) % 5 < 2 { run.id() } else { idle.id() };
			for (player, crowd) in crowd.iter_mut().enumerate() {
				crowd.request(request(player)).expect("requests come from the graph");
			}
			pool.advance_players(&mut crowd, delta, &mut alley)
				.expect("players should advance");

			for (player, (crowd, alone)) in crowd.iter().zip(&mut alone).enumerate() {
				let alone = ready(alone.advance(delta, request(player), &mut pool));
				let crowd = crowd.pose().expect("a resident initial clip gives every player a pose");
				assert_eq!(crowd.local_pose, alone.local_pose, "player {player} in frame {frame}");
				assert_eq!(crowd.global_pose, alone.global_pose, "player {player} in frame {frame}");
				assert_eq!(crowd.root_motion, alone.root_motion, "player {player} in frame {frame}");
				assert_eq!(crowd.state, alone.state, "player {player} in frame {frame}");
			}
		}
	}

	#[test]
	fn player_cancels_loading_transition_states_and_completes_loaded_ones() {
		let idle_animation = test_animation("idle", 0.0);
		let start_animation = test_animation("start", 1.0);
		let walk_animation = test_animation("walk", 2.0);
		let byte_budget = packed_test_animation_bytes("idle", 0.0)
			+ packed_test_animation_bytes("start", 1.0)
			+ packed_test_animation_bytes("walk", 2.0);
		let mut pool = AnimationPool::detached(byte_budget);
		pool.admit("idle.animation".into(), idle_animation);

		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let start_walk = idle
			.to(walk)
			.with(AnimationClip::once("start.animation"))
			.when(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, None);

		player
			.advance(MediaTime::ZERO, idle.id(), &mut pool)
			.expect("initial idle pose");
		player
			.advance(MediaTime::ZERO, walk.id(), &mut pool)
			.expect("queues start-walk clip");

		assert_eq!(player.state(), Some(idle.id()));
		player
			.advance(MediaTime::ZERO, idle.id(), &mut pool)
			.expect("cancels stale start-walk request");

		assert_eq!(player.state(), Some(idle.id()));

		pool.admit("start.animation".into(), start_animation);
		pool.admit("walk.animation".into(), walk_animation);
		player
			.advance(MediaTime::ZERO, walk.id(), &mut pool)
			.expect("starts transition state");

		assert_eq!(player.state(), Some(start_walk.id()));
		player
			.advance(MediaTime::from_seconds(1), idle.id(), &mut pool)
			.expect("finishes transition-state clip and enters its completion state despite the request changing");

		assert_eq!(player.state(), Some(walk.id()));
	}

	#[test]
	fn player_enters_completion_states_in_the_frame_a_one_shot_ends() {
		let idle_animation = test_animation("idle", 0.0);
		let start_animation = test_animation("start", 1.0);
		let walk_animation = test_animation("walk", 2.0);
		let byte_budget = packed_test_animation_bytes("idle", 0.0)
			+ packed_test_animation_bytes("start", 1.0)
			+ packed_test_animation_bytes("walk", 2.0);
		let mut pool = AnimationPool::detached(byte_budget);
		pool.admit("idle.animation".into(), idle_animation);
		pool.admit("start.animation".into(), start_animation);
		pool.admit("walk.animation".into(), walk_animation);

		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let start_walk = idle
			.to(walk)
			.with(AnimationClip::once("start.animation"))
			.anytime(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("root")));

		ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
		ready(player.advance(MediaTime::ZERO, walk.id(), &mut pool));
		assert_eq!(player.state(), Some(start_walk.id()));
		ready(player.advance(MediaTime::from_millis(750), walk.id(), &mut pool));

		// The last 250 ms of the start clip move 0.25, and the 250 ms left over play the walk loop for another 0.5.
		let crossing = ready(player.advance(MediaTime::from_millis(500), walk.id(), &mut pool));
		math::assert_float_eq!(crossing.root_motion.translation.x(), 0.75);
		assert_eq!(player.state(), Some(walk.id()));

		// Playback continues from the carried-over walk time instead of repeating the start clip's last pose.
		let next = ready(player.advance(MediaTime::from_millis(250), walk.id(), &mut pool));
		math::assert_float_eq!(next.root_motion.translation.x(), 0.5);
	}

	#[test]
	fn waiting_routes_start_in_the_frame_their_source_reaches_a_wait_frame() {
		let mut pool = pool_with(vec![
			("idle.animation", test_animation("idle", 1.0)),
			("walk.animation", test_animation("walk", 4.0)),
		]);
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		// Frame 5 of a 10 fps clip is half a second in.
		idle.to(walk).at_frame(5).anytime(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("root")));

		ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
		ready(player.advance(MediaTime::from_millis(200), idle.id(), &mut pool));
		ready(player.advance(MediaTime::from_millis(200), walk.id(), &mut pool));
		assert_eq!(player.state(), Some(idle.id()), "the route must wait for frame 5");
		assert_eq!(player.waiting_for(), Some(walk.id()));

		// Requesting another state cancels the wait.
		ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
		assert_eq!(player.waiting_for(), None);

		// Idle plays the 100 ms up to frame 5, moving 0.1, and walk the 100 ms left over, moving 0.4.
		let crossing = ready(player.advance(MediaTime::from_millis(200), walk.id(), &mut pool));
		math::assert_float_eq!(crossing.root_motion.translation.x(), 0.5);
		assert_eq!(player.state(), Some(walk.id()));
		assert_eq!(player.waiting_for(), None);
	}

	#[test]
	fn time_until_route_reports_the_nearest_wait_point_of_each_route() {
		let mut pool = pool_with(vec![
			("idle.animation", test_animation("idle", 1.0)),
			("walk.animation", test_animation("walk", 4.0)),
			("run.animation", test_animation("run", 4.0)),
		]);
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let run = builder.state("run").with(AnimationClip::looping("run.animation"));
		let crouch = builder.state("crouch").with(AnimationClip::looping("idle.animation"));
		// Frames of a 10 fps clip: 2 is 200 ms in, 5 is 500 ms in and 8 is 800 ms in.
		idle.to(walk).at_frame(5).anytime(MediaTime::ZERO);
		idle.to(run).at_frames("idle.animation", [2, 8]).anytime(MediaTime::ZERO);
		idle.to(crouch).anytime(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("root")));
		assert_eq!(
			player.time_until_route(walk.id()),
			None,
			"nothing plays before the initial clip loads"
		);

		ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
		ready(player.advance(MediaTime::from_millis(300), idle.id(), &mut pool));
		let millis = |time: Option<MediaTime>| time.map(|time| (time.as_seconds_f32() * 1000.0).round() as i64);
		assert_eq!(millis(player.time_until_route(walk.id())), Some(200));
		assert_eq!(
			millis(player.time_until_route(run.id())),
			Some(500),
			"frame 2 has passed, so frame 8 is next"
		);
		assert_eq!(
			player.time_until_route(crouch.id()),
			Some(MediaTime::ZERO),
			"a route that does not wait starts at once"
		);
		assert_eq!(
			player.time_until_route(idle.id()),
			None,
			"the playing state has no route to itself"
		);

		// A looping source wraps around to the first wait point.
		ready(player.advance(MediaTime::from_millis(600), idle.id(), &mut pool));
		assert_eq!(millis(player.time_until_route(run.id())), Some(300));
	}

	#[test]
	fn routes_enter_their_target_at_an_entry_frame() {
		let mut pool = pool_with(vec![
			("idle.animation", test_animation("idle", 1.0)),
			("stop.animation", test_animation("stop", 1.0)),
			("walk.animation", test_animation("walk", 4.0)),
		]);
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk).entering_at_frame("walk.animation", 5).anytime(MediaTime::ZERO);
		walk.to(idle)
			.entering_at_frame("idle.animation", 2)
			.with(AnimationClip::once("stop.animation"))
			.anytime(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, None);

		ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
		let entered = ready(player.advance(MediaTime::ZERO, walk.id(), &mut pool)).local_pose[0];
		assert_eq!(player.state(), Some(walk.id()));
		math::assert_float_eq!(entered.translation.x(), 2.0);

		// The stop clip's completion starts idle at frame 2 and plays the 100 ms left over from there.
		ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
		let landed = ready(player.advance(MediaTime::from_millis(1100), idle.id(), &mut pool)).local_pose[0];
		assert_eq!(player.state(), Some(idle.id()));
		math::assert_float_eq!(landed.translation.x(), 0.3);
	}

	#[test]
	fn one_shot_routes_start_from_the_pose_and_pace_of_their_source() {
		// The walk moves at 3 m/s and wraps every second; the stop picks up where frame 5 of the walk is, at x = 1.5. A
		// stop that keeps the walk's pace leaves the blend nothing to hide, so the pose must keep its pace across it. A
		// slower stop leaves a difference in pace the blend takes up, which a 250 ms blend at 30 fps spreads over more
		// than a frame; the waiting route used to count the stop's own travel as an offset and the plain one to decay
		// its offset over the frame it was measured at.
		for (stop_end, most) in [(3.0, 1e-3), (2.0, 0.06)] {
			for waits in [true, false] {
				let mut pool = pool_with(vec![
					("idle.animation", test_animation("idle", 0.0)),
					("walk.animation", test_animation("walk", 3.0)),
					(
						"stop.animation",
						test_motion_from("stop", [1.5, 0.0, 0.0], [stop_end, 0.0, 0.0], 0.5),
					),
				]);
				let builder = AnimationGraph::builder();
				let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
				let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
				let route = walk.to(idle);
				let route = if waits { route.at_frame(5) } else { route };
				route
					.with(AnimationClip::once("stop.animation"))
					.exit(MediaTime::from_millis(250))
					.anytime(MediaTime::from_millis(250));
				let graph = builder.build(walk).expect("graph should build");
				let mut player = pool.create_player(&graph, None);

				let frame = MediaTime::from_seconds_f32(1.0 / 30.0);
				ready(player.advance(MediaTime::ZERO, walk.id(), &mut pool));
				// The waiting route is asked for early and reaches frame 5 partway through a frame; the plain one is asked
				// for on the frame the walk reaches x = 1.5.
				let (lead, request_at) = if waits {
					(MediaTime::from_millis(380), 2)
				} else {
					(MediaTime::from_millis(500), 3)
				};
				ready(player.advance(lead - frame * 3, walk.id(), &mut pool));
				let positions: Vec<f32> = (0..12)
					.map(|index| {
						let requested = if index >= request_at { idle.id() } else { walk.id() };
						ready(player.advance(frame, requested, &mut pool)).local_pose[0]
							.translation
							.x()
					})
					.collect();
				let jolt = positions
					.windows(3)
					.map(|window| (window[2] - 2.0 * window[1] + window[0]).abs())
					.fold(0.0, f32::max);
				assert!(
					jolt < most,
					"a {} route into a stop ending at {stop_end} changed pace by {jolt} m per frame: {positions:?}",
					if waits { "waiting" } else { "plain" }
				);
			}
		}
	}

	#[test]
	fn hand_over_blends_across_a_loop_wrap_without_a_lurch() {
		let mut pool = pool_with(vec![
			("idle.animation", test_animation("idle", 0.0)),
			("stop.animation", test_animation("stop", 1.0)),
			("walk.animation", test_animation("walk", 4.0)),
		]);
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk)
			.entering_at_frame("walk.animation", 10)
			.with(AnimationClip::once("stop.animation"))
			.exit(MediaTime::from_millis(1000))
			.anytime(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, None);

		ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
		ready(player.advance(MediaTime::ZERO, walk.id(), &mut pool));
		let before = ready(player.advance(MediaTime::from_millis(900), walk.id(), &mut pool)).local_pose[0];
		math::assert_float_eq!(before.translation.x(), 0.9);

		// The stop clip ends 100 ms in, and the walk, entered at its last frame, plays the 20 ms left over across its
		// wrap, where its root jumps back from 4 to 0. The blend hides that jump, and its velocity, measured inside one
		// cycle, moves the pose no faster than the walk does.
		let after = ready(player.advance(MediaTime::from_millis(120), walk.id(), &mut pool)).local_pose[0];
		assert_eq!(player.state(), Some(walk.id()));
		assert!(
			(after.translation.x() - before.translation.x()).abs() < 0.12 * 4.0,
			"the hand-over moved the root from {} to {}",
			before.translation.x(),
			after.translation.x()
		);
	}

	#[test]
	fn player_inertializes_one_shot_exits_and_keeps_their_root_motion() {
		let byte_budget = packed_test_animation_bytes("idle", 0.0)
			+ packed_test_animation_bytes("start", 1.0)
			+ packed_test_animation_bytes("walk", 2.0);
		let mut pool = AnimationPool::detached(byte_budget);
		pool.admit("idle.animation".into(), test_animation("idle", 0.0));
		pool.admit("start.animation".into(), test_animation("start", 1.0));
		pool.admit("walk.animation".into(), test_animation("walk", 2.0));

		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk)
			.with(AnimationClip::once("start.animation"))
			.exit(MediaTime::from_millis(500))
			.anytime(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");

		let mut visual = pool.create_player(&graph, None);
		let mut moving = pool.create_player(&graph, Some(RootMotionSettings::full("root")));
		for player in [&mut visual, &mut moving] {
			ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
			ready(player.advance(MediaTime::ZERO, walk.id(), &mut pool));
			ready(player.advance(MediaTime::from_millis(750), walk.id(), &mut pool));
		}

		// The start clip ends at x = 1 and the walk loop's leftover 250 ms sit at x = 0.5; the blend keeps the pose
		// between them instead of cutting to the walk loop.
		let crossing = ready(visual.advance(MediaTime::from_millis(500), walk.id(), &mut pool)).local_pose[0]
			.translation
			.x();
		assert!(
			crossing > 0.5 && crossing < 1.0,
			"blended pose should sit between both clips, got {crossing}"
		);
		assert_eq!(visual.state(), Some(walk.id()));
		let settled = ready(visual.advance(MediaTime::from_millis(500), walk.id(), &mut pool)).local_pose[0]
			.translation
			.x();
		math::assert_float_eq!(settled, 1.5);

		// Root motion stays continuous: the start clip's last 250 ms plus the walk loop's leftover 250 ms, then the
		// walk loop alone while the pose still blends.
		math::assert_float_eq!(
			ready(moving.advance(MediaTime::from_millis(500), walk.id(), &mut pool))
				.root_motion
				.translation
				.x(),
			0.75
		);
		math::assert_float_eq!(
			ready(moving.advance(MediaTime::from_millis(100), walk.id(), &mut pool))
				.root_motion
				.translation
				.x(),
			0.2
		);
	}

	/// Admits clips into a pool sized for exactly them.
	fn pool_with(clips: Vec<(&str, PackedAnimationData)>) -> AnimationPool {
		let mut pool = AnimationPool::detached(clips.iter().map(|(_, clip)| clip.resident_bytes()).sum());
		for (id, clip) in clips {
			pool.admit(id.into(), clip);
		}
		pool
	}

	#[test]
	fn linear_blends_mix_pose_and_root_motion_by_position() {
		let mut pool = pool_with(vec![
			("slow.animation", test_animation("slow", 1.0)),
			("fast.animation", test_animation("fast", 3.0)),
		]);
		let builder = AnimationGraph::builder();
		let speed = builder.state("speed").linear(
			AnimationBlend::new()
				.sample(1.0, AnimationClip::looping("fast.animation"))
				.sample(0.0, AnimationClip::looping("slow.animation")),
		);
		let graph = builder.build(speed).expect("graph should build");
		let mut visual = pool.create_player(&graph, None);
		let mut moving = pool.create_player(&graph, Some(RootMotionSettings::full("root")));

		ready(visual.advance(MediaTime::ZERO, speed.id().at(0.25), &mut pool));
		// A quarter of the way to the fast clip: 0.75 of the slow clip's 0.5 plus 0.25 of the fast clip's 1.5.
		let pose = ready(visual.advance(MediaTime::from_millis(500), speed.id().at(0.25), &mut pool));
		math::assert_float_eq!(pose.local_pose[0].translation.x(), 0.75);

		ready(moving.advance(MediaTime::ZERO, speed.id().at(0.25), &mut pool));
		let root_motion = ready(moving.advance(MediaTime::from_millis(500), speed.id().at(0.25), &mut pool)).root_motion;
		math::assert_float_eq!(root_motion.translation.x(), 0.75);
	}

	#[test]
	fn blended_clips_share_one_phase_over_a_weighted_duration() {
		let mut pool = pool_with(vec![
			("short.animation", test_motion("short", [1.0, 0.0, 0.0], 1.0)),
			("long.animation", test_motion("long", [1.0, 0.0, 0.0], 3.0)),
		]);
		let builder = AnimationGraph::builder();
		let cycle = builder.state("cycle").linear(
			AnimationBlend::new()
				.sample(0.0, AnimationClip::looping("short.animation"))
				.sample(1.0, AnimationClip::looping("long.animation")),
		);
		let graph = builder.build(cycle).expect("graph should build");
		let mut player = pool.create_player(&graph, None);

		ready(player.advance(MediaTime::ZERO, cycle.id().at(0.5), &mut pool));
		// An even mix of 1 s and 3 s cycles lasts 2 s, so after 1 s both clips sit halfway through: x = 0.5 in each.
		let pose = ready(player.advance(MediaTime::from_seconds(1), cycle.id().at(0.5), &mut pool));
		math::assert_float_eq!(pose.local_pose[0].translation.x(), 0.5);
	}

	#[test]
	fn planar_blends_move_toward_the_requested_direction() {
		let mut pool = pool_with(vec![
			("forward.animation", test_motion("forward", [0.0, 0.0, 1.0], 1.0)),
			("right.animation", test_motion("right", [1.0, 0.0, 0.0], 1.0)),
			("backward.animation", test_motion("backward", [0.0, 0.0, -1.0], 1.0)),
			("left.animation", test_motion("left", [-1.0, 0.0, 0.0], 1.0)),
		]);
		let builder = AnimationGraph::builder();
		let walk = builder.state("walk").planar(
			AnimationBlend::new()
				.sample([0.0, 1.0], AnimationClip::looping("forward.animation"))
				.sample([1.0, 0.0], AnimationClip::looping("right.animation"))
				.sample([0.0, -1.0], AnimationClip::looping("backward.animation"))
				.sample([-1.0, 0.0], AnimationClip::looping("left.animation")),
		);
		let graph = builder.build(walk).expect("graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("root")));

		ready(player.advance(MediaTime::ZERO, walk.id().at([0.5, 0.5]), &mut pool));
		let root_motion = ready(player.advance(MediaTime::from_millis(500), walk.id().at([0.5, 0.5]), &mut pool)).root_motion;
		// Halfway between forward and right walks diagonally, at the clips' shared speed of 1 m/s mixed evenly.
		math::assert_float_eq!(root_motion.translation.x(), 0.25);
		math::assert_float_eq!(root_motion.translation.z(), 0.25);

		// Requesting the state without a position keeps the last one.
		let root_motion = ready(player.advance(MediaTime::from_millis(500), walk.id(), &mut pool)).root_motion;
		math::assert_float_eq!(root_motion.translation.x(), 0.25);
		math::assert_float_eq!(root_motion.translation.z(), 0.25);
	}

	#[test]
	fn blends_entered_on_their_own_continue_at_their_space_position() {
		let mut pool = pool_with(vec![
			("forward.animation", test_motion("forward", [0.0, 0.0, 1.0], 1.0)),
			("right.animation", test_motion("right", [1.0, 0.0, 0.0], 1.0)),
			("backward.animation", test_motion("backward", [0.0, 0.0, -1.0], 1.0)),
			("left.animation", test_motion("left", [-1.0, 0.0, 0.0], 1.0)),
			("jump.animation", test_motion("jump", [0.0, 0.0, 0.0], 0.5)),
		]);
		let builder = AnimationGraph::builder();
		let moving = builder.planar_space("moving");
		let directions = |name: &str| {
			moving.state(name).planar(
				AnimationBlend::new()
					.sample([0.0, 1.0], AnimationClip::looping("forward.animation"))
					.sample([1.0, 0.0], AnimationClip::looping("right.animation"))
					.sample([0.0, -1.0], AnimationClip::looping("backward.animation"))
					.sample([-1.0, 0.0], AnimationClip::looping("left.animation")),
			)
		};
		let walk = directions("walk");
		let landed = directions("landed");
		walk.to(landed)
			.with(AnimationClip::once("jump.animation"))
			.when(MediaTime::ZERO);
		let graph = builder.build(walk).expect("graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("root")));

		ready(player.advance(MediaTime::ZERO, walk.id().at([1.0, 0.0]), &mut pool));
		ready(player.advance(MediaTime::ZERO, landed.id(), &mut pool));
		// The jump clip ends after 0.5 s and hands the rest of the frame to the landing, which nobody gave a position.
		let root_motion = ready(player.advance(MediaTime::from_millis(750), landed.id(), &mut pool)).root_motion;

		// The landing walks right like the walk did, rather than standing at the centre where the directions cancel.
		math::assert_float_eq!(root_motion.translation.x(), 0.25);
		math::assert_float_eq!(root_motion.translation.z(), 0.0);
	}

	#[test]
	fn one_shot_blends_keep_the_position_their_space_had_when_they_start() {
		let mut pool = pool_with(vec![
			("forward.animation", test_motion("forward", [0.0, 0.0, 1.0], 1.0)),
			("right.animation", test_motion("right", [1.0, 0.0, 0.0], 1.0)),
			("backward.animation", test_motion("backward", [0.0, 0.0, -1.0], 1.0)),
			("left.animation", test_motion("left", [-1.0, 0.0, 0.0], 1.0)),
			("idle.animation", test_motion("idle", [0.0, 0.0, 0.0], 1.0)),
		]);
		let directions = |playback: fn(&'static str) -> AnimationClip| {
			AnimationBlend::new()
				.sample([0.0, 1.0], playback("forward.animation"))
				.sample([1.0, 0.0], playback("right.animation"))
				.sample([0.0, -1.0], playback("backward.animation"))
				.sample([-1.0, 0.0], playback("left.animation"))
		};
		let builder = AnimationGraph::builder();
		let moving = builder.planar_space("moving");
		let walk = moving.state("walk").planar(directions(AnimationClip::looping));
		let idle = moving.state("idle").with(AnimationClip::looping("idle.animation"));
		// The stop must complete, so requesting the walk again while it plays only moves the space's position.
		walk.to(idle).with(directions(AnimationClip::once)).when(MediaTime::ZERO);
		idle.to(walk).when(MediaTime::ZERO);
		let graph = builder.build(walk).expect("graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("root")));

		ready(player.advance(MediaTime::ZERO, walk.id().at([1.0, 0.0]), &mut pool));
		ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
		let root_motion = ready(player.advance(MediaTime::from_millis(250), walk.id().at([0.0, 1.0]), &mut pool)).root_motion;

		// The stop heads right, where the walk was when it started, though the space now points forward.
		math::assert_float_eq!(root_motion.translation.x(), 0.25);
		math::assert_float_eq!(root_motion.translation.z(), 0.0);
	}

	#[test]
	fn routes_wait_for_the_frames_of_the_heaviest_named_clip() {
		let mut pool = pool_with(vec![
			("slow.animation", test_animation("slow", 1.0)),
			("fast.animation", test_animation("fast", 2.0)),
			("idle.animation", test_animation("idle", 0.0)),
		]);
		let builder = AnimationGraph::builder();
		let speed = builder.state("speed").linear(
			AnimationBlend::new()
				.sample(0.0, AnimationClip::looping("slow.animation"))
				.sample(1.0, AnimationClip::looping("fast.animation")),
		);
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		// Frames of a 10 fps clip: 2 is 200 ms in and 7 is 700 ms in. Both clips last 1 s, and so does any mix.
		speed
			.to(idle)
			.at_frames("slow.animation", [2])
			.at_frames("fast.animation", [7])
			.anytime(MediaTime::ZERO);
		let graph = builder.build(speed).expect("graph should build");
		let mut player = pool.create_player(&graph, None);
		let millis = |player: &AnimationGraphPlayer, position: f32| {
			player
				.time_until_route(idle.id())
				.map(|time| (time.as_seconds_f32() * 1000.0).round() as i64)
				.map(|time| (position, time))
		};

		for (position, expected) in [(0.25, 200), (0.75, 700), (0.5, 200)] {
			ready(player.advance(MediaTime::ZERO, speed.id().at(position), &mut pool));
			assert_eq!(
				millis(&player, position),
				Some((position, expected)),
				"the clip that weighs the most, or the first named among equals, picks the frames"
			);
		}
	}

	#[test]
	fn anytime_direct_routes_cut_short_one_shot_clips_and_when_routes_wait() {
		for interruptible in [true, false] {
			let mut pool = pool_with(vec![
				("idle.animation", test_animation("idle", 0.0)),
				("start.animation", test_animation("start", 1.0)),
				("walk.animation", test_animation("walk", 2.0)),
			]);
			let builder = AnimationGraph::builder();
			let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
			let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
			let start = idle
				.to(walk)
				.with(AnimationClip::once("start.animation"))
				.anytime(MediaTime::ZERO);
			let stop = walk.to(idle);
			if interruptible {
				stop.anytime(MediaTime::ZERO);
			} else {
				stop.when(MediaTime::ZERO);
			}
			let graph = builder.build(idle).expect("graph should build");
			let mut player = pool.create_player(&graph, None);

			ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
			ready(player.advance(MediaTime::ZERO, walk.id(), &mut pool));
			ready(player.advance(MediaTime::from_millis(100), walk.id(), &mut pool));
			assert_eq!(player.state(), Some(start.id()));
			ready(player.advance(MediaTime::from_millis(100), idle.id(), &mut pool));

			let expected = if interruptible { idle.id() } else { start.id() };
			assert_eq!(player.state(), Some(expected), "interruptible: {interruptible}");
		}
	}

	#[test]
	fn anytime_routes_redirect_a_blend_in_progress_from_the_pose_on_screen() {
		for interruptible in [true, false] {
			let mut pool = pool_with(vec![
				("idle.animation", test_animation("idle", 0.0)),
				("walk.animation", test_animation("walk", 2.0)),
				("run.animation", test_animation("run", 4.0)),
			]);
			let builder = AnimationGraph::builder();
			let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
			let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
			let run = builder.state("run").with(AnimationClip::looping("run.animation"));
			idle.to(walk).anytime(MediaTime::from_seconds(1));
			let onward = walk.to(run);
			if interruptible {
				onward.anytime(MediaTime::from_seconds(1));
			} else {
				onward.when(MediaTime::from_seconds(1));
			}
			let graph = builder.build(idle).expect("graph should build");
			let mut player = pool.create_player(&graph, None);
			let frame = MediaTime::from_millis(16);

			ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
			let mut shown = 0.0;
			for _ in 0..10 {
				shown = ready(player.advance(frame, walk.id(), &mut pool)).local_pose[0]
					.translation
					.x();
			}
			let redirected = ready(player.advance(frame, run.id(), &mut pool)).local_pose[0]
				.translation
				.x();

			if interruptible {
				assert_eq!(player.state(), Some(run.id()));
				// The pose keeps moving from where it was shown, at most a few frames' travel of the fastest clip.
				assert!(
					(redirected - shown).abs() < 4.0 * frame.as_seconds_f32() * 3.0,
					"the pose jumped from {shown} to {redirected}"
				);
			} else {
				assert_eq!(player.state(), Some(walk.id()));
			}
		}
	}

	#[test]
	fn redirected_blends_keep_root_motion_continuous() {
		let mut pool = pool_with(vec![
			("idle.animation", test_animation("idle", 0.0)),
			("walk.animation", test_animation("walk", 2.0)),
			("run.animation", test_animation("run", 4.0)),
		]);
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let run = builder.state("run").with(AnimationClip::looping("run.animation"));
		idle.to(walk).anytime(MediaTime::from_seconds(1));
		walk.to(run).anytime(MediaTime::from_seconds(1));
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("root")));
		let frame = MediaTime::from_millis(16);
		// No clip travels further than the run in one frame, so neither may the blend.
		let fastest = 4.0 * frame.as_seconds_f32() + 1.0e-4;

		ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
		let requests = std::iter::repeat_n(walk.id(), 10).chain(std::iter::repeat_n(run.id(), 5));
		for requested in requests {
			let travel = ready(player.advance(frame, requested, &mut pool)).root_motion.translation.x();
			assert!(travel.abs() <= fastest, "root motion jumped {travel} in one frame");
		}
		assert_eq!(player.state(), Some(run.id()));
	}

	#[test]
	fn rings_with_a_centre_sample_cross_the_middle_without_side_clips() {
		// The side clips also rise, so any share they get shows up in the pose's height.
		let mut pool = pool_with(vec![
			("still.animation", test_motion("still", [0.0, 0.0, 0.0], 1.0)),
			("forward.animation", test_motion("forward", [0.0, 0.0, 1.0], 1.0)),
			("right.animation", test_motion("right", [1.0, 1.0, 0.0], 1.0)),
			("backward.animation", test_motion("backward", [0.0, 0.0, -1.0], 1.0)),
			("left.animation", test_motion("left", [-1.0, 1.0, 0.0], 1.0)),
		]);
		let builder = AnimationGraph::builder();
		let walk = builder.state("walk").planar(
			AnimationBlend::new()
				.sample([0.0, 1.0], AnimationClip::looping("forward.animation"))
				.sample([1.0, 0.0], AnimationClip::looping("right.animation"))
				.sample([0.0, 0.0], AnimationClip::looping("still.animation"))
				.sample([0.0, -1.0], AnimationClip::looping("backward.animation"))
				.sample([-1.0, 0.0], AnimationClip::looping("left.animation")),
		);
		let graph = builder.build(walk).expect("graph should build");
		let mut player = pool.create_player(&graph, None);

		ready(player.advance(MediaTime::ZERO, walk.id().at([0.0, -0.5]), &mut pool));
		let pose = ready(player.advance(MediaTime::from_millis(500), walk.id().at([0.0, -0.5]), &mut pool));

		// Halfway back mixes the backward clip and the centre evenly, and no side clip.
		math::assert_float_eq!(pose.local_pose[0].translation.y(), 0.0);
		math::assert_float_eq!(pose.local_pose[0].translation.z(), -0.25);
	}

	#[test]
	fn inertialized_requests_blend_a_position_jump_and_plain_ones_switch_at_once() {
		for inertialized in [true, false] {
			let mut pool = pool_with(vec![
				("forward.animation", test_motion("forward", [0.0, 0.0, 1.0], 1.0)),
				("backward.animation", test_motion("backward", [0.0, 0.0, -1.0], 1.0)),
			]);
			let builder = AnimationGraph::builder();
			let walk = builder.state("walk").linear(
				AnimationBlend::new()
					.sample(-1.0, AnimationClip::looping("backward.animation"))
					.sample(1.0, AnimationClip::looping("forward.animation")),
			);
			let graph = builder.build(walk).expect("graph should build");
			let mut player = pool.create_player(&graph, None);
			let frame = MediaTime::from_millis(16);

			ready(player.advance(MediaTime::ZERO, walk.id().at(1.0), &mut pool));
			let mut shown = 0.0;
			for _ in 0..20 {
				shown = ready(player.advance(frame, walk.id().at(1.0), &mut pool)).local_pose[0]
					.translation
					.z();
			}
			let turnaround = if inertialized {
				walk.id().at(-1.0).inertialized(MediaTime::from_millis(200))
			} else {
				walk.id().at(-1.0)
			};
			let jumped = ready(player.advance(frame, turnaround, &mut pool)).local_pose[0]
				.translation
				.z();

			// Twenty frames into the forward clip sit at z = +0.32 and the backward clip at -0.34, a 0.66 jump. The
			// inertialized blend starts moving toward it, front-loaded, but covers well under half of it in a frame.
			if inertialized {
				assert!((jumped - shown).abs() < 0.2, "the pose jumped from {shown} to {jumped}");
			} else {
				assert!(jumped < -0.3, "a plain request should switch at once, got {jumped}");
			}
			let mut settled = 0.0;
			for _ in 0..15 {
				settled = ready(player.advance(frame, walk.id().at(-1.0), &mut pool)).local_pose[0]
					.translation
					.z();
			}
			// Once the blend is over, the pose is the backward clip's alone, which runs to -1 over the cycle.
			let expected = -(36.0 * frame.as_seconds_f32());
			assert!(
				(settled - expected).abs() < 1.0e-3,
				"settled at {settled}, expected {expected}"
			);
			assert_eq!(player.state(), Some(walk.id().into()));
		}
	}

	#[test]
	fn blend_states_reject_overlapping_positions_and_mixed_playback() {
		let builder = AnimationGraph::builder();
		let overlapping = builder.state("overlapping").linear(
			AnimationBlend::new()
				.sample(0.0, AnimationClip::looping("first.animation"))
				.sample(0.0, AnimationClip::looping("second.animation")),
		);
		assert!(matches!(
			builder.build(overlapping),
			Err(AnimationGraphBuildError::InvalidBlend { state: 0, .. })
		));

		let builder = AnimationGraph::builder();
		let mixed = builder.state("mixed").linear(
			AnimationBlend::new()
				.sample(0.0, AnimationClip::looping("loop.animation"))
				.sample(1.0, AnimationClip::once("once.animation")),
		);
		assert_eq!(
			builder.build(mixed).err(),
			Some(AnimationGraphBuildError::MixedBlendPlayback { state: 0 })
		);
	}

	#[test]
	fn player_interrupts_active_anytime_transition_states() {
		let idle_animation = test_animation("idle", 0.0);
		let start_animation = test_animation("start", 1.0);
		let stop_animation = test_animation("stop", 1.0);
		let walk_animation = test_animation("walk", 2.0);
		let byte_budget = packed_test_animation_bytes("idle", 0.0)
			+ packed_test_animation_bytes("start", 1.0)
			+ packed_test_animation_bytes("stop", 1.0)
			+ packed_test_animation_bytes("walk", 2.0);
		let mut pool = AnimationPool::detached(byte_budget);
		pool.admit("idle.animation".into(), idle_animation);
		pool.admit("start.animation".into(), start_animation);
		pool.admit("stop.animation".into(), stop_animation);
		pool.admit("walk.animation".into(), walk_animation);

		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let start_walk = idle
			.to(walk)
			.with(AnimationClip::once("start.animation"))
			.anytime(MediaTime::ZERO);
		let stop_walk = walk
			.to(idle)
			.with(AnimationClip::once("stop.animation"))
			.anytime(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, None);

		player
			.advance(MediaTime::ZERO, idle.id(), &mut pool)
			.expect("initial idle pose");
		player
			.advance(MediaTime::ZERO, walk.id(), &mut pool)
			.expect("starts the walk transition clip");

		assert_eq!(player.state(), Some(start_walk.id()));
		player
			.advance(MediaTime::ZERO, idle.id(), &mut pool)
			.expect("interrupts the walk transition clip");

		assert_eq!(player.state(), Some(stop_walk.id()));
		player
			.advance(MediaTime::from_seconds(1), idle.id(), &mut pool)
			.expect("finishes the stop transition clip");
		player
			.advance(MediaTime::ZERO, idle.id(), &mut pool)
			.expect("enters idle after the stop transition clip");

		assert_eq!(player.state(), Some(idle.id()));
	}

	#[test]
	fn player_waits_until_the_initial_clip_supplies_its_canonical_skeleton() {
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let graph = builder.build(idle).expect("graph should build");
		let animation = test_animation("idle", 0.0);
		let mut pool = AnimationPool::detached(animation.resident_bytes());
		let mut player = pool.create_player(&graph, None);

		assert!(matches!(player.advance(MediaTime::ZERO, idle.id(), &mut pool), Ok(None)));

		pool.admit("idle.animation".into(), animation);
		let pose = ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
		assert_eq!(pose.skeleton.nodes.len(), 1);
	}

	#[test]
	fn player_selectively_extracts_object_space_root_motion_across_a_remapped_scaled_loop() {
		let root_rotation =
			math::Orientation::try_from_rotation_vector(Vector::<ParentSpace>::new(0.0, std::f32::consts::FRAC_PI_2, 0.0))
				.unwrap();
		let root_rest = LocalTransform {
			rotation: root_rotation,
			scale: math::Scale::from_array([0.01; 3]),
			..LocalTransform::identity()
		};
		let hips_rest = LocalTransform {
			translation: math::Vector::from_array([0.0, 100.0, 0.0]),
			..LocalTransform::identity()
		};
		let source = Skeleton {
			nodes: vec![
				test_node(Some("Root"), None, root_rest),
				test_node(Some("Hips"), Some(0), hips_rest),
			],
		};
		// The target inserts helper nodes before Hips, matching FBX rigs whose
		// compatible named joints do not share source indices.
		let target = Skeleton {
			nodes: vec![
				test_node(None, None, LocalTransform::identity()),
				test_node(Some("Root"), Some(0), root_rest),
				test_node(Some("IK Helper"), Some(1), LocalTransform::identity()),
				test_node(Some("Hips"), Some(1), hips_rest),
			],
		};
		let animation = Animation {
			name: Some("walk".into()),
			skeleton: Reference::in_memory("scaled.skeleton", source),
			duration: 1.0,
			frame_rate: None,
			tracks: vec![NodeTrack {
				node: 1,
				translation: Some(TranslationCurve::Linear {
					times: vec![0.0, 1.0],
					values: vec![math::Vector::new(0.0, 100.0, 0.0), math::Vector::new(20.0, 110.0, -100.0)],
				}),
				rotation: Some(RotationCurve::Linear {
					times: vec![0.0, 1.0],
					values: vec![
						math::Orientation::try_from_array([0.0, 0.0, 0.0, 1.0]).unwrap(),
						math::Orientation::try_from_array([0.0, 0.382_683_43, 0.0, 0.923_879_5]).unwrap(),
					],
				}),
				scale: None,
			}],
		};
		let idle_animation = Animation {
			name: Some("idle".into()),
			skeleton: Reference::in_memory("canonical.skeleton", target),
			duration: 1.0,
			frame_rate: None,
			tracks: Vec::new(),
		};
		let byte_budget = idle_animation.estimated_resident_bytes() + animation.estimated_resident_bytes();
		let mut pool = AnimationPool::detached(byte_budget);
		pool.admit("idle.animation".into(), PackedAnimationData::from_resource(idle_animation));
		pool.admit("walk.animation".into(), PackedAnimationData::from_resource(animation));
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk).when(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(
			&graph,
			Some(RootMotionSettings {
				node_name: "Hips",
				translation: RootMotionTranslation::Z,
				rotation: RootMotionRotation::None,
			}),
		);

		let initial = ready(player.advance(MediaTime::ZERO, idle.id(), &mut pool));
		assert_eq!(initial.root_motion.translation, math::Vector::zero());
		let switched = ready(player.advance(MediaTime::ZERO, walk.id(), &mut pool));
		assert_eq!(switched.root_motion.translation, math::Vector::zero());
		let first = ready(player.advance(MediaTime::from_millis(750), walk.id(), &mut pool));
		math::assert_float_eq!(first.root_motion.translation.x(), -0.75);
		math::assert_float_eq!(first.root_motion.translation.y(), 0.0);
		math::assert_float_eq!(first.root_motion.translation.z(), 0.0);

		assert_eq!(first.local_pose[3].translation, math::Vector::new(15.0, 107.5, 0.0));
		assert_ne!(first.local_pose[3].rotation, LocalTransform::identity().rotation);

		let wrapped = ready(player.advance(MediaTime::from_millis(500), walk.id(), &mut pool));
		math::assert_float_eq!(wrapped.root_motion.translation.x(), -0.5);
		math::assert_float_eq!(wrapped.root_motion.translation.y(), 0.0);
		math::assert_float_eq!(wrapped.root_motion.translation.z(), 0.0);

		assert_eq!(wrapped.local_pose[3].translation, math::Vector::new(5.0, 102.5, 0.0));
		assert_ne!(wrapped.local_pose[3].rotation, LocalTransform::identity().rotation);
	}
}
