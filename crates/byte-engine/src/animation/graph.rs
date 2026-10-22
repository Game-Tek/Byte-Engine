//! Code-authored skeletal animation state machines and asynchronous clip pooling.
//!
//! Build an [`AnimationGraph`] with [`AnimationGraphBuilder`], then create an
//! [`AnimationGraphPlayer`] for each animated skeleton. The player evaluates
//! synchronously from retained pose buffers while [`AnimationPool`] loads clip
//! resources on an application-owned async worker. Client code selects one
//! durable [`AnimationStateId`] with normal Rust control flow, then the player
//! reconciles its current playback toward that requested state. A state plays
//! one clip, or blends several placed on a line or a plane: build those with
//! [`AnimationGraphStateBuilder::linear`] and
//! [`AnimationGraphStateBuilder::planar`], and request them with a position
//! through [`LinearStateId::at`] and [`PlanarStateId::at`]. Group states that
//! share one position, like a walk and its starts and stops, into a space with
//! [`AnimationGraphBuilder::planar_space`]. Connect state
//! handles with [`AnimationGraphState::to`], then use
//! [`AnimationGraphTransitionConditionBuilder::when`] or
//! [`AnimationGraphTransitionConditionBuilder::anytime`] to choose whether a
//! one-shot transition clip can be interrupted.
//!
//! # Connection order
//!
//! 1. During application setup, build the immutable graph and create one
//!    [`AnimationPool`] with the application's [`ResourceManager`].
//! 2. Spawn the [`AnimationLoadWorker`] returned by [`AnimationPool::new`] on
//!    the same async runtime that serves resource requests. The pool only
//!    enqueues work, so clips remain in `Loading` until this worker runs.
//! 3. Create one [`AnimationGraphPlayer`] per animated instance with
//!    [`AnimationPool::create_player`]. The pool loads the initial clip, whose
//!    skeleton becomes the canonical target for every graph clip. As the player
//!    enters each state, it also requests clips in directly reachable states.
//! 4. Each application tick, call [`AnimationPool::update`] once, select an
//!    authored [`AnimationStateId`] (or an [`AnimationRequest`] that places a
//!    blend state), call [`AnimationGraphPlayer::advance`],
//!    apply its root motion to the owning object, and send
//!    [`AnimationGraphPose::global_pose`] to rendering. For many characters,
//!    set each player's request with [`AnimationGraphPlayer::request`] and
//!    advance them together with [`AnimationPool::advance_players`], which
//!    spreads them over the alley's lanes; then read each one's
//!    [`AnimationGraphPlayer::pose`].
//!
//! Players share everything that isn't their own playback: the graph, the
//! clips' bindings to the canonical skeleton, the pool's clips, and the poses a
//! frame fills and reads, which belong to the thread that evaluates. The
//! renderer's `UpdatePose` message owns its transform vector, so copying at
//! that cross-system boundary is currently intentional. See
//! `crates/byte-engine/examples/animation_graph.rs` for the complete headed
//! application sequence.

/// Bounds asynchronous animation load requests independently from the clip byte budget.
pub const ANIMATION_LOAD_QUEUE_CAPACITY: usize = 64;

/// Bounds retained load outcomes so diagnostics cannot outgrow the clip pool.
pub const ANIMATION_POOL_EVENT_CAPACITY: usize = ANIMATION_LOAD_QUEUE_CAPACITY;

/// The `AnimationStateId` struct identifies one client-requestable state inside its originating [`AnimationGraph`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AnimationStateId {
	graph: u64,
	index: usize,
}

/// The `AnimationPlayback` enum selects whether a state clip repeats or stops at its final pose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationPlayback {
	/// Restarts the clip from its first sample after it reaches its duration.
	Loop,
	/// Holds the clip's final sample after it reaches its duration.
	Once,
}

/// The `AnimationClip` struct identifies the resource and playback behavior used by one state.
///
/// [`AnimationPool`] keys clips by their resource ID, so a clip keeps a stable identity across arena residency and
/// eviction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnimationClip {
	/// Shared, so the players that list the clip as missing hold it without copying the ID.
	resource_id: Arc<str>,
	playback: AnimationPlayback,
}

impl AnimationClip {
	/// Creates a clip that restarts from its first sample after reaching its duration.
	pub fn looping(resource_id: impl Into<Arc<str>>) -> Self {
		Self {
			resource_id: resource_id.into(),
			playback: AnimationPlayback::Loop,
		}
	}

	/// Creates a clip that holds its final sample after reaching its duration.
	pub fn once(resource_id: impl Into<Arc<str>>) -> Self {
		Self {
			resource_id: resource_id.into(),
			playback: AnimationPlayback::Once,
		}
	}

	/// Returns the resource ID requested by an [`AnimationPool`].
	pub fn resource_id(&self) -> &str {
		&self.resource_id
	}

	/// Returns the resource ID as the graph shares it, for players that keep it past an evaluation.
	pub(crate) fn shared_resource_id(&self) -> &Arc<str> {
		&self.resource_id
	}

	/// Returns the playback behavior used after the clip reaches its duration.
	pub const fn playback(&self) -> AnimationPlayback {
		self.playback
	}
}

struct StateTransition {
	/// Competes with other matching routes by wait time; every fitted, contributing source clip must have frames.
	matching_frames: bool,
	// `target` may be an internal one-shot state, while `destination` is the
	// persistent state requested by client code.
	target: AnimationStateId,
	destination: AnimationStateId,
	/// The critically damped inertialization duration. [`MediaTime::ZERO`] switches immediately.
	duration: MediaTime,
	// Only `.anytime` routes cut short a playing one-shot clip or a blend in progress.
	can_interrupt_transition: bool,
	/// Moments of the source's playback the route waits for before it starts; empty starts it at once.
	wait: Box<[ClipPoint]>,
	/// Where in its clips the target starts; empty starts it at its beginning.
	entry: Box<[ClipPoint]>,
}

/// The `SelectedRoute` struct is a route the player takes: where to, how long to blend, when, and where to start.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SelectedRoute<'graph> {
	target: AnimationStateId,
	duration: MediaTime,
	wait: &'graph [ClipPoint],
	entry: &'graph [ClipPoint],
	/// Where the graph keeps `wait` and `entry`, for a player that keeps the route past this selection.
	wait_points: RoutePoints,
	entry_points: RoutePoints,
	origin: RouteOrigin,
}

/// The `RoutePoints` enum names a selected route's wait or entry points by where the graph keeps them, so a player
/// that waits for a route keeps it small and without borrowing the graph.
///
/// Resolve it with [`AnimationGraph::points`]. A route with no points of a kind names [`Self::None`].
#[derive(Clone, Copy, Debug, PartialEq)]
enum RoutePoints {
	None,
	/// The wait points of route `route` among the routes of the state or space `origin` names.
	Wait {
		origin: RouteOrigin,
		route: u32,
	},
	/// The entry points of route `route` among the routes of the state or space `origin` names.
	Entry {
		origin: RouteOrigin,
		route: u32,
	},
	/// The points the completion of the one-shot state at this index starts its completion state at.
	CompletionEntry(u32),
}

/// The `RouteOrigin` enum names where a blend or pending route came from, so a player's data stream can say why it
/// took it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RouteOrigin {
	/// A route of the state at this index.
	State(usize),
	/// A route of the space at this index.
	Space(usize),
	AnyState,
	/// The completion of the one-shot state at this index.
	Completion(usize),
	/// A position change of the playing blend, which takes no route.
	Position,
	/// The start of the initial state.
	Initial,
}

