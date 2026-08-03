//! Code-authored skeletal animation state machines and asynchronous clip pooling.
//!
//! Build an [`AnimationGraph`] with [`AnimationGraphBuilder`], then create an
//! [`AnimationGraphPlayer`] for each animated skeleton. The player evaluates
//! synchronously from retained pose buffers while [`AnimationPool`] loads clip
//! resources on an application-owned async worker.
//!
//! # Connection order
//!
//! 1. During application setup, build the immutable graph and create one
//!    [`AnimationPool`] with the application's [`ResourceManager`].
//! 2. Spawn the [`AnimationLoadWorker`] returned by [`AnimationPool::new`] on
//!    the same async runtime that serves resource requests. The pool only
//!    enqueues work, so clips remain in `Loading` until this worker runs.
//! 3. Load or otherwise obtain the target mesh skeleton, then create one
//!    [`AnimationGraphPlayer`] per animated instance. Use
//!    [`AnimationGraphPlayer::new_owned`] when the app transfers ownership of
//!    a loaded skeleton into the player.
//! 4. Each application tick, create typed input, call
//!    [`AnimationGraphPlayer::advance`], apply its root motion to the owning
//!    object, and send [`AnimationGraphPose::global_pose`] to rendering.
//!
//! The graph player owns its evaluation buffers. The existing
//! [`crate::rendering::UpdatePose`] message owns its matrix vector, so copying
//! at that cross-system boundary is currently intentional. See
//! `crates/byte-engine/examples/animation_graph.rs` for the complete headed
//! application sequence.

use std::{collections::VecDeque, fmt, num::NonZeroUsize, ops::Deref, sync::Arc};

use math::Matrix;
use resource_management::{
	resource::resource_manager::ResourceManager,
	resources::{
		animation::Animation,
		skeleton::{LocalTransform, Skeleton, SkeletonPoseMap},
	},
};

use super::{
	inertialization::PoseInertializer,
	root_motion::RootMotionDelta,
	skeletal::{sample_local_pose, write_global_pose},
};
use crate::{
	core::{async_runtime, EntityHandle},
	MediaTime,
};

/// Bounds asynchronous animation load requests independently from the clip byte budget.
pub const ANIMATION_LOAD_QUEUE_CAPACITY: usize = 64;

/// Bounds retained load outcomes so diagnostics cannot outgrow the clip pool.
pub const ANIMATION_POOL_EVENT_CAPACITY: usize = ANIMATION_LOAD_QUEUE_CAPACITY;

/// The `AnimationStateId` struct identifies one state inside an [`AnimationGraph`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AnimationStateId(usize);

/// The `AnimationPlayback` enum selects whether a state clip repeats or stops at its final pose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationPlayback {
	Loop,
	Once,
}

/// The `AnimationClip` struct identifies the resource and playback behavior used by one state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnimationClip {
	resource_id: String,
	playback: AnimationPlayback,
}

impl AnimationClip {
	/// Creates a clip that restarts from its first sample after reaching its duration.
	pub fn looping(resource_id: impl Into<String>) -> Self {
		Self {
			resource_id: resource_id.into(),
			playback: AnimationPlayback::Loop,
		}
	}

