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
//! through [`LinearStateId::at`] and [`PlanarStateId::at`]. Connect state
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
	resource_id: Box<str>,
	playback: AnimationPlayback,
}

impl AnimationClip {
	/// Creates a clip that restarts from its first sample after reaching its duration.
	pub fn looping(resource_id: impl Into<Box<str>>) -> Self {
		Self {
			resource_id: resource_id.into(),
			playback: AnimationPlayback::Loop,
		}
	}

	/// Creates a clip that holds its final sample after reaching its duration.
	pub fn once(resource_id: impl Into<Box<str>>) -> Self {
		Self {
			resource_id: resource_id.into(),
			playback: AnimationPlayback::Once,
		}
	}

	/// Returns the resource ID requested by an [`AnimationPool`].
	pub fn resource_id(&self) -> &str {
		&self.resource_id
	}

	/// Returns the playback behavior used after the clip reaches its duration.
	pub const fn playback(&self) -> AnimationPlayback {
		self.playback
	}
}

struct StateTransition {
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
	/// Where in its clips the target starts; `None` starts it at its beginning.
	entry: Option<ClipPoint>,
}

/// The `SelectedRoute` struct is a route the player takes: where to, how long to blend, when, and where to start.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SelectedRoute<'graph> {
	target: AnimationStateId,
	duration: MediaTime,
	wait: &'graph [ClipPoint],
	entry: Option<ClipPoint>,
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
	/// Set only for one-shot transition states: the state entered after the clip finishes.
	completion: Option<AnimationStateId>,
	/// How long a finished one-shot inertializes into its completion state; zero cuts straight to it.
	completion_blend: MediaTime,
	/// Where in its clips the completion state starts after the one-shot.
	completion_entry: Option<ClipPoint>,
	transitions: Vec<StateTransition>,
}

impl AnimationGraphStateData {
	/// Selects the first authored route whose persistent destination was requested.
	///
	/// When `interrupting` an active transition clip, only `.anytime` routes qualify.
	fn select_route(&self, requested: AnimationStateId, interrupting: bool) -> Option<SelectedRoute<'_>> {
		self.transitions
			.iter()
			.find(|transition| (!interrupting || transition.can_interrupt_transition) && transition.destination == requested)
			.map(|transition| SelectedRoute {
				target: transition.target,
				duration: transition.duration,
				wait: &transition.wait,
				entry: transition.entry,
			})
	}
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
	/// Targets every state can route to after its own routes, with how long each blends in.
	any_state_routes: Box<[(AnimationStateId, MediaTime)]>,
	initial: AnimationStateId,
	/// Clip data resolved once against the canonical skeleton, which every player of the graph shares.
	binding: runtime::GraphBinding,
}

impl AnimationGraph {
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
				transitions: Vec::new(),
				any_state_routes: Vec::new(),
				invalid_blends: Vec::new(),
				invalid_points: Vec::new(),
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