impl SelectedRoute<'_> {
	#[cfg(test)]
	fn pair(self) -> (AnimationStateId, MediaTime) {
		(self.target, self.duration)
	}
}

struct AnimationGraphStateData {
	name: String,
	motion: StateMotion,
	/// The space whose position the state's blend weighs its clips at, and whose routes the state takes after its own.
	space: Option<usize>,
	/// Set only for one-shot transition states: the state entered after the clip finishes.
	completion: Option<AnimationStateId>,
	/// Whether a request for another state cuts a playing one-shot short along its completion state's `.anytime`
	/// routes. A one-shot that must complete plays to its end; only any-state routes cut it short.
	interruptible: bool,
	/// How long a finished one-shot inertializes into its completion state; zero cuts straight to it.
	completion_blend: MediaTime,
	/// Where in its clips the completion state starts after the one-shot.
	completion_entry: Box<[ClipPoint]>,
	/// The angle about +Y, in radians, the entity takes over through root motion when the one-shot hands over to its
	/// completion state; zero for none.
	completion_turn: f32,
	transitions: Vec<StateTransition>,
}

/// The `AnimationSpaceData` struct is a group of states that share one blend position, and the routes every one of
/// them can take.
struct AnimationSpaceData {
	name: String,
	kind: SpaceKind,
	/// Routes every state of the space takes after its own, added with [`AnimationGraphSpace::any_state_to`].
	transitions: Vec<StateTransition>,
}

/// Selects the first of `transitions` whose persistent destination was requested.
///
/// When `interrupting` an active transition clip, only `.anytime` routes qualify.
fn select_route(
	transitions: &[StateTransition],
	origin: RouteOrigin,
	requested: AnimationStateId,
	interrupting: bool,
) -> Option<SelectedRoute<'_>> {
	transitions
		.iter()
		.enumerate()
		.find(|(_, transition)| (!interrupting || transition.can_interrupt_transition) && transition.destination == requested)
		.map(|(index, transition)| {
			let route = u32::try_from(index).expect("a state has fewer routes than `u32::MAX`");
			SelectedRoute {
				target: transition.target,
				duration: transition.duration,
				wait: &transition.wait,
				entry: &transition.entry,
				wait_points: if transition.wait.is_empty() {
					RoutePoints::None
				} else {
					RoutePoints::Wait { origin, route }
				},
				entry_points: if transition.entry.is_empty() {
					RoutePoints::None
				} else {
					RoutePoints::Entry { origin, route }
				},
				origin,
			}
		})
}

/// The `AnimationGraph` struct stores immutable routes used to reconcile requested animation states.
///
/// Build the graph once with [`AnimationGraphBuilder`], then share it between
/// any number of [`AnimationGraphPlayer`] instances. Client code selects one of
/// the graph's durable [`AnimationStateId`] values before each evaluation.
///
/// The graph also keeps its clips' pose maps, durations, and frame rates once
/// players have resolved them against its canonical skeleton, so players that
/// share a graph share that work too.
pub struct AnimationGraph {
	id: u64,
	states: Vec<AnimationGraphStateData>,
	spaces: Box<[AnimationSpaceData]>,
	/// Targets every state can route to after its own routes, with how long each blends in.
	any_state_routes: Box<[(AnimationStateId, MediaTime)]>,
	initial: AnimationStateId,
	/// How many round-robin states the graph has, which sizes each player's [`RoundRobinTurns`].
	round_robins: usize,
	/// Clip data resolved once against the canonical skeleton, which every player of the graph shares.
	binding: runtime::GraphBinding,
}

impl AnimationGraph {
	/// Returns the number that tells this graph from every other, which its state IDs and the animators that play it
	/// carry.
	pub const fn id(&self) -> u64 {
		self.id
	}

	/// Starts a builder for an animation reconciliation graph.
	pub fn builder() -> AnimationGraphBuilder {
		use std::hash::BuildHasher as _;

		// Each `RandomState` gets fresh keys from the standard library, so every
		// builder gets a distinct 64-bit identity without an engine-owned counter.
		// The identity only guards against mixing state IDs from different graphs.
		let graph = std::hash::RandomState::new().hash_one(0u64);
		AnimationGraphBuilder {
			graph,
			data: RefCell::new(Some(AnimationGraphBuilderData {
				states: Vec::new(),
				spaces: Vec::new(),
				transitions: Vec::new(),
				any_state_routes: Vec::new(),
				round_robins: 0,
				errors: Vec::new(),
			})),
		}
	}

	/// Returns the graph's initial state.
	pub const fn initial_state(&self) -> AnimationStateId {
		self.initial
	}

	/// Returns the number of authored states.
	pub fn state_count(&self) -> usize {
		self.states.len()
	}

	fn state(&self, id: AnimationStateId) -> &AnimationGraphStateData {
		debug_assert_eq!(id.graph, self.id, "animation state must belong to this graph");
		// Builder validation keeps every runtime state ID in range.
		&self.states[id.index]
	}

	/// Returns the routes of the space `id` belongs to, which it takes after its own; empty outside every space.
	fn space_routes(&self, id: AnimationStateId) -> &[StateTransition] {
		self.state(id).space.map_or(&[], |space| &self.spaces[space].transitions)
	}

	/// Selects the first route toward `requested` among `id`'s own routes, then its space's.
	fn select_state_route(
		&self,
		id: AnimationStateId,
		requested: AnimationStateId,
		interrupting: bool,
	) -> Option<SelectedRoute<'_>> {
		let state = self.state(id);
		select_route(&state.transitions, RouteOrigin::State(id.index), requested, interrupting).or_else(|| {
			let space = state.space?;
			select_route(
				&self.spaces[space].transitions,
				RouteOrigin::Space(space),
				requested,
				interrupting,
			)
		})
	}

	/// Selects a route toward the requested persistent state or completes a one-shot state.
	fn select_transition(
		&self,
		source: AnimationStateId,
		requested: AnimationStateId,
		source_finished: bool,
	) -> Option<SelectedRoute<'_>> {
		let source_state = self.state(source);
		let Some(completion) = source_state.completion else {
			return if source == requested {
				None
			} else {
				self.select_state_route(source, requested, false)
					.or_else(|| self.select_any_state_route(requested))
			};
		};

		// A one-shot's space only gives its blend a position: it takes its own routes, then, if it may be interrupted,
		// its completion's `.anytime` routes. Any-state routes cut either kind short.
		if let Some(transition) = select_route(&source_state.transitions, RouteOrigin::State(source.index), requested, false) {
			return Some(transition);
		}
		if requested != completion {
			if source_state.interruptible
				&& let Some(transition) = self.select_state_route(completion, requested, true)
			{
				// Its wait points name moments of the completion state, which is not playing yet.
				return Some(SelectedRoute {
					wait: &[],
					wait_points: RoutePoints::None,
					..transition
				});
			}
			if let Some(transition) = self.select_any_state_route(requested) {
				return Some(transition);
			}
		}

		source_finished.then_some(SelectedRoute {
			target: completion,
			duration: source_state.completion_blend,
			wait: &[],
			entry: &source_state.completion_entry,
			wait_points: RoutePoints::None,
			entry_points: if source_state.completion_entry.is_empty() {
				RoutePoints::None
			} else {
				RoutePoints::CompletionEntry(u32::try_from(source.index).expect("a graph has fewer states than `u32::MAX`"))
			},
			origin: RouteOrigin::Completion(source.index),
		})
	}

	/// Selects an `.anytime` route that cuts short a blend into `destination` toward the requested state.
	///
	/// A one-shot destination is left to the routes [`Self::select_transition`] finds once it plays on its own.
	fn select_interruption(&self, destination: AnimationStateId, requested: AnimationStateId) -> Option<SelectedRoute<'_>> {
		let state = self.state(destination);
		if destination == requested || state.completion.is_some() {
			return None;
		}
		self.select_state_route(destination, requested, true)
			.or_else(|| self.select_any_state_route(requested))
	}

	/// Selects a route every state can take toward the requested state, added with
	/// [`AnimationGraphBuilder::any_state_to`].
	fn select_any_state_route(&self, requested: AnimationStateId) -> Option<SelectedRoute<'_>> {
		self.any_state_routes
			.iter()
			.find(|&&(target, _)| target == requested)
			.map(|&(target, duration)| SelectedRoute {
				target,
				duration,
				wait: &[],
				entry: &[],
				wait_points: RoutePoints::None,
				entry_points: RoutePoints::None,
				origin: RouteOrigin::AnyState,
			})
	}

	/// Returns the points `points` names, which a player kept from a route it selected.
	fn points(&self, points: RoutePoints) -> &[ClipPoint] {
		let routes = |origin| match origin {
			RouteOrigin::State(index) => &self.states[index].transitions,
			RouteOrigin::Space(index) => &self.spaces[index].transitions,
			_ => unreachable!("only state and space routes name points"),
		};
		match points {
			RoutePoints::None => &[],
			RoutePoints::Wait { origin, route } => &routes(origin)[route as usize].wait,
			RoutePoints::Entry { origin, route } => &routes(origin)[route as usize].entry,
			RoutePoints::CompletionEntry(state) => &self.states[state as usize].completion_entry,
		}
	}
}