	/// Creates a clip that holds its final sample after reaching its duration.
	pub fn once(resource_id: impl Into<String>) -> Self {
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

type TransitionPredicate<I> = Arc<dyn Fn(&I) -> bool + Send + Sync>;

enum AnimationTransitionTrigger<I> {
	Predicate(TransitionPredicate<I>),
	Finished,
	Always,
}

/// The `AnimationTransition` struct describes when and how a state changes.
pub struct AnimationTransition<I> {
	trigger: AnimationTransitionTrigger<I>,
	duration: MediaTime,
}

impl<I> AnimationTransition<I> {
	/// Creates a transition that starts when the typed input predicate returns true.
	pub fn when<F>(predicate: F) -> Self
	where
		F: Fn(&I) -> bool + Send + Sync + 'static,
	{
		Self {
			trigger: AnimationTransitionTrigger::Predicate(Arc::new(predicate)),
			duration: MediaTime::ZERO,
		}
	}

	/// Creates a transition that starts after a one-shot source clip reaches its final sample.
	pub const fn when_finished() -> Self {
		Self {
			trigger: AnimationTransitionTrigger::Finished,
			duration: MediaTime::ZERO,
		}
	}

	/// Creates an unconditional transition.
	pub const fn always() -> Self {
		Self {
			trigger: AnimationTransitionTrigger::Always,
			duration: MediaTime::ZERO,
		}
	}

	/// Smooths the transition with critically damped inertialization for the supplied duration.
	pub const fn inertialize(mut self, duration: MediaTime) -> Self {
		self.duration = duration;
		self
	}

	fn matches(&self, input: &I, source_finished: bool) -> bool {
		match &self.trigger {
			AnimationTransitionTrigger::Predicate(predicate) => predicate(input),
			AnimationTransitionTrigger::Finished => source_finished,
			AnimationTransitionTrigger::Always => true,
		}
	}
}

struct StateTransition<I> {
	target: AnimationStateId,
	transition: AnimationTransition<I>,
}

struct AnimationGraphState<I> {
	name: String,
	clip: AnimationClip,
	transitions: Vec<StateTransition<I>>,
}

/// The `AnimationGraph` struct stores an immutable, typed animation state machine.
///
/// Build the graph once with [`AnimationGraphBuilder`], then share it between
/// any number of [`AnimationGraphPlayer`] instances that use the same input type.
pub struct AnimationGraph<I> {
	states: Vec<AnimationGraphState<I>>,
	initial: AnimationStateId,
}

impl<I> AnimationGraph<I> {
	/// Starts a builder for a typed animation state machine.
	pub fn builder() -> AnimationGraphBuilder<I> {
		AnimationGraphBuilder::new()
	}

	/// Returns the graph's initial state.
	pub const fn initial_state(&self) -> AnimationStateId {
		self.initial
	}

	/// Returns the number of authored states.
	pub fn state_count(&self) -> usize {
		self.states.len()
	}

	fn state(&self, id: AnimationStateId) -> &AnimationGraphState<I> {
		// Builder validation keeps every runtime state ID in range.
		&self.states[id.0]
	}
}

struct PendingStateTransition<I> {
	source: AnimationStateId,
	target: AnimationStateId,
	transition: AnimationTransition<I>,
}

/// The `AnimationGraphBuilder` struct assembles named clip states and their ordered transitions.
pub struct AnimationGraphBuilder<I> {
	states: Vec<AnimationGraphState<I>>,
	transitions: Vec<PendingStateTransition<I>>,
}

impl<I> Default for AnimationGraphBuilder<I> {
	fn default() -> Self {
		Self::new()
	}
}

impl<I> AnimationGraphBuilder<I> {
	/// Creates an empty animation graph builder.
	pub fn new() -> Self {
		Self {
			states: Vec::new(),
			transitions: Vec::new(),
		}
	}

	/// Adds one named clip state and returns its stable graph-local ID.
	pub fn state(&mut self, name: impl Into<String>, clip: AnimationClip) -> AnimationStateId {
		let id = AnimationStateId(self.states.len());
		self.states.push(AnimationGraphState {
			name: name.into(),
			clip,
			transitions: Vec::new(),
		});
		id
	}

	/// Adds a transition after previously added transitions from the same source state.
	///
	/// The player checks transitions in this order and takes the first one that matches.
	pub fn transition(
		&mut self,
		source: AnimationStateId,
		target: AnimationStateId,
		transition: AnimationTransition<I>,
	) -> &mut Self {
		self.transitions.push(PendingStateTransition {
			source,
			target,
			transition,
		});
		self
	}

	/// Validates the graph and selects the initial state.
	pub fn build(mut self, initial: AnimationStateId) -> Result<AnimationGraph<I>, AnimationGraphBuildError> {
		if initial.0 >= self.states.len() {
			return Err(AnimationGraphBuildError::InitialStateOutOfRange {
				state: initial.0,
				state_count: self.states.len(),
			});
		}

		for (state_index, state) in self.states.iter().enumerate() {
			if state.name.trim().is_empty() {
				return Err(AnimationGraphBuildError::EmptyStateName { state: state_index });
			}
			if state.clip.resource_id.trim().is_empty() {
				return Err(AnimationGraphBuildError::EmptyResourceId { state: state_index });
			}
			if self.states[..state_index].iter().any(|other| other.name == state.name) {
				return Err(AnimationGraphBuildError::DuplicateStateName {
					name: state.name.clone(),
				});
			}
		}

		for pending in self.transitions {
			if pending.source.0 >= self.states.len() {
				return Err(AnimationGraphBuildError::TransitionSourceOutOfRange {
					state: pending.source.0,
					state_count: self.states.len(),
				});
			}
			if pending.target.0 >= self.states.len() {
				return Err(AnimationGraphBuildError::TransitionTargetOutOfRange {
					state: pending.target.0,
					state_count: self.states.len(),
				});
			}
			if pending.transition.duration < MediaTime::ZERO {
				return Err(AnimationGraphBuildError::NegativeTransitionDuration);
			}
			self.states[pending.source.0].transitions.push(StateTransition {
				target: pending.target,
				transition: pending.transition,
			});
		}

		Ok(AnimationGraph {
			states: self.states,
			initial,
		})
	}
}

/// The `AnimationGraphBuildError` enum reports invalid graph authoring input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnimationGraphBuildError {
	InitialStateOutOfRange { state: usize, state_count: usize },
	EmptyStateName { state: usize },
	EmptyResourceId { state: usize },
	DuplicateStateName { name: String },
	TransitionSourceOutOfRange { state: usize, state_count: usize },
	TransitionTargetOutOfRange { state: usize, state_count: usize },
	NegativeTransitionDuration,
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
			Self::NegativeTransitionDuration => write!(
				formatter,
				"Animation transition duration is negative. The most likely cause is using a timeline offset instead of a non-negative duration."
			),
		}
	}
}

impl std::error::Error for AnimationGraphBuildError {}

/// The `AnimationPoolConfig` struct sets the decoded clip memory available to an [`AnimationPool`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnimationPoolConfig {
	byte_budget: NonZeroUsize,
}

impl AnimationPoolConfig {
	/// Creates a strict byte budget for decoded animation resources.
	pub const fn new(byte_budget: NonZeroUsize) -> Self {
		Self { byte_budget }
	}

	/// Returns the maximum estimated decoded bytes retained by the pool cache.
	pub const fn byte_budget(self) -> usize {
		self.byte_budget.get()
	}
}

#[derive(Debug)]
struct CachedAnimation {
	resource_id: String,
	animation: Arc<Animation>,
	resident_bytes: usize,
	last_used: u64,
}

struct PendingAnimationLoad {
	resource_id: String,
	command: Option<AnimationLoadCommand>,
}

struct FailedAnimationLoad {
	resource_id: String,
}

struct BlockedAnimationLoad {
	resource_id: String,
	resident_bytes: usize,
}

enum AnimationLoadCommand {
	Load { resource_id: String },
}

enum AnimationLoadCompletion {
	Ready { resource_id: String, animation: Animation },
	Failed { resource_id: String, error: String },
}

/// The `AnimationPoolRequest` enum reports whether a requested clip can be sampled immediately.
#[derive(Debug)]
pub enum AnimationPoolRequest {
	Ready(Arc<Animation>),
	Loading,
	WaitingForCapacity,
	Failed,
}

/// The `AnimationPoolEvent` enum reports load outcomes that require application-level handling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnimationPoolEvent {
	LoadFailed {
		resource_id: String,
		error: String,
	},
	Oversized {
		resource_id: String,
		resident_bytes: usize,
		byte_budget: usize,
	},
	Evicted {
		resource_id: String,
	},
}