	fn contains(&self, id: AnimationStateId) -> bool {
		id.graph == self.id && id.index < self.states.len()
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
				source_state
					.select_route(requested, false)
					.or_else(|| self.select_any_state_route(requested))
			};
		};

		if let Some(transition) = source_state.select_route(requested, false) {
			return Some(transition);
		}
		if requested != completion {
			if let Some(transition) = self.state(completion).select_route(requested, true) {
				// Its wait points name moments of the completion state, which is not playing yet.
				return Some(SelectedRoute { wait: &[], ..transition });
			}
			if let Some(transition) = self.select_any_state_route(requested) {
				return Some(transition);
			}
		}

		source_finished.then_some(SelectedRoute {
			target: completion,
			duration: source_state.completion_blend,
			wait: &[],
			entry: source_state.completion_entry,
		})
	}

	/// Selects an `.anytime` route that cuts short a blend into `destination` toward the requested state.
	///
	/// A one-shot destination is left to the routes [`Self::select_transition`] finds once it plays on its own.
	fn select_interruption(
		&self,
		destination: AnimationStateId,
		requested: AnimationStateId,
	) -> Option<SelectedRoute<'_>> {
		let state = self.state(destination);
		if destination == requested || state.completion.is_some() {
			return None;
		}
		state
			.select_route(requested, true)
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
				entry: None,
			})
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
	/// Routes beside their source state, which [`AnimationGraphBuilder::build`] validates before attaching them.
	transitions: Vec<(AnimationStateId, StateTransition)>,
	/// Targets every state can route to, added with [`AnimationGraphBuilder::any_state_to`].
	any_state_routes: Vec<(AnimationStateId, MediaTime)>,
	/// Blend states whose samples were rejected, reported by [`AnimationGraphBuilder::build`].
	invalid_blends: Vec<(usize, BlendSpaceError)>,
	/// Route points that named a clip their state does not play, reported by [`AnimationGraphBuilder::build`].
	invalid_points: Vec<AnimationGraphBuildError>,
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
				self.invalid_points
					.push(AnimationGraphBuildError::RoutePointNeedsClip { state: state.index });
				return None;
			}
		};
		if index.is_none() {
			self.invalid_points.push(AnimationGraphBuildError::RoutePointClipMissing {
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
		completion: Option<AnimationStateId>,
		completion_blend: MediaTime,
	) -> AnimationStateId {
		self.states.push(AnimationGraphStateData {
			name,
			motion,
			completion,
			completion_blend,
			completion_entry: None,
			transitions: Vec::new(),
		});
		AnimationStateId {
			graph,
			index: self.states.len() - 1,
		}
	}
}

/// The `AnimationGraphStateBuilder` struct assigns a clip to a named graph state.
pub struct AnimationGraphStateBuilder<'builder> {
	graph: u64,
	data: &'builder RefCell<Option<AnimationGraphBuilderData>>,
	name: String,
}

/// The `AnimationGraphState` struct connects a state to other states during graph authoring.
#[derive(Clone, Copy)]
pub struct AnimationGraphState<'builder> {
	data: &'builder RefCell<Option<AnimationGraphBuilderData>>,
	id: AnimationStateId,
}

/// The `AnimationGraphTransitionBuilder` struct assigns a clip to a route between two states.
pub struct AnimationGraphTransitionBuilder<'builder> {
	source: AnimationGraphState<'builder>,
	target: AnimationGraphState<'builder>,
	wait: Vec<ClipPoint>,
	entry: Option<ClipPoint>,
}

/// The `AnimationGraphTransitionConditionBuilder` struct holds unfinished one-shot route authoring so callers can choose its interruption policy.
pub struct AnimationGraphTransitionConditionBuilder<'builder> {
	source: AnimationGraphState<'builder>,
	target: AnimationGraphState<'builder>,
	clip: AnimationClip,
	completion_blend: MediaTime,
	wait: Vec<ClipPoint>,
	entry: Option<ClipPoint>,
}