/// The `AnimationGraphBuilder` struct assembles named clip states and their ordered reconciliation routes.
///
/// Start one with [`AnimationGraph::builder`], then name states with [`Self::state`].
pub struct AnimationGraphBuilder {
	graph: u64,
	data: RefCell<Option<AnimationGraphBuilderData>>,
}

struct AnimationGraphBuilderData {
	states: Vec<AnimationGraphStateData>,
	spaces: Vec<AnimationSpaceData>,
	/// Routes beside their source state, which [`AnimationGraphBuilder::build`] validates before attaching them.
	transitions: Vec<(AnimationStateId, StateTransition)>,
	/// Targets every state can route to, added with [`AnimationGraphBuilder::any_state_to`].
	any_state_routes: Vec<(AnimationStateId, MediaTime)>,
	/// How many round-robin states were built, which numbers their slots.
	round_robins: usize,
	/// Problems found while authoring, like a rejected blend or a route point naming a clip its state does not play,
	/// which [`AnimationGraphBuilder::build`] reports, the first one first.
	errors: Vec<AnimationGraphBuildError>,
}

impl AnimationGraphBuilderData {
	/// Resolves a moment of `state`'s clip named `clip`, or of its only clip when `clip` is `None`, recording an error
	/// for [`AnimationGraphBuilder::build`] when the state has no such clip.
	fn clip_point(&mut self, state: AnimationStateId, clip: Option<&str>, time: ClipTime) -> Option<ClipPoint> {
		let clips = self.states[state.index].motion.clips();
		let index = match clip {
			Some(clip) => clips.iter().position(|candidate| candidate.resource_id() == clip),
			None if clips.len() == 1 => Some(0),
			None => {
				self.errors
					.push(AnimationGraphBuildError::RoutePointNeedsClip { state: state.index });
				return None;
			}
		};
		if index.is_none() {
			self.errors.push(AnimationGraphBuildError::RoutePointClipMissing {
				state: state.index,
				clip: clip.unwrap_or_default().to_string(),
			});
		}
		index.map(|clip| ClipPoint { clip, time })
	}
}

impl AnimationGraphBuilderData {
	/// Appends a state with no routes and returns its ID in `graph`.
	fn push_state(
		&mut self,
		graph: u64,
		name: String,
		motion: StateMotion,
		space: Option<usize>,
		completion: Option<AnimationStateId>,
		completion_blend: MediaTime,
	) -> AnimationStateId {
		self.states.push(AnimationGraphStateData {
			name,
			motion,
			space,
			completion,
			interruptible: true,
			completion_blend,
			completion_entry: Box::default(),
			completion_turn: 0.0,
			transitions: Vec::new(),
		});
		AnimationStateId {
			graph,
			index: self.states.len() - 1,
		}
	}

	/// Appends a space with no routes and returns its index.
	fn push_space(&mut self, name: String, kind: SpaceKind) -> usize {
		self.spaces.push(AnimationSpaceData {
			name,
			kind,
			transitions: Vec::new(),
		});
		self.spaces.len() - 1
	}

	/// Unwraps an authored motion, recording a rejected blend for [`AnimationGraphBuilder::build`] against the state
	/// about to be pushed.
	fn motion(&mut self, motion: AnimationMotion) -> StateMotion {
		motion.0.unwrap_or_else(|error| {
			self.errors.push(AnimationGraphBuildError::InvalidBlend {
				state: self.states.len(),
				error,
			});
			// A placeholder keeps state IDs stable until the build fails.
			StateMotion::Clip(AnimationClip::looping("invalid-blend"))
		})
	}
}

/// The `AnimationGraphStateBuilder` struct assigns a clip to a named graph state.
pub struct AnimationGraphStateBuilder<'builder> {
	graph: u64,
	data: &'builder RefCell<Option<AnimationGraphBuilderData>>,
	name: String,
	/// The space the state joins, set when it was named through [`AnimationGraphSpace::state`].
	space: Option<usize>,
}

/// The `AnimationGraphSpace` struct groups states that share one blend position, like the idle, walk, starts, and
/// stops of a character moving in one direction.
///
/// Create one with [`AnimationGraphBuilder::planar_space`] or [`AnimationGraphBuilder::linear_space`], then add its
/// states with [`Self::state`]. Requesting any blend state of the space with a position, like
/// [`PlanarStateId::at`], moves the position for every blend of the space, and a one-shot blend on a route into the
/// space, like a stop, plays at the position the space had when it starts. Add routes every state of the space takes
/// with [`Self::any_state_to`].
#[derive(Clone, Copy)]
pub struct AnimationGraphSpace<'builder> {
	graph: u64,
	data: &'builder RefCell<Option<AnimationGraphBuilderData>>,
	index: usize,
}

/// The `AnimationGraphState` struct connects a state to other states during graph authoring.
#[derive(Clone, Copy)]
pub struct AnimationGraphState<'builder> {
	data: &'builder RefCell<Option<AnimationGraphBuilderData>>,
	id: AnimationStateId,
}

/// The `AnimationGraphTransitionBuilder` struct assigns a clip to a route between two states.
pub struct AnimationGraphTransitionBuilder<'builder> {
	matching_frames: bool,
	source: AnimationGraphState<'builder>,
	target: AnimationGraphState<'builder>,
	wait: Vec<ClipPoint>,
	entry: Vec<ClipPoint>,
}

/// The `AnimationGraphTransitionConditionBuilder` struct holds unfinished one-shot route authoring so callers can choose its interruption policy.
pub struct AnimationGraphTransitionConditionBuilder<'builder> {
	matching_frames: bool,
	source: AnimationGraphState<'builder>,
	target: AnimationGraphState<'builder>,
	motion: AnimationMotion,
	completion_blend: MediaTime,
	turn: f32,
	wait: Vec<ClipPoint>,
	entry: Vec<ClipPoint>,
}