/// The `AnimationPool` struct owns a byte-bounded LRU cache of asynchronously loaded clips.
///
/// [`AnimationGraphPlayer::advance`] updates the pool before evaluating, so a
/// normal player loop needs no separate update call. Call [`Self::update`] only
/// when preloading clips during a frame without a player. Spawn the
/// [`AnimationLoadWorker`] returned by [`Self::new`] on the application runtime
/// that owns resource loading.
pub struct AnimationPool {
	commands: kanal::Sender<AnimationLoadCommand>,
	completions: kanal::Receiver<AnimationLoadCompletion>,
	cache: Vec<CachedAnimation>,
	pending: Vec<PendingAnimationLoad>,
	failed: Vec<FailedAnimationLoad>,
	blocked: Vec<BlockedAnimationLoad>,
	events: VecDeque<AnimationPoolEvent>,
	byte_budget: usize,
	resident_bytes: usize,
	next_use: u64,
	commands_closed: bool,
	completions_closed: bool,
}

impl AnimationPool {
	/// Creates the pool and its worker with bounded request and completion queues.
	pub fn new(resource_manager: EntityHandle<ResourceManager>, config: AnimationPoolConfig) -> (Self, AnimationLoadWorker) {
		let (commands, command_receiver) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
		let (completion_sender, completions) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
		(
			Self {
				commands: commands.to_sync(),
				completions: completions.to_sync(),
				cache: Vec::new(),
				pending: Vec::with_capacity(ANIMATION_LOAD_QUEUE_CAPACITY),
				failed: Vec::with_capacity(ANIMATION_LOAD_QUEUE_CAPACITY),
				blocked: Vec::with_capacity(ANIMATION_LOAD_QUEUE_CAPACITY),
				events: VecDeque::with_capacity(ANIMATION_POOL_EVENT_CAPACITY),
				byte_budget: config.byte_budget(),
				resident_bytes: 0,
				next_use: 0,
				commands_closed: false,
				completions_closed: false,
			},
			AnimationLoadWorker {
				resource_manager,
				commands: command_receiver,
				completions: completion_sender,
			},
		)
	}

	/// Submits queued loads and adopts completed clips without blocking the caller.
	pub fn update(&mut self) {
		self.submit_requests();
		if self.completions_closed {
			return;
		}

		loop {
			match self.completions.try_recv_realtime() {
				Ok(Some(completion)) => self.process_completion(completion),
				Ok(None) => break,
				Err(_) => {
					self.completions_closed = true;
					break;
				}
			}
		}
	}

	/// Returns a loaded clip or queues its asynchronous load without waiting.
	pub fn request(&mut self, resource_id: &str) -> AnimationPoolRequest {
		let use_stamp = self.next_use();
		if let Some(entry) = self.cache.iter_mut().find(|entry| entry.resource_id == resource_id) {
			entry.last_used = use_stamp;
			return AnimationPoolRequest::Ready(entry.animation.clone());
		}
		if self.failed.iter().any(|failure| failure.resource_id == resource_id) {
			return AnimationPoolRequest::Failed;
		}
		if let Some(index) = self.blocked.iter().position(|blocked| blocked.resource_id == resource_id) {
			let resident_bytes = self.blocked[index].resident_bytes;
			if !self.make_room(resident_bytes) {
				return AnimationPoolRequest::WaitingForCapacity;
			}
			self.blocked.swap_remove(index);
		} else if self.blocked.len() == ANIMATION_LOAD_QUEUE_CAPACITY {
			// Existing blocked requests get the next admission opportunity. Do not
			// start more work that cannot be retained under the strict byte budget.
			return AnimationPoolRequest::WaitingForCapacity;
		}
		if self.pending.iter().any(|pending| pending.resource_id == resource_id) {
			return AnimationPoolRequest::Loading;
		}

		if self.queue_load(resource_id) {
			AnimationPoolRequest::Loading
		} else {
			// The queue is bounded. The next player update retries without growing
			// a second unbounded waiting list.
			AnimationPoolRequest::WaitingForCapacity
		}
	}

	/// Marks a player-held clip as recently used without cloning or moving it.
	fn touch(&mut self, animation: &Arc<Animation>) {
		let use_stamp = self.next_use();
		if let Some(entry) = self.cache.iter_mut().find(|entry| Arc::ptr_eq(&entry.animation, animation)) {
			entry.last_used = use_stamp;
		}
	}

	/// Clears one recorded load failure and requests that resource again.
	pub fn retry(&mut self, resource_id: &str) -> AnimationPoolRequest {
		if let Some(index) = self.failed.iter().position(|failure| failure.resource_id == resource_id) {
			self.failed.swap_remove(index);
		}
		self.request(resource_id)
	}

	/// Returns the cache bytes currently retained by the pool.
	pub const fn resident_bytes(&self) -> usize {
		self.resident_bytes
	}

	/// Returns the configured cache byte budget.
	pub const fn byte_budget(&self) -> usize {
		self.byte_budget
	}