impl<'builder> AnimationGraphStateBuilder<'builder> {
	/// Assigns the clip played while this state is active.
	pub fn with(self, clip: AnimationClip) -> AnimationGraphState<'builder> {
		let mut builder = self.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		let id = builder.push_state(self.graph, self.name, StateMotion::Clip(clip), None, MediaTime::ZERO);
		AnimationGraphState { data: self.data, id }
	}

	/// Plays a blend of clips placed on a line. Request it with a position through [`LinearStateId::at`].
	pub fn linear(self, blend: AnimationBlend<f32>) -> LinearGraphState<'builder> {
		LinearGraphState(self.with_blend(blend.into_motion()))
	}

	/// Plays a blend of clips placed on a plane, like locomotion directions. Request it with a position through
	/// [`PlanarStateId::at`].
	pub fn planar(self, blend: AnimationBlend<[f32; 2]>) -> PlanarGraphState<'builder> {
		PlanarGraphState(self.with_blend(blend.into_motion()))
	}

	/// Adds a blend state, recording a rejected blend for [`AnimationGraphBuilder::build`] to report.
	fn with_blend(self, motion: Result<StateMotion, BlendSpaceError>) -> AnimationGraphState<'builder> {
		let mut builder = self.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		let motion = motion.unwrap_or_else(|error| {
			builder.invalid_blends.push((builder.states.len(), error));
			// A placeholder keeps state IDs stable until the build fails.
			StateMotion::Clip(AnimationClip::looping("invalid-blend"))
		});
		let id = builder.push_state(self.graph, self.name, motion, None, MediaTime::ZERO);
		AnimationGraphState { data: self.data, id }
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
			source: self,
			target,
			wait: Vec::new(),
			entry: None,
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
	pub fn entering_at_frame(mut self, clip: &str, frame: u32) -> Self {
		self.entry = self.target.point(Some(clip), ClipTime::Frame(frame));
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
				target: self.target.id,
				destination: self.target.id,
				duration: inertialization,
				can_interrupt_transition,
				wait: self.wait.into(),
				entry: self.entry,
			},
		));
		self.target
	}

	/// Assigns the one-shot clip played between the source and target states.
	pub fn with(self, clip: AnimationClip) -> AnimationGraphTransitionConditionBuilder<'builder> {
		AnimationGraphTransitionConditionBuilder {
			source: self.source,
			target: self.target,
			clip,
			completion_blend: MediaTime::ZERO,
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

	/// Adds a one-shot route that must complete before reconciling another request.
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
		let name = format!(
			"{} -> {} [{}]",
			builder.states[self.source.id.index].name,
			builder.states[self.target.id.index].name,
			builder.states.len(),
		);
		let id = builder.push_state(
			self.source.id.graph,
			name,
			StateMotion::Clip(self.clip),
			Some(self.target.id),
			self.completion_blend,
		);
		builder.states[id.index].completion_entry = self.entry;
		builder.transitions.push((
			self.source.id,
			StateTransition {
				target: id,
				destination: self.target.id,
				duration: inertialization,
				can_interrupt_transition,
				wait: self.wait.into(),
				entry: None,
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
		}
	}

	/// Adds a route from every state to `target`, taken when the playing state has no route of its own to it. Use it for
	/// a state any moment can lead to, like a death, instead of a route from each state.
	///
	/// Like an [`AnimationGraphTransitionBuilder::anytime`] route, it cuts short a one-shot clip or a blend in
	/// progress, inertializing over `inertialization` from the pose on screen.
	pub fn any_state_to<'builder>(&'builder self, target: impl Into<AnimationGraphState<'builder>>, inertialization: MediaTime) {
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
		if let Some(&(state, error)) = data.invalid_blends.first() {
			return Err(AnimationGraphBuildError::InvalidBlend { state, error });
		}
		if let Some(error) = data.invalid_points.first() {
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
			binding: runtime::GraphBinding::new(data.states.len()),
			states: data.states,
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
	/// A blend state cannot keep its clips in step because some loop and others play once.
	MixedBlendPlayback {
		/// The index of the blend state.
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
				"Animation blend state mixes looping and one-shot clips. The most likely cause is state {state} using `AnimationClip::once` for some samples and `AnimationClip::looping` for others."
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
pub use motion::{AnimationBlend, AnimationRequest, LinearGraphState, LinearStateId, PlanarGraphState, PlanarStateId};
use motion::{BlendPositions, BlendValue, ClipPoint, ClipTime, StateMotion};
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
	AnimationPoolEvent, AnimationPoolRequest, RootMotionRotation, RootMotionSettings, RootMotionTranslation,
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
			graph.select_transition(start_walk.id, idle.id, false).map(SelectedRoute::pair),
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
		let route = |source: AnimationStateId, finished| graph.select_transition(source, dead.id(), finished).map(SelectedRoute::pair);

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