impl<'builder> AnimationGraphStateBuilder<'builder> {
	/// Assigns the clip played while this state is active.
	pub fn with(self, clip: AnimationClip) -> AnimationGraphState<'builder> {
		let mut builder = self.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		let id = builder.push_state(
			self.graph,
			self.name,
			StateMotion::Clip(clip),
			self.space,
			None,
			MediaTime::ZERO,
		);
		AnimationGraphState { data: self.data, id }
	}

	/// Plays one of `clips` at a time, the next one each time a player enters the state, starting with the first and
	/// wrapping after the last. Use it to vary a state that plays again and again, like a death or a hit reaction,
	/// without picking the clip in application code.
	///
	/// Every clip must loop, or every one must play once. Each player keeps its own turn. Request the state like a clip
	/// state, with [`AnimationGraphState::id`], and connect it with [`AnimationGraphState::to`] or
	/// [`AnimationGraphBuilder::any_state_to`].
	pub fn round_robin(self, clips: impl IntoIterator<Item = AnimationClip>) -> AnimationGraphState<'builder> {
		let mut builder = self.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		// Slots number the round robins in the order they were built.
		let slot = builder.round_robins;
		builder.round_robins += 1;
		let motion = StateMotion::RoundRobin {
			clips: clips.into_iter().collect(),
			slot,
		};
		let id = builder.push_state(self.graph, self.name, motion, self.space, None, MediaTime::ZERO);
		AnimationGraphState { data: self.data, id }
	}

	/// Plays a blend of clips placed on a line. Request it with a position through [`LinearStateId::at`].
	///
	/// Named through [`AnimationGraphSpace::state`], it weighs its clips at the space's position, which must be linear.
	/// Otherwise it forms a space of its own, named after the state.
	pub fn linear(self, blend: AnimationBlend<f32>) -> LinearGraphState<'builder> {
		LinearGraphState(self.with_blend(blend.into()))
	}

	/// Plays a blend of clips placed on a plane, like locomotion directions. Request it with a position through
	/// [`PlanarStateId::at`].
	///
	/// Named through [`AnimationGraphSpace::state`], it weighs its clips at the space's position, which must be planar.
	/// Otherwise it forms a space of its own, named after the state.
	pub fn planar(self, blend: AnimationBlend<[f32; 2]>) -> PlanarGraphState<'builder> {
		PlanarGraphState(self.with_blend(blend.into()))
	}

	/// Adds a blend state to its space, or to a new space of its own, recording a rejected blend or a blend of the
	/// wrong kind for [`AnimationGraphBuilder::build`] to report.
	fn with_blend(self, motion: AnimationMotion) -> AnimationGraphState<'builder> {
		let mut builder = self.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		let motion = builder.motion(motion);
		// Only a rejected blend, which fails the build anyway, has no kind.
		let kind = motion.kind().unwrap_or(SpaceKind::Linear);
		let space = match self.space {
			Some(space) => {
				if builder.spaces[space].kind != kind {
					builder.errors.push(AnimationGraphBuildError::SpaceKindMismatch {
						state: builder.states.len(),
					});
				}
				space
			}
			None => builder.push_space(self.name.clone(), kind),
		};
		let id = builder.push_state(self.graph, self.name, motion, Some(space), None, MediaTime::ZERO);
		AnimationGraphState { data: self.data, id }
	}
}

impl<'builder> AnimationGraphSpace<'builder> {
	/// Names a state of this space. Call [`AnimationGraphStateBuilder::with`] next for a clip, or
	/// [`AnimationGraphStateBuilder::planar`] or [`AnimationGraphStateBuilder::linear`] for a blend of the space's kind.
	pub fn state(self, name: impl Into<String>) -> AnimationGraphStateBuilder<'builder> {
		AnimationGraphStateBuilder {
			graph: self.graph,
			data: self.data,
			name: name.into(),
			space: Some(self.index),
		}
	}

	/// Adds a route from every state of this space to `target`, taken when the playing state has no route of its own
	/// to it. Use it for a state the whole space leads to the same way, like settling back into the idle.
	///
	/// Like [`AnimationGraphBuilder::any_state_to`], it cuts short a one-shot clip leading into the space or a blend in
	/// progress, inertializing over `inertialization` from the pose on screen. The graph's any-state routes come after
	/// it.
	pub fn any_state_to(self, target: impl Into<AnimationGraphState<'builder>>, inertialization: MediaTime) {
		let target = target.into();
		assert!(
			std::ptr::eq(self.data, target.data),
			"the target animation state must use this space's builder"
		);
		let mut data = self.data.borrow_mut();
		let data = data.as_mut().expect("the animation graph has already been built");
		data.spaces[self.index].transitions.push(StateTransition {
			matching_frames: false,
			target: target.id,
			destination: target.id,
			duration: inertialization,
			can_interrupt_transition: true,
			wait: Box::default(),
			entry: Box::default(),
		});
	}
}

impl<'builder> AnimationGraphState<'builder> {
	/// Returns the durable state ID that client code can request after the graph is built.
	pub const fn id(self) -> AnimationStateId {
		self.id
	}

	/// Starts an authored route that reconciles toward `target`, which may be any kind of state.
	pub fn to(self, target: impl Into<Self>) -> AnimationGraphTransitionBuilder<'builder> {
		let target = target.into();
		assert!(
			std::ptr::eq(self.data, target.data),
			"animation graph states must use the same builder"
		);
		AnimationGraphTransitionBuilder {
			matching_frames: false,
			source: self,
			target,
			wait: Vec::new(),
			entry: Vec::new(),
		}
	}

	/// Resolves a moment of this state's clip `clip`, or of its only clip, for a route point.
	fn point(self, clip: Option<&str>, time: ClipTime) -> Option<ClipPoint> {
		let mut builder = self.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		builder.clip_point(self.id, clip, time)
	}
}

impl<'builder> AnimationGraphTransitionBuilder<'builder> {
	/// Makes this route one of several fitted alternatives to the same destination, so a single request, such as
	/// idle, picks the stop that fits the playing blend soonest, like a stop off either foot.
	///
	/// An alternative is eligible only when it names frames on every contributing source clip that any alternative
	/// names frames on; clips no alternative names don't constrain the choice. The eligible alternative whose frame
	/// comes first wins, and equal times keep authoring order. While the source blends in, the route waits for it to
	/// settle instead of taking a route of its space.
	///
	/// Next, name the clips' frames with [`Self::at_frames`], then assign the stop with [`Self::with`].
	pub fn matching_frames(mut self) -> Self {
		self.matching_frames = true;
		self
	}

	/// Waits for the source's playback to reach `frame` of its clip before taking the route. Use it to start a clip
	/// at the point of a loop it fits, like a jump off the right foot.
	///
	/// Frames count from the clip's first frame at its authored frame rate. The source must play one clip; for a blend,
	/// use [`Self::at_frames`]. A route that plays a one-shot through [`Self::with`] waits before playing it. Requesting
	/// another state while the route waits cancels it.
	pub fn at_frame(mut self, frame: u32) -> Self {
		self.wait.extend(self.source.point(None, ClipTime::Frame(frame)));
		self
	}

	/// Waits for the source's playback to reach any of `frames` of its clip `clip`, the resource ID of one of the
	/// source's clips. For a blend, its clips share one phase, so the frames of one clip place the moments for all.
	///
	/// Call it for several clips of a blend to fit each, like the walk loops a stop blend's clips were measured
	/// against: the route waits for the frames of the named clip that weighs the most, the first named among equals.
	pub fn at_frames(mut self, clip: &str, frames: impl IntoIterator<Item = u32>) -> Self {
		for frame in frames {
			self.wait.extend(self.source.point(Some(clip), ClipTime::Frame(frame)));
		}
		self
	}