	/// Drains asynchronous load and eviction events without allocating a new event list.
	pub fn drain_events(&mut self) -> std::collections::vec_deque::Drain<'_, AnimationPoolEvent> {
		self.events.drain(..)
	}

	fn next_use(&mut self) -> u64 {
		let use_stamp = self.next_use;
		self.next_use = self.next_use.wrapping_add(1);
		use_stamp
	}

	fn queue_load(&mut self, resource_id: &str) -> bool {
		if self.commands_closed || self.pending.len() >= ANIMATION_LOAD_QUEUE_CAPACITY {
			return false;
		}
		let resource_id = resource_id.to_string();
		self.pending.push(PendingAnimationLoad {
			command: Some(AnimationLoadCommand::Load {
				resource_id: resource_id.clone(),
			}),
			resource_id,
		});
		true
	}

	/// Records a load failure until the caller deliberately retries that resource.
	fn remember_failure(&mut self, resource_id: String) {
		self.failed.push(FailedAnimationLoad { resource_id });
	}

	/// Retains the bounded set of clips that completed while every cache entry was in use.
	fn block_admission(&mut self, resource_id: String, resident_bytes: usize) {
		debug_assert!(self.blocked.len() < ANIMATION_LOAD_QUEUE_CAPACITY);
		self.blocked.push(BlockedAnimationLoad {
			resource_id,
			resident_bytes,
		});
	}

	/// Records the newest pool outcomes without making event consumption a memory requirement.
	fn push_event(&mut self, event: AnimationPoolEvent) {
		if self.events.len() == ANIMATION_POOL_EVENT_CAPACITY {
			self.events.pop_front();
		}
		self.events.push_back(event);
	}

	fn submit_requests(&mut self) {
		if self.commands_closed {
			return;
		}

		for pending in &mut self.pending {
			let Some(_) = pending.command else {
				continue;
			};
			match self.commands.try_send_option_realtime(&mut pending.command) {
				Ok(true) | Ok(false) => {}
				Err(_) => {
					self.commands_closed = true;
					break;
				}
			}
		}
	}

	fn process_completion(&mut self, completion: AnimationLoadCompletion) {
		let resource_id = match &completion {
			AnimationLoadCompletion::Ready { resource_id, .. } | AnimationLoadCompletion::Failed { resource_id, .. } => {
				resource_id.as_str()
			}
		};
		if let Some(index) = self.pending.iter().position(|pending| pending.resource_id == resource_id) {
			self.pending.swap_remove(index);
		}

		match completion {
			AnimationLoadCompletion::Ready { resource_id, animation } => self.admit(resource_id, animation),
			AnimationLoadCompletion::Failed { resource_id, error } => {
				self.remember_failure(resource_id.clone());
				self.push_event(AnimationPoolEvent::LoadFailed { resource_id, error });
			}
		}
	}

	fn admit(&mut self, resource_id: String, animation: Animation) {
		let resident_bytes = animation.estimated_resident_bytes();
		if resident_bytes > self.byte_budget {
			self.remember_failure(resource_id.clone());
			self.push_event(AnimationPoolEvent::Oversized {
				resource_id,
				resident_bytes,
				byte_budget: self.byte_budget,
			});
			return;
		}
		if !self.make_room(resident_bytes) {
			if self.blocked.len() < ANIMATION_LOAD_QUEUE_CAPACITY {
				self.block_admission(resource_id, resident_bytes);
			}
			return;
		}

		self.resident_bytes = self.resident_bytes.saturating_add(resident_bytes);
		let last_used = self.next_use();
		self.cache.push(CachedAnimation {
			resource_id,
			animation: Arc::new(animation),
			resident_bytes,
			last_used,
		});
	}

	/// Evicts inactive least-recently-used entries until a new clip fits.
	fn make_room(&mut self, required_bytes: usize) -> bool {
		if required_bytes > self.byte_budget {
			return false;
		}
		while self.resident_bytes.saturating_add(required_bytes) > self.byte_budget {
			let Some((index, _)) = self
				.cache
				.iter()
				.enumerate()
				.filter(|(_, entry)| Arc::strong_count(&entry.animation) == 1)
				.min_by_key(|(_, entry)| entry.last_used)
			else {
				return false;
			};
			let evicted = self.cache.swap_remove(index);
			self.resident_bytes = self.resident_bytes.saturating_sub(evicted.resident_bytes);
			self.push_event(AnimationPoolEvent::Evicted {
				resource_id: evicted.resource_id,
			});
		}
		true
	}
}

/// The `AnimationLoadWorker` struct resolves animation resources away from synchronous pose evaluation.
pub struct AnimationLoadWorker {
	resource_manager: EntityHandle<ResourceManager>,
	commands: kanal::AsyncReceiver<AnimationLoadCommand>,
	completions: kanal::AsyncSender<AnimationLoadCompletion>,
}

impl AnimationLoadWorker {
	/// Loads queued clips until the animation pool drops its command channel.
	pub async fn run(self) {
		while let Ok(AnimationLoadCommand::Load { resource_id }) = self.commands.recv().await {
			let completion = match self.resource_manager.request::<Animation>(&resource_id).await {
				Ok(reference) => AnimationLoadCompletion::Ready {
					resource_id,
					// Animation resources keep decoded curves in metadata, so the
					// reference reader is intentionally released before pooling.
					animation: reference.into_resource(),
				},
				Err(error) => AnimationLoadCompletion::Failed { resource_id, error },
			};
			if self.completions.send(completion).await.is_err() {
				break;
			}
			async_runtime::yield_now().await;
		}
	}
}

struct RuntimeClip {
	state: AnimationStateId,
	animation: Arc<Animation>,
	pose_map: SkeletonPoseMap,
	playback: AnimationPlayback,
	time_seconds: f32,
}

impl RuntimeClip {
	fn new(state: AnimationStateId, animation: Arc<Animation>, playback: AnimationPlayback, target: &Skeleton) -> Self {
		let pose_map = SkeletonPoseMap::by_name(animation.skeleton.resource(), target);
		Self {
			state,
			animation,
			pose_map,
			playback,
			time_seconds: 0.0,
		}
	}

	fn is_finished(&self) -> bool {
		self.playback == AnimationPlayback::Once && self.time_seconds >= self.animation.duration
	}

	fn advance(&mut self, delta: MediaTime) -> ClipAdvance {
		let previous_time = self.time_seconds;
		let duration = self.animation.duration;
		if duration <= 0.0 {
			self.time_seconds = 0.0;
			return ClipAdvance { wrapped_loops: 0 };
		}

		let advanced = previous_time + delta.as_seconds_f32();
		match self.playback {
			AnimationPlayback::Loop => {
				let wrapped_loops = (advanced / duration).floor().max(0.0) as usize;
				self.time_seconds = advanced.rem_euclid(duration);
				ClipAdvance { wrapped_loops }
			}
			AnimationPlayback::Once => {
				self.time_seconds = advanced.min(duration);
				ClipAdvance { wrapped_loops: 0 }
			}
		}
	}
}

#[derive(Clone, Copy)]
struct ClipAdvance {
	wrapped_loops: usize,
}

struct ActiveTransition {
	source: RuntimeClip,
	destination: RuntimeClip,
	duration: MediaTime,
	elapsed: MediaTime,
	begun: bool,
}

struct PendingPlayerTransition {
	target: AnimationStateId,
	duration: Option<MediaTime>,
}

#[derive(Clone, Copy)]
struct RootMotionTarget {
	node: usize,
	reference: LocalTransform,
}

/// Keeps a player target borrowed from an asset owner or directly owned by the player.
enum PlayerTargetSkeleton<'a> {
	Borrowed(&'a Skeleton),
	Owned(Skeleton),
}

impl Deref for PlayerTargetSkeleton<'_> {
	type Target = Skeleton;

	fn deref(&self) -> &Self::Target {
		match self {
			Self::Borrowed(target) => target,
			Self::Owned(target) => target,
		}
	}
}

/// The `AnimationGraphPose` struct borrows the latest player pose and frame root motion.
pub struct AnimationGraphPose<'a> {
	local_pose: &'a [LocalTransform],
	global_pose: &'a [Matrix],
	root_motion: RootMotionDelta,
}

impl<'a> AnimationGraphPose<'a> {
	/// Returns blendable local transforms with extracted root motion removed when configured.
	pub fn local_pose(&self) -> &'a [LocalTransform] {
		self.local_pose
	}

	/// Returns renderer-facing global skeleton matrices.
	pub fn global_pose(&self) -> &'a [Matrix] {
		self.global_pose
	}

	/// Returns this frame's root translation and rotation delta.
	pub const fn root_motion(&self) -> RootMotionDelta {
		self.root_motion
	}
}

/// The `AnimationGraphPlayer` struct evaluates one graph for one target skeleton without steady-state allocation.
///
/// Call [`Self::advance`] with the current input and shared [`AnimationPool`].
/// Apply [`AnimationGraphPose::root_motion`] to the owning transform, then send
/// [`AnimationGraphPose::global_pose`] through the renderer-facing pose update.
pub struct AnimationGraphPlayer<'graph, 'target, I> {
	graph: &'graph AnimationGraph<I>,
	target: PlayerTargetSkeleton<'target>,
	root_motion: Option<RootMotionTarget>,
	active: Option<RuntimeClip>,
	transition: Option<ActiveTransition>,
	pending: Option<PendingPlayerTransition>,
	active_previous: Vec<LocalTransform>,
	active_current: Vec<LocalTransform>,
	destination_previous: Vec<LocalTransform>,
	destination_current: Vec<LocalTransform>,
	active_source: Vec<LocalTransform>,
	destination_source: Vec<LocalTransform>,
	loop_source: Vec<LocalTransform>,
	loop_start: Vec<LocalTransform>,
	loop_end: Vec<LocalTransform>,
	local_pose: Vec<LocalTransform>,
	global_pose: Vec<Matrix>,
	inertializer: PoseInertializer,
}

impl<'graph, 'target, I> AnimationGraphPlayer<'graph, 'target, I> {
	/// Creates a player with retained pose storage sized for the target skeleton.
	///
	/// `root_motion_node` selects a target-skeleton node whose translation and
	/// rotation are delivered separately and removed from the visual pose.
	pub fn new(
		graph: &'graph AnimationGraph<I>,
		target: &'target Skeleton,
		root_motion_node: Option<usize>,
	) -> Result<Self, AnimationGraphPlayerError> {
		Self::with_target(graph, PlayerTargetSkeleton::Borrowed(target), root_motion_node)
	}

	/// Initializes a player after selecting its borrowed or owned skeleton storage.
	fn with_target(
		graph: &'graph AnimationGraph<I>,
		target: PlayerTargetSkeleton<'target>,
		root_motion_node: Option<usize>,
	) -> Result<Self, AnimationGraphPlayerError> {
		let root_motion = root_motion_node
			.map(|node| {
				target
					.nodes
					.get(node)
					.map(|root| RootMotionTarget {
						node,
						reference: root.rest_local,
					})
					.ok_or(AnimationGraphPlayerError::RootNodeOutOfRange {
						node,
						pose_len: target.nodes.len(),
					})
			})
			.transpose()?;
		let node_count = target.nodes.len();
		let rest_pose: Vec<_> = target.nodes.iter().map(|node| node.rest_local).collect();
		let mut global_pose = Vec::with_capacity(node_count);
		write_global_pose(&target, &rest_pose, &mut global_pose).expect("Target rest pose must match its skeleton node count");

		Ok(Self {
			graph,
			target,
			root_motion,
			active: None,
			transition: None,
			pending: Some(PendingPlayerTransition {
				target: graph.initial_state(),
				duration: None,
			}),
			active_previous: rest_pose.clone(),
			active_current: rest_pose.clone(),
			destination_previous: rest_pose.clone(),
			destination_current: rest_pose.clone(),
			active_source: Vec::new(),
			destination_source: Vec::new(),
			loop_source: Vec::new(),
			loop_start: rest_pose.clone(),
			loop_end: rest_pose.clone(),
			local_pose: rest_pose,
			global_pose,
			inertializer: PoseInertializer::new(node_count),
		})
	}

	/// Returns the currently playing destination state, if its initial clip has loaded.
	pub fn state(&self) -> Option<AnimationStateId> {
		self.transition
			.as_ref()
			.map(|transition| transition.destination.state)
			.or_else(|| self.active.as_ref().map(|active| active.state))
	}

	/// Advances playback, starts ready transitions, and borrows the resulting pose.
	///
	/// This first adopts any completed pool loads, then selects the first matching
	/// transition, evaluates the pose, and returns root motion for the same frame.
	/// Next, apply [`AnimationGraphPose::root_motion`] to the owning object and
	/// submit [`AnimationGraphPose::global_pose`] to the rendering system.
	pub fn advance(
		&mut self,
		delta: MediaTime,
		input: &I,
		pool: &mut AnimationPool,
	) -> Result<AnimationGraphPose<'_>, AnimationGraphPlayerError> {
		if delta < MediaTime::ZERO {
			return Err(AnimationGraphPlayerError::NegativeDelta);
		}
		pool.update();
		self.start_pending(pool);
		self.touch_active_clips(pool);

		if self.transition.is_none() && self.active.is_some() && self.pending.is_none() {
			self.select_transition(input);
			self.start_pending(pool);
		}

		let root_motion = if self.transition.is_some() {
			self.advance_transition(delta)
		} else if self.active.is_some() {
			self.advance_active(delta)
		} else {
			self.write_rest_pose();
			RootMotionDelta::IDENTITY
		};