	/// Waits for the source's playback to reach `time` into its clip, for clips without an authored frame rate. The
	/// source must play one clip.
	pub fn at_time(mut self, time: MediaTime) -> Self {
		self.wait
			.extend(self.source.point(None, ClipTime::Seconds(time.as_seconds_f32())));
		self
	}

	/// Starts the target at `frame` of its clip `clip` instead of at its beginning. Use it to pick up a loop at the
	/// point that fits the pose the route leaves from. On a route that plays a one-shot through [`Self::with`], the
	/// target starts there once the one-shot hands over, like landing into a walk mid-stride.
	///
	/// Call it for several clips of a blend to fit each: the named clip that weighs the most where the target starts
	/// picks the frame, the first named among equals.
	pub fn entering_at_frame(mut self, clip: &str, frame: u32) -> Self {
		self.entry.extend(self.target.point(Some(clip), ClipTime::Frame(frame)));
		self
	}

	/// Adds a direct route without playing an intermediate clip, taken only once the source has settled.
	///
	/// `inertialization` smooths the switch with critically damped inertialization for that duration.
	/// Pass [`MediaTime::ZERO`] to switch immediately. Use [`Self::anytime`] for a route that may also cut short a
	/// blend into the source or a one-shot clip that leads to it.
	pub fn when(self, inertialization: MediaTime) -> AnimationGraphState<'builder> {
		self.add(inertialization, false)
	}

	/// Adds a direct route that is also taken while the source is still being blended in, or while a one-shot clip
	/// that leads to the source plays, cutting either short from the pose on screen.
	///
	/// `inertialization` works as in [`Self::when`]. Responsive locomotion uses these routes so a change of input
	/// never waits for a blend or a start clip to finish.
	pub fn anytime(self, inertialization: MediaTime) -> AnimationGraphState<'builder> {
		self.add(inertialization, true)
	}

	/// Adds the direct route with the selected interruption policy.
	fn add(self, inertialization: MediaTime, can_interrupt_transition: bool) -> AnimationGraphState<'builder> {
		let mut builder = self.source.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		builder.transitions.push((
			self.source.id,
			StateTransition {
				matching_frames: self.matching_frames,
				target: self.target.id,
				destination: self.target.id,
				duration: inertialization,
				can_interrupt_transition,
				wait: self.wait.into(),
				entry: self.entry.into(),
			},
		));
		self.target
	}

	/// Assigns the one-shot played between the source and target states: an [`AnimationClip`], or an
	/// [`AnimationBlend`] of clips that play once.
	///
	/// A blend joins the target's space, or the source's when the target's space is of the other kind, and weighs its
	/// clips at that space's position and follows the position while it plays, like a loop. Use it for a start or
	/// a stop that follows the walk's direction; keep requesting it without a position to hold the direction it set
	/// off in.
	pub fn with(self, motion: impl Into<AnimationMotion>) -> AnimationGraphTransitionConditionBuilder<'builder> {
		AnimationGraphTransitionConditionBuilder {
			matching_frames: self.matching_frames,
			source: self.source,
			target: self.target,
			motion: motion.into(),
			completion_blend: MediaTime::ZERO,
			turn: 0.0,
			wait: self.wait,
			entry: self.entry,
		}
	}
}

impl<'builder> AnimationGraphTransitionConditionBuilder<'builder> {
	/// Inertializes from the one-shot clip's last pose into the target state over `inertialization` once the clip
	/// ends, which hides small pose mismatches between the two clips. Without it the target starts immediately.
	pub fn exit(mut self, inertialization: MediaTime) -> Self {
		self.completion_blend = inertialization;
		self
	}

	/// Hands `yaw`, in radians about +Y, to the entity through root motion when the one-shot completes. Use it for a
	/// turn in place: pass the angle the clip turns the body in its pose, and make the target a state whose pose
	/// faces where the clip started, so the entity takes over the turn in the frame the pose gives it up. The look
	/// chain holds its direction across it on its own. A one-shot cut short hands over no turn.
	pub fn turning(mut self, yaw: f32) -> Self {
		self.turn = yaw;
		self
	}

	/// Adds a one-shot route that must complete before reconciling another request. Only an any-state route, added
	/// with [`AnimationGraphBuilder::any_state_to`], cuts its clip short.
	///
	/// `inertialization` works as in [`AnimationGraphTransitionBuilder::when`].
	pub fn when(self, inertialization: MediaTime) -> AnimationGraphState<'builder> {
		self.add(inertialization, false)
	}

	/// Adds a one-shot route that can reconcile another request before it completes.
	///
	/// `inertialization` works as in [`AnimationGraphTransitionBuilder::when`].
	pub fn anytime(self, inertialization: MediaTime) -> AnimationGraphState<'builder> {
		self.add(inertialization, true)
	}

	/// Adds the internal one-shot state and its source route with the selected interruption policy.
	fn add(self, inertialization: MediaTime, can_interrupt_transition: bool) -> AnimationGraphState<'builder> {
		let mut builder = self.source.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		let (source, target) = (
			&builder.states[self.source.id.index].name,
			&builder.states[self.target.id.index].name,
		);
		// The second and later one-shots between the same pair take a number, so every name stays unique.
		let earlier = builder
			.transitions
			.iter()
			.filter(|(from, route)| {
				*from == self.source.id && route.destination == self.target.id && route.target != route.destination
			})
			.count();
		let name = match earlier {
			0 => format!("{source} -> {target}"),
			earlier => format!("{source} -> {target} {}", earlier + 1),
		};
		let motion = builder.motion(self.motion);
		// A blend takes its position from the target's space, or the source's when only that one is of its kind.
		let space = motion.kind().and_then(|kind| {
			let space = [self.target.id, self.source.id]
				.into_iter()
				.filter_map(|state| builder.states[state.index].space)
				.find(|&space| builder.spaces[space].kind == kind);
			if space.is_none() {
				builder.errors.push(AnimationGraphBuildError::BlendOutsideSpace {
					state: builder.states.len(),
				});
			}
			space
		});
		let id = builder.push_state(
			self.source.id.graph,
			name,
			motion,
			space,
			Some(self.target.id),
			self.completion_blend,
		);
		builder.states[id.index].completion_entry = self.entry.into();
		builder.states[id.index].completion_turn = self.turn;
		builder.states[id.index].interruptible = can_interrupt_transition;
		builder.transitions.push((
			self.source.id,
			StateTransition {
				matching_frames: self.matching_frames,
				target: id,
				destination: self.target.id,
				duration: inertialization,
				can_interrupt_transition,
				wait: self.wait.into(),
				entry: Box::default(),
			},
		));
		AnimationGraphState {
			data: self.source.data,
			id,
		}
	}
}

impl AnimationGraphBuilder {
	/// Names a state. Call [`AnimationGraphStateBuilder::with`] next to assign its clip.
	pub fn state(&self, name: impl Into<String>) -> AnimationGraphStateBuilder<'_> {
		AnimationGraphStateBuilder {
			graph: self.graph,
			data: &self.data,
			name: name.into(),
			space: None,
		}
	}

	/// Starts a space whose states share one position on a plane, like the direction and speed a character moves at.
	/// Add its states with [`AnimationGraphSpace::state`].
	pub fn planar_space(&self, name: impl Into<String>) -> AnimationGraphSpace<'_> {
		self.space(name.into(), SpaceKind::Planar)
	}

	/// Starts a space whose states share one position on a line, like a speed. Add its states with
	/// [`AnimationGraphSpace::state`].
	pub fn linear_space(&self, name: impl Into<String>) -> AnimationGraphSpace<'_> {
		self.space(name.into(), SpaceKind::Linear)
	}

	fn space(&self, name: String, kind: SpaceKind) -> AnimationGraphSpace<'_> {
		let mut data = self.data.borrow_mut();
		let data = data.as_mut().expect("the animation graph has already been built");
		AnimationGraphSpace {
			graph: self.graph,
			data: &self.data,
			index: data.push_space(name, kind),
		}
	}

	/// Adds a route from every state to `target`, taken when the playing state has no route of its own to it. Use it for
	/// a state any moment can lead to, like a death, instead of a route from each state.
	///
	/// Like an [`AnimationGraphTransitionBuilder::anytime`] route, it cuts short a one-shot clip or a blend in
	/// progress, inertializing over `inertialization` from the pose on screen.
	pub fn any_state_to<'builder>(
		&'builder self,
		target: impl Into<AnimationGraphState<'builder>>,
		inertialization: MediaTime,
	) {
		let target = target.into();
		assert!(
			std::ptr::eq(&self.data, target.data),
			"the target animation state must use this builder"
		);
		let mut data = self.data.borrow_mut();
		let data = data.as_mut().expect("the animation graph has already been built");
		data.any_state_routes.push((target.id, inertialization));
	}

	/// Adds an internal one-shot state that falls through to `completion` after it finishes.
	/// Tests use it to author states that [`Self::build`] must reject.
	#[cfg(test)]
	fn transition_state(&self, name: impl Into<String>, clip: AnimationClip, completion: AnimationStateId) -> AnimationStateId {
		let mut data = self.data.borrow_mut();
		let data = data.as_mut().expect("the animation graph has already been built");
		data.push_state(
			self.graph,
			name.into(),
			StateMotion::Clip(clip),
			None,
			Some(completion),
			MediaTime::ZERO,
		)
	}

	/// Validates the graph and selects the initial state.
	pub fn build<'builder>(
		&'builder self,
		initial: impl Into<AnimationGraphState<'builder>>,
	) -> Result<AnimationGraph, AnimationGraphBuildError> {
		let initial = initial.into();
		assert!(
			std::ptr::eq(&self.data, initial.data),
			"the initial animation state must use this builder"
		);
		let mut data = self
			.data
			.borrow_mut()
			.take()
			.expect("the animation graph has already been built");
		let initial = initial.id;
		if let Some(error) = data.errors.first() {
			return Err(error.clone());
		}
		if initial.index >= data.states.len() {
			return Err(AnimationGraphBuildError::InitialStateOutOfRange {
				state: initial.index,
				state_count: data.states.len(),
			});
		}

		for (state_index, state) in data.states.iter().enumerate() {
			if state.name.trim().is_empty() {
				return Err(AnimationGraphBuildError::EmptyStateName { state: state_index });
			}
			// Checked before playback, which reads a state's first clip.
			if state.motion.clips().is_empty() {
				return Err(AnimationGraphBuildError::EmptyRoundRobin { state: state_index });
			}
			if state.motion.clips().iter().any(|clip| clip.resource_id().trim().is_empty()) {
				return Err(AnimationGraphBuildError::EmptyResourceId { state: state_index });
			}
			if state
				.motion
				.clips()
				.iter()
				.any(|clip| clip.playback() != state.motion.playback())
			{
				return Err(AnimationGraphBuildError::MixedBlendPlayback { state: state_index });
			}
			if data.states[..state_index].iter().any(|other| other.name == state.name) {
				return Err(AnimationGraphBuildError::DuplicateStateName {
					name: state.name.clone(),
				});
			}
			if let Some(completion) = state.completion {
				if state.motion.playback() != AnimationPlayback::Once {
					return Err(AnimationGraphBuildError::TransitionStateMustPlayOnce { state: state_index });
				}
				if completion.graph != self.graph || completion.index >= data.states.len() {
					return Err(AnimationGraphBuildError::TransitionStateCompletionOutOfRange {
						state: state_index,
						completion: completion.index,
						state_count: data.states.len(),
					});
				}
			}
		}

		for (space_index, space) in data.spaces.iter().enumerate() {
			if space.name.trim().is_empty() {
				return Err(AnimationGraphBuildError::EmptySpaceName { space: space_index });
			}
			if data.spaces[..space_index].iter().any(|other| other.name == space.name) {
				return Err(AnimationGraphBuildError::DuplicateSpaceName {
					name: space.name.clone(),
				});
			}
			if space
				.transitions
				.iter()
				.any(|transition| transition.duration < MediaTime::ZERO)
			{
				return Err(AnimationGraphBuildError::NegativeTransitionDuration);
			}
		}

		for (source, transition) in data.transitions {
			if source.graph != self.graph || source.index >= data.states.len() {
				return Err(AnimationGraphBuildError::TransitionSourceOutOfRange {
					state: source.index,
					state_count: data.states.len(),
				});
			}
			if transition.target.graph != self.graph || transition.target.index >= data.states.len() {
				return Err(AnimationGraphBuildError::TransitionTargetOutOfRange {
					state: transition.target.index,
					state_count: data.states.len(),
				});
			}
			if transition.duration < MediaTime::ZERO {
				return Err(AnimationGraphBuildError::NegativeTransitionDuration);
			}
			data.states[source.index].transitions.push(transition);
		}
		if data.any_state_routes.iter().any(|&(_, duration)| duration < MediaTime::ZERO) {
			return Err(AnimationGraphBuildError::NegativeTransitionDuration);
		}

		Ok(AnimationGraph {
			id: self.graph,
			round_robins: data.round_robins,
			binding: runtime::GraphBinding::new(data.states.len()),
			states: data.states,
			spaces: data.spaces.into(),
			any_state_routes: data.any_state_routes.into(),
			initial,
		})
	}
}