		Ok(AnimationGraphPose {
			local_pose: &self.local_pose,
			global_pose: &self.global_pose,
			root_motion,
		})
	}

	/// Refreshes LRU state for clips retained by this player without cloning their resources.
	fn touch_active_clips(&self, pool: &mut AnimationPool) {
		if let Some(transition) = &self.transition {
			pool.touch(&transition.source.animation);
			pool.touch(&transition.destination.animation);
		} else if let Some(active) = &self.active {
			pool.touch(&active.animation);
		}
	}

	fn start_pending(&mut self, pool: &mut AnimationPool) {
		let Some(pending) = self.pending.as_ref() else {
			return;
		};
		let target_state = self.graph.state(pending.target);
		let AnimationPoolRequest::Ready(animation) = pool.request(target_state.clip.resource_id()) else {
			return;
		};
		let pending = self.pending.take().expect("pending state transition was checked above");
		let destination = RuntimeClip::new(pending.target, animation, target_state.clip.playback(), &self.target);

		if let Some(duration) = pending.duration {
			let source = self.active.take().expect("only a loaded state can start a graph transition");
			reserve_source_pose(&source, &mut self.loop_source);
			reserve_source_pose(&destination, &mut self.destination_source);
			sample_target_pose(
				&destination,
				&self.target,
				&mut self.destination_source,
				&mut self.destination_current,
			);
			self.destination_previous.copy_from_slice(&self.destination_current);
			self.transition = Some(ActiveTransition {
				source,
				destination,
				duration,
				elapsed: MediaTime::ZERO,
				begun: false,
			});
		} else {
			reserve_source_pose(&destination, &mut self.active_source);
			reserve_source_pose(&destination, &mut self.loop_source);
			sample_target_pose(&destination, &self.target, &mut self.active_source, &mut self.active_current);
			self.active_previous.copy_from_slice(&self.active_current);
			self.active = Some(destination);
		}
	}

	fn select_transition(&mut self, input: &I) {
		let active = self.active.as_ref().expect("transition selection requires an active clip");
		let source_state = self.graph.state(active.state);
		let source_finished = active.is_finished();
		let Some(transition) = source_state
			.transitions
			.iter()
			.find(|transition| transition.transition.matches(input, source_finished))
		else {
			return;
		};
		self.pending = Some(PendingPlayerTransition {
			target: transition.target,
			duration: Some(transition.transition.duration),
		});
	}

	fn advance_active(&mut self, delta: MediaTime) -> RootMotionDelta {
		let active = self.active.as_mut().expect("active clip was checked before advancing");
		let advance = active.advance(delta);
		std::mem::swap(&mut self.active_previous, &mut self.active_current);
		sample_target_pose(active, &self.target, &mut self.active_source, &mut self.active_current);
		let root_motion = root_delta(
			self.root_motion,
			active,
			&self.active_previous,
			&self.active_current,
			advance,
			&self.target,
			&mut self.loop_source,
			&mut self.loop_start,
			&mut self.loop_end,
		);
		self.local_pose.copy_from_slice(&self.active_current);
		self.remove_root_motion_from_visual_pose();
		self.write_global_pose();
		root_motion
	}

	fn advance_transition(&mut self, delta: MediaTime) -> RootMotionDelta {
		let root_motion_target = self.root_motion;
		let target = &self.target;
		let (root_motion, completed) = {
			let transition = self.transition.as_mut().expect("transition was checked before advancing");
			let source_advance = transition.source.advance(delta);
			let destination_advance = transition.destination.advance(delta);
			std::mem::swap(&mut self.active_previous, &mut self.active_current);
			std::mem::swap(&mut self.destination_previous, &mut self.destination_current);
			sample_target_pose(&transition.source, target, &mut self.active_source, &mut self.active_current);
			sample_target_pose(
				&transition.destination,
				target,
				&mut self.destination_source,
				&mut self.destination_current,
			);

			let source_root_motion = root_delta(
				root_motion_target,
				&transition.source,
				&self.active_previous,
				&self.active_current,
				source_advance,
				target,
				&mut self.loop_source,
				&mut self.loop_start,
				&mut self.loop_end,
			);
			let destination_root_motion = root_delta(
				root_motion_target,
				&transition.destination,
				&self.destination_previous,
				&self.destination_current,
				destination_advance,
				target,
				&mut self.loop_source,
				&mut self.loop_start,
				&mut self.loop_end,
			);

			let duration = transition.duration;
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
						.expect("player pose buffers must match the target skeleton");
					transition.begun = true;
				}
				self.inertializer
					.apply(&self.destination_current, delta, &mut self.local_pose)
					.expect("player pose buffers must match the target skeleton");
			}

			transition.elapsed = (transition.elapsed + delta).min(duration);
			let transition_factor = if duration == MediaTime::ZERO {
				1.0
			} else {
				(transition.elapsed.as_seconds_f32() / duration.as_seconds_f32()).clamp(0.0, 1.0)
			};
			(
				source_root_motion.blend(destination_root_motion, transition_factor),
				transition.elapsed == duration,
			)
		};
		self.remove_root_motion_from_visual_pose();
		self.write_global_pose();

		if completed {
			let destination = self
				.transition
				.take()
				.expect("transition remains owned until its duration completes")
				.destination;
			self.active_previous.copy_from_slice(&self.destination_current);
			self.active_current.copy_from_slice(&self.destination_current);
			self.active = Some(destination);
		}
		root_motion
	}

	fn write_rest_pose(&mut self) {
		for (output, node) in self.local_pose.iter_mut().zip(&self.target.nodes) {
			*output = node.rest_local;
		}
		self.write_global_pose();
	}

	fn remove_root_motion_from_visual_pose(&mut self) {
		let Some(root_motion) = self.root_motion else {
			return;
		};
		self.local_pose[root_motion.node].translation = root_motion.reference.translation;
		self.local_pose[root_motion.node].rotation = root_motion.reference.rotation;
	}

	fn write_global_pose(&mut self) {
		write_global_pose(&self.target, &self.local_pose, &mut self.global_pose)
			.expect("player local pose always matches its target skeleton");
	}
}