/// The `AnimationGraphBuildError` enum reports invalid graph authoring input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnimationGraphBuildError {
	/// The selected initial state is outside the graph being built.
	InitialStateOutOfRange {
		/// The invalid initial state index.
		state: usize,
		/// The number of states available in the graph.
		state_count: usize,
	},
	/// A state cannot be identified because its name is empty.
	EmptyStateName {
		/// The index of the unnamed state.
		state: usize,
	},
	/// A state cannot load its clip because its resource ID is empty.
	EmptyResourceId {
		/// The index of the state with no resource ID.
		state: usize,
	},
	/// A state name cannot identify one state because another state uses it.
	DuplicateStateName {
		/// The name shared by multiple states.
		name: String,
	},
	/// A space cannot be identified because its name is empty.
	EmptySpaceName {
		/// The index of the unnamed space, in the order spaces were created.
		space: usize,
	},
	/// A space name cannot identify one space because another space uses it.
	DuplicateSpaceName {
		/// The name shared by multiple spaces.
		name: String,
	},
	/// A blend state cannot weigh its clips at its space's position because the two are of different kinds.
	SpaceKindMismatch {
		/// The index of the blend state.
		state: usize,
	},
	/// A one-shot blend has no position to weigh its clips at because neither end of its route is in a space of its
	/// kind.
	BlendOutsideSpace {
		/// The index of the one-shot blend's state.
		state: usize,
	},
	/// A transition cannot start because its source is outside the graph.
	TransitionSourceOutOfRange {
		/// The invalid source state index.
		state: usize,
		/// The number of states available in the graph.
		state_count: usize,
	},
	/// A transition cannot complete because its target is outside the graph.
	TransitionTargetOutOfRange {
		/// The invalid target state index.
		state: usize,
		/// The number of states available in the graph.
		state_count: usize,
	},
	/// A transition state cannot complete because its clip uses looping playback.
	TransitionStateMustPlayOnce {
		/// The index of the transition state with looping playback.
		state: usize,
	},
	/// A transition state cannot complete because its completion state is outside the graph.
	TransitionStateCompletionOutOfRange {
		/// The index of the transition state.
		state: usize,
		/// The invalid completion state index.
		completion: usize,
		/// The number of states available in the graph.
		state_count: usize,
	},
	/// A transition cannot be built with a duration before [`MediaTime::ZERO`].
	NegativeTransitionDuration,
	/// A blend state cannot weigh its clips because their positions are invalid.
	InvalidBlend {
		/// The index of the blend state.
		state: usize,
		/// Why the positions were rejected.
		error: BlendSpaceError,
	},
	/// A blend or round-robin state cannot keep its clips in step because some loop and others play once.
	MixedBlendPlayback {
		/// The index of the blend or round-robin state.
		state: usize,
	},
	/// A round-robin state has no clip to play.
	EmptyRoundRobin {
		/// The index of the round-robin state.
		state: usize,
	},
	/// A route point names a clip that its state does not play.
	RoutePointClipMissing {
		/// The index of the state the point was looked up in: the route's source for a wait, its target for an entry.
		state: usize,
		/// The clip resource ID the point named.
		clip: String,
	},
	/// A route point does not say which clip it counts frames of, but its state plays several.
	RoutePointNeedsClip {
		/// The index of the blend state.
		state: usize,
	},
}

impl fmt::Display for AnimationGraphBuildError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::InitialStateOutOfRange { state, state_count } => write!(
				formatter,
				"Animation graph initial state is missing. The most likely cause is selecting state {state} in a graph with {state_count} states."
			),
			Self::EmptyStateName { state } => write!(
				formatter,
				"Animation graph state has no name. The most likely cause is state {state} being created with an empty label."
			),
			Self::EmptyResourceId { state } => write!(
				formatter,
				"Animation graph state has no resource ID. The most likely cause is state {state} being created without an animation asset."
			),
			Self::DuplicateStateName { name } => write!(
				formatter,
				"Animation graph state name is duplicated. The most likely cause is multiple states named '{name}'."
			),
			Self::EmptySpaceName { space } => write!(
				formatter,
				"Animation graph space has no name. The most likely cause is space {space} being created with an empty label."
			),
			Self::DuplicateSpaceName { name } => write!(
				formatter,
				"Animation graph space name is duplicated. The most likely cause is multiple spaces named '{name}', or a blend state built outside a space, which forms a space named after itself, sharing a space's name."
			),
			Self::SpaceKindMismatch { state } => write!(
				formatter,
				"Animation blend state is of a different kind than its space. The most likely cause is state {state} being a linear blend in a planar space, or a planar blend in a linear one."
			),
			Self::BlendOutsideSpace { state } => write!(
				formatter,
				"Animation one-shot blend has no space to take its position from. The most likely cause is the route of state {state} joining two states that belong to no space of the blend's kind."
			),
			Self::TransitionSourceOutOfRange { state, state_count } => write!(
				formatter,
				"Animation transition source is missing. The most likely cause is selecting state {state} in a graph with {state_count} states."
			),
			Self::TransitionTargetOutOfRange { state, state_count } => write!(
				formatter,
				"Animation transition target is missing. The most likely cause is selecting state {state} in a graph with {state_count} states."
			),
			Self::TransitionStateMustPlayOnce { state } => write!(
				formatter,
				"Animation transition state does not play once. The most likely cause is state {state} using a looping clip."
			),
			Self::TransitionStateCompletionOutOfRange {
				state,
				completion,
				state_count,
			} => write!(
				formatter,
				"Animation transition-state completion is missing. The most likely cause is state {state} selecting completion state {completion} in a graph with {state_count} states."
			),
			Self::NegativeTransitionDuration => write!(
				formatter,
				"Animation transition duration is negative. The most likely cause is using a timeline offset instead of a non-negative duration."
			),
			Self::InvalidBlend { state, error } => write!(
				formatter,
				"Animation blend state cannot weigh its clips ({error:?}). The most likely cause is state {state} placing two clips at the same position, or a planar blend whose clips do not form a ring and needs explicit triangles."
			),
			Self::RoutePointClipMissing { state, clip } => write!(
				formatter,
				"Animation route point names a clip its state does not play. The most likely cause is state {state} not having '{clip}' among its clips."
			),
			Self::RoutePointNeedsClip { state } => write!(
				formatter,
				"Animation route point does not say which clip it counts frames of. The most likely cause is calling `at_frame` or `at_time` on a route from blend state {state}; use `at_frames` with one of its clips."
			),
			Self::MixedBlendPlayback { state } => write!(
				formatter,
				"Animation blend or round-robin state mixes looping and one-shot clips. The most likely cause is state {state} using `AnimationClip::once` for some clips and `AnimationClip::looping` for others."
			),
			Self::EmptyRoundRobin { state } => write!(
				formatter,
				"Animation round-robin state has no clips. The most likely cause is state {state} being built with `round_robin` from an empty list."
			),
		}
	}
}

impl std::error::Error for AnimationGraphBuildError {}

mod motion;
mod runtime;

use std::{
	cell::RefCell,
	collections::VecDeque,
	fmt,
	num::NonZeroUsize,
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU64, Ordering},
	},
};

use math::{AffineMatrix, Vector};
pub use motion::{
	AnimationBlend, AnimationMotion, AnimationRequest, LinearGraphState, LinearStateId, PlanarGraphState, PlanarStateId,
};
use motion::{BlendPositions, BlendValue, ClipPoint, ClipTime, RoundRobinTurns, SpaceKind, StateMotion};
use resource_management::{
	resource::resource_manager::ResourceManager,
	resources::{
		ParentSpace,
		animation::Animation,
		skeleton::{LocalTransform, Skeleton},
	},
};
#[doc(hidden)]
pub use runtime::benchmarks;
pub use runtime::{
	AnimationGraphPlayer, AnimationGraphPlayerError, AnimationGraphPose, AnimationLoadWorker, AnimationPool,
	AnimationPoolEvent, AnimationPoolRequest, LookChainSettings, LookTarget, RootMotionRotation, RootMotionSettings,
	RootMotionTranslation,
};

use super::{
	blend::{BlendSpaceError, BlendWeights, blend_local_poses},
	inertialization::PoseInertializer,
	packed::{PackedAnimation, PackedAnimationData, TrackTargets},
	root_motion::RootMotionDelta,
	skeletal::write_global_pose,
};
use crate::{
	MediaTime,
	core::{EntityHandle, async_runtime},
};

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn builder_preserves_transition_order_and_rejects_invalid_graphs() {
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk).when(MediaTime::ZERO);
		idle.to(idle)
			.with(AnimationClip::once("restart.animation"))
			.when(MediaTime::from_millis(100));
		let graph = builder.build(idle).expect("expected graph value");

		assert_eq!(graph.state_count(), 3);
		assert_eq!(graph.state(idle.id).transitions.len(), 2);
		assert_eq!(
			graph.select_transition(idle.id(), idle.id(), false),
			None,
			"requesting the active persistent state must not restart a self-route"
		);

		let invalid = AnimationGraph::builder();
		let state = invalid.state("").with(AnimationClip::once("clip.animation"));

		assert!(matches!(
			invalid.build(state),
			Err(AnimationGraphBuildError::EmptyStateName { state: 0 })
		));
	}

	#[test]
	fn anytime_transition_states_interrupt_toward_their_completion_state() {
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
		let graph = builder.build(idle).expect("transition state should be valid");

		assert_eq!(
			graph
				.select_transition(start_walk.id, idle.id, false)
				.map(SelectedRoute::pair),
			Some((stop_walk.id, MediaTime::ZERO)),
			"a matching any-time transition must interrupt an active transition clip"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, walk.id, false),
			None,
			"the active any-time transition must not restart itself"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, walk.id, true).map(SelectedRoute::pair),
			Some((walk.id, MediaTime::ZERO)),
			"an uninterrupted transition state must fall through to its completion"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, idle.id, true).map(SelectedRoute::pair),
			Some((stop_walk.id, MediaTime::ZERO)),
			"a matching any-time transition must take priority over completion"
		);

		let looping_state = AnimationGraph::builder();
		let idle = looping_state.state("idle").with(AnimationClip::looping("idle.animation"));
		looping_state.transition_state("invalid", AnimationClip::looping("invalid.animation"), idle.id);

		assert!(matches!(
			looping_state.build(idle),
			Err(AnimationGraphBuildError::TransitionStateMustPlayOnce { state: 1 })
		));

		let missing_completion = AnimationGraph::builder();
		let idle = missing_completion
			.state("idle")
			.with(AnimationClip::looping("idle.animation"));
		missing_completion.transition_state(
			"invalid",
			AnimationClip::once("invalid.animation"),
			super::AnimationStateId {
				graph: idle.id.graph,
				index: 2,
			},
		);

		assert!(matches!(
			missing_completion.build(idle),
			Err(AnimationGraphBuildError::TransitionStateCompletionOutOfRange {
				state: 1,
				completion: 2,
				state_count: 2,
			})
		));
	}

	#[test]
	fn any_state_routes_lead_from_every_state_after_its_own_routes() {
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let dead = builder.state("dead").with(AnimationClip::once("death.animation"));
		let start_walk = idle
			.to(walk)
			.with(AnimationClip::once("start.animation"))
			.anytime(MediaTime::ZERO);
		walk.to(dead).when(MediaTime::from_millis(100));
		builder.any_state_to(dead, MediaTime::from_millis(300));
		let graph = builder.build(idle).expect("graph should build");
		let route =
			|source: AnimationStateId, finished| graph.select_transition(source, dead.id(), finished).map(SelectedRoute::pair);

		assert_eq!(route(idle.id(), false), Some((dead.id(), MediaTime::from_millis(300))));
		assert_eq!(
			route(start_walk.id(), false),
			Some((dead.id(), MediaTime::from_millis(300))),
			"an any-state route cuts short a one-shot clip"
		);
		assert_eq!(
			route(walk.id(), false),
			Some((dead.id(), MediaTime::from_millis(100))),
			"a state's own route comes first"
		);
		assert_eq!(route(dead.id(), false), None, "the target does not route to itself");
	}

	#[test]
	fn space_routes_lead_from_every_state_of_the_space_between_its_own_and_any_state_routes() {
		let builder = AnimationGraph::builder();
		let standing = builder.planar_space("standing");
		let idle = standing.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = standing.state("walk").with(AnimationClip::looping("walk.animation"));
		let crouch = builder.state("crouch").with(AnimationClip::looping("crouch.animation"));
		let dead = builder.state("dead").with(AnimationClip::once("death.animation"));
		let start_walk = idle
			.to(walk)
			.with(AnimationClip::once("start.animation"))
			.anytime(MediaTime::ZERO);
		walk.to(dead).when(MediaTime::from_millis(100));
		standing.any_state_to(dead, MediaTime::from_millis(200));
		builder.any_state_to(dead, MediaTime::from_millis(300));
		let graph = builder.build(idle).expect("graph should build");
		let route = |source: AnimationStateId| graph.select_transition(source, dead.id(), false).map(SelectedRoute::pair);

		assert_eq!(route(idle.id()), Some((dead.id(), MediaTime::from_millis(200))));
		assert_eq!(
			route(walk.id()),
			Some((dead.id(), MediaTime::from_millis(100))),
			"a state's own route comes first"
		);
		assert_eq!(
			route(start_walk.id()),
			Some((dead.id(), MediaTime::from_millis(200))),
			"the space of a one-shot's completion cuts the one-shot short"
		);
		assert_eq!(
			route(crouch.id()),
			Some((dead.id(), MediaTime::from_millis(300))),
			"a state outside the space takes the graph's any-state route"
		);
	}

	#[test]
	fn spaces_reject_blends_without_a_position_of_their_kind() {
		let line = || {
			AnimationBlend::new()
				.sample(0.0, AnimationClip::looping("slow.animation"))
				.sample(1.0, AnimationClip::looping("fast.animation"))
		};

		let builder = AnimationGraph::builder();
		let standing = builder.planar_space("standing");
		let idle = standing.state("idle").with(AnimationClip::looping("idle.animation"));
		standing.state("speed").linear(line());
		assert!(matches!(
			builder.build(idle),
			Err(AnimationGraphBuildError::SpaceKindMismatch { state: 1 })
		));

		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk)
			.with(
				AnimationBlend::new()
					.sample(0.0, AnimationClip::once("start-slow.animation"))
					.sample(1.0, AnimationClip::once("start-fast.animation")),
			)
			.anytime(MediaTime::ZERO);
		assert!(matches!(
			builder.build(idle),
			Err(AnimationGraphBuildError::BlendOutsideSpace { state: 2 })
		));

		let builder = AnimationGraph::builder();
		let speed = builder.state("speed").linear(line());
		builder.linear_space("speed");
		assert!(matches!(
			builder.build(speed),
			Err(AnimationGraphBuildError::DuplicateSpaceName { name }) if name == "speed"
		));
	}

	#[test]
	fn route_points_must_name_a_clip_of_their_state() {
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk).at_frames("walk.animation", [3]).anytime(MediaTime::ZERO);

		assert!(matches!(
			builder.build(idle),
			Err(AnimationGraphBuildError::RoutePointClipMissing { state: 0, clip }) if clip == "walk.animation"
		));

		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let blend = builder.state("move").linear(
			AnimationBlend::new()
				.sample(0.0, AnimationClip::looping("walk.animation"))
				.sample(1.0, AnimationClip::looping("run.animation")),
		);
		blend.to(idle).at_frame(3).anytime(MediaTime::ZERO);

		assert!(matches!(
			builder.build(idle),
			Err(AnimationGraphBuildError::RoutePointNeedsClip { state: 1 })
		));
	}

	#[test]
	fn transition_states_prioritize_authored_exits_over_completion() {
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let start_walk = idle
			.to(walk)
			.with(AnimationClip::once("start.animation"))
			.when(MediaTime::ZERO);
		start_walk.to(idle).when(MediaTime::ZERO);
		let graph = builder.build(idle).expect("transition state should be valid");

		assert_eq!(
			graph.select_transition(start_walk.id, idle.id, true).map(SelectedRoute::pair),
			Some((idle.id, MediaTime::ZERO)),
			"an authored exit must take priority over transition-state completion"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, walk.id, true).map(SelectedRoute::pair),
			Some((walk.id, MediaTime::ZERO)),
			"a transition state without a matching exit must complete normally"
		);
	}
}