impl<'graph, I> AnimationGraphPlayer<'graph, 'static, I> {
	/// Creates a player that owns the target skeleton for its full playback lifetime.
	///
	/// Use this after receiving a mesh or skeleton from an async loader. It avoids
	/// coupling the player lifetime to a temporary resource reference. Next, call
	/// [`Self::advance`] from the application's per-frame animation system.
	pub fn new_owned(
		graph: &'graph AnimationGraph<I>,
		target: Skeleton,
		root_motion_node: Option<usize>,
	) -> Result<Self, AnimationGraphPlayerError> {
		Self::with_target(graph, PlayerTargetSkeleton::Owned(target), root_motion_node)
	}
}

/// The `AnimationGraphPlayerError` enum reports invalid player inputs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationGraphPlayerError {
	RootNodeOutOfRange { node: usize, pose_len: usize },
	NegativeDelta,
}

impl fmt::Display for AnimationGraphPlayerError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::RootNodeOutOfRange { node, pose_len } => write!(
				formatter,
				"Animation root-motion node is outside the target pose. The most likely cause is selecting node {node} in a skeleton with {pose_len} nodes."
			),
			Self::NegativeDelta => write!(
				formatter,
				"Animation graph advance delta is negative. The most likely cause is passing a timeline offset instead of a frame duration."
			),
		}
	}
}

impl std::error::Error for AnimationGraphPlayerError {}

/// Samples one loaded clip into target-skeleton local transforms using retained scratch buffers.
fn sample_target_pose(
	clip: &RuntimeClip,
	target: &Skeleton,
	source_output: &mut Vec<LocalTransform>,
	target_output: &mut Vec<LocalTransform>,
) {
	sample_local_pose(&clip.animation, clip.time_seconds, source_output);
	clip.pose_map
		.write_target_local_pose(source_output, target, target_output)
		.expect("animation pose maps are built from the source clip skeleton");
}

/// Reserves source-skeleton sampling storage when a clip becomes active, never during steady evaluation.
fn reserve_source_pose(clip: &RuntimeClip, output: &mut Vec<LocalTransform>) {
	let node_count = clip.animation.skeleton.resource().nodes.len();
	if output.capacity() < node_count {
		output.reserve(node_count - output.capacity());
	}
}

/// Calculates one clip's root delta, preserving forward progress across loop boundaries.
#[allow(clippy::too_many_arguments)]
fn root_delta(
	root_motion: Option<RootMotionTarget>,
	clip: &RuntimeClip,
	previous: &[LocalTransform],
	current: &[LocalTransform],
	advance: ClipAdvance,
	target: &Skeleton,
	loop_source: &mut Vec<LocalTransform>,
	loop_start: &mut Vec<LocalTransform>,
	loop_end: &mut Vec<LocalTransform>,
) -> RootMotionDelta {
	let Some(root_motion) = root_motion else {
		return RootMotionDelta::IDENTITY;
	};
	if advance.wrapped_loops == 0 {
		return RootMotionDelta::between(previous[root_motion.node], current[root_motion.node]);
	}

	// Sample the clip ends only for a loop crossing. This keeps the common
	// steady-state path to one clip sample while preserving forward root motion.
	sample_local_pose(&clip.animation, clip.animation.duration, loop_source);
	clip.pose_map
		.write_target_local_pose(loop_source, target, loop_end)
		.expect("animation pose maps are built from the source clip skeleton");
	sample_local_pose(&clip.animation, 0.0, loop_source);
	clip.pose_map
		.write_target_local_pose(loop_source, target, loop_start)
		.expect("animation pose maps are built from the source clip skeleton");

	let mut delta = RootMotionDelta::between(previous[root_motion.node], loop_end[root_motion.node]);
	let full_loop = RootMotionDelta::between(loop_start[root_motion.node], loop_end[root_motion.node]);
	for _ in 1..advance.wrapped_loops {
		delta = delta.then(full_loop);
	}
	delta.then(RootMotionDelta::between(
		loop_start[root_motion.node],
		current[root_motion.node],
	))
}

#[cfg(test)]
mod tests {
	use std::{collections::VecDeque, num::NonZeroUsize};

	use resource_management::{
		resources::{
			animation::{Animation, NodeTrack, Vector3Curve},
			skeleton::{LocalTransform, Skeleton, SkeletonNode},
		},
		Reference,
	};

	use super::{
		AnimationClip, AnimationGraph, AnimationGraphBuildError, AnimationGraphPlayer, AnimationPool, AnimationPoolConfig,
		AnimationPoolEvent, AnimationPoolRequest, AnimationTransition,
	};
	use crate::MediaTime;

	fn test_skeleton() -> Skeleton {
		Skeleton {
			nodes: vec![SkeletonNode {
				name: Some("root".into()),
				parent: None,
				rest_local: LocalTransform::identity(),
			}],
		}
	}

	fn test_animation(name: &str, end_translation: f32) -> Animation {
		Animation {
			name: Some(name.into()),
			skeleton: Reference::in_memory("test.skeleton", test_skeleton()),
			duration: 1.0,
			tracks: vec![NodeTrack {
				node: 0,
				translation: Some(Vector3Curve::Linear {
					times: vec![0.0, 1.0],
					values: vec![[0.0; 3], [end_translation, 0.0, 0.0]],
				}),
				rotation: None,
				scale: None,
			}],
		}
	}

	fn pool(byte_budget: usize) -> AnimationPool {
		let (commands, _command_receiver) = kanal::bounded_async(super::ANIMATION_LOAD_QUEUE_CAPACITY);
		let (_completion_sender, completions) = kanal::bounded_async(super::ANIMATION_LOAD_QUEUE_CAPACITY);
		AnimationPool {
			commands: commands.to_sync(),
			completions: completions.to_sync(),
			cache: Vec::new(),
			pending: Vec::with_capacity(super::ANIMATION_LOAD_QUEUE_CAPACITY),
			failed: Vec::with_capacity(super::ANIMATION_LOAD_QUEUE_CAPACITY),
			blocked: Vec::with_capacity(super::ANIMATION_LOAD_QUEUE_CAPACITY),
			events: VecDeque::with_capacity(super::ANIMATION_POOL_EVENT_CAPACITY),
			byte_budget,
			resident_bytes: 0,
			next_use: 0,
			commands_closed: false,
			completions_closed: false,
		}
	}

	#[test]
	fn builder_preserves_transition_order_and_rejects_invalid_graphs() {
		let mut builder = AnimationGraph::<bool>::builder();
		let idle = builder.state("idle", AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk", AnimationClip::looping("walk.animation"));
		builder
			.transition(idle, walk, AnimationTransition::when(|input| *input))
			.transition(
				idle,
				idle,
				AnimationTransition::always().inertialize(MediaTime::from_millis(100)),
			);
		let graph = builder.build(idle).expect("expected graph value");
		assert_eq!(graph.state_count(), 2);
		assert_eq!(graph.state(idle).transitions.len(), 2);

		let mut invalid = AnimationGraph::<()>::builder();
		let state = invalid.state("", AnimationClip::once("clip.animation"));
		assert!(matches!(
			invalid.build(state),
			Err(AnimationGraphBuildError::EmptyStateName { state: 0 })
		));
	}

	#[test]
	fn pool_configuration_exposes_its_strict_byte_budget() {
		let config = AnimationPoolConfig::new(NonZeroUsize::new(128).expect("non-zero budget"));
		assert_eq!(config.byte_budget(), 128);
		assert_eq!(super::ANIMATION_LOAD_QUEUE_CAPACITY, 64);
		let _ = std::mem::size_of::<AnimationPool>();
		let _ = AnimationPoolEvent::Evicted {
			resource_id: "idle.animation".into(),
		};
	}

	#[test]
	fn pool_evicts_lru_entries_but_keeps_clips_pinned_by_players() {
		let idle = test_animation("idle", 1.0);
		let walk = test_animation("walk", 2.0);
		let budget = idle.estimated_resident_bytes().max(walk.estimated_resident_bytes());
		let mut first_pool = pool(budget);

		first_pool.admit("idle.animation".into(), idle);
		first_pool.admit("walk.animation".into(), walk);
		assert!(first_pool.cache.iter().any(|entry| entry.resource_id == "walk.animation"));
		assert!(first_pool.cache.iter().all(|entry| entry.resource_id != "idle.animation"));
		assert!(first_pool
			.drain_events()
			.any(|event| matches!(event, AnimationPoolEvent::Evicted { resource_id } if resource_id == "idle.animation")));

		let idle = test_animation("idle", 1.0);
		let walk = test_animation("walk", 2.0);
		let budget = idle.estimated_resident_bytes().max(walk.estimated_resident_bytes());
		let mut pool = pool(budget);
		pool.admit("idle.animation".into(), idle);
		let pinned = match pool.request("idle.animation") {
			AnimationPoolRequest::Ready(animation) => animation,
			_ => panic!("expected cached idle animation"),
		};
		pool.admit("walk.animation".into(), walk);
		assert!(pool.cache.iter().any(|entry| entry.resource_id == "idle.animation"));
		assert!(matches!(
			pool.request("walk.animation"),
			AnimationPoolRequest::WaitingForCapacity
		));

		drop(pinned);
		assert!(matches!(pool.request("walk.animation"), AnimationPoolRequest::Loading));
		assert!(pool.cache.iter().all(|entry| entry.resource_id != "idle.animation"));
	}

	#[test]
	fn oversized_clips_fail_once_until_the_caller_explicitly_retries_them() {
		let animation = test_animation("oversized", 1.0);
		let mut pool = pool(animation.estimated_resident_bytes() - 1);
		pool.admit("oversized.animation".into(), animation);

		assert!(matches!(pool.request("oversized.animation"), AnimationPoolRequest::Failed));
		assert!(pool.drain_events().any(|event| matches!(
			event,
			AnimationPoolEvent::Oversized {
				resource_id,
				..
			} if resource_id == "oversized.animation"
		)));
	}

	#[test]
	fn player_returns_root_motion_and_removes_it_from_the_visual_pose() {
		let target = test_skeleton();
		let idle = test_animation("idle", 1.0);
		let run = test_animation("run", 3.0);
		let byte_budget = idle.estimated_resident_bytes().saturating_add(run.estimated_resident_bytes());
		let mut pool = pool(byte_budget);
		pool.admit("idle.animation".into(), idle);
		pool.admit("run.animation".into(), run);

		let mut builder = AnimationGraph::<bool>::builder();
		let idle = builder.state("idle", AnimationClip::looping("idle.animation"));
		let run = builder.state("run", AnimationClip::looping("run.animation"));
		builder.transition(idle, run, AnimationTransition::when(|running| *running));
		let graph = builder.build(idle).expect("graph should build");
		let mut player = AnimationGraphPlayer::new(&graph, &target, Some(0)).expect("player should build");

		let initial = player.advance(MediaTime::ZERO, &false, &mut pool).expect("initial pose");
		assert_eq!(initial.local_pose()[0], LocalTransform::identity());
		let root_motion = player
			.advance(MediaTime::from_millis(500), &false, &mut pool)
			.expect("idle pose")
			.root_motion();
		assert_eq!(root_motion.translation, [0.5, 0.0, 0.0]);
		assert_eq!(
			player
				.advance(MediaTime::ZERO, &false, &mut pool)
				.expect("visual pose")
				.local_pose()[0]
				.translation,
			[0.0; 3]
		);

		let switched = player.advance(MediaTime::ZERO, &true, &mut pool).expect("run transition");
		assert_eq!(switched.root_motion().translation, [0.0; 3]);
		assert_eq!(player.state(), Some(run));
		assert_eq!(
			player
				.advance(MediaTime::from_millis(500), &true, &mut pool)
				.expect("run pose")
				.root_motion()
				.translation,
			[1.5, 0.0, 0.0]
		);
		assert_eq!(
			player
				.advance(MediaTime::from_millis(750), &true, &mut pool)
				.expect("looped run pose")
				.root_motion()
				.translation,
			[2.25, 0.0, 0.0]
		);
	}
}
