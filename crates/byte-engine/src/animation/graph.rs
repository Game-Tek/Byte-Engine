//! Code-authored skeletal animation state machines and asynchronous clip pooling.
//!
//! Build an [`AnimationGraph`] with [`AnimationGraphBuilder`], then create an
//! [`AnimationGraphPlayer`] for each animated skeleton. The player evaluates
//! synchronously from retained pose buffers while [`AnimationPool`] loads clip
//! resources on an application-owned async worker. Use
//! [`AnimationGraphBuilder::transition_state`] for a one-shot authored clip,
//! such as a locomotion start or stop, that completes into its configured successor state.
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
	Reference,
};

use super::{
	inertialization::PoseInertializer,
	math::multiply_quaternion,
	packed::{PackedAnimation, PackedAnimationData},
	root_motion::RootMotionDelta,
	skeletal::write_global_pose,
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

/// The `AnimationLease` struct keeps a stable clip identity across arena residency and eviction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnimationLease {
	resource_id: String,
}

impl AnimationLease {
	/// Creates a lease handle for a clip that the pool may load, evict, and load again.
	pub fn new(resource_id: impl Into<String>) -> Self {
		Self {
			resource_id: resource_id.into(),
		}
	}

	/// Returns the resource ID used when an evicted lease needs another asynchronous load.
	pub fn resource_id(&self) -> &str {
		&self.resource_id
	}
}

/// The `AnimationClip` struct identifies the leased resource and playback behavior used by one state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnimationClip {
	lease: AnimationLease,
	playback: AnimationPlayback,
}

impl AnimationClip {
	/// Creates a clip that restarts from its first sample after reaching its duration.
	pub fn looping(resource_id: impl Into<String>) -> Self {
		Self {
			lease: AnimationLease::new(resource_id),
			playback: AnimationPlayback::Loop,
		}
	}

	/// Creates a clip that holds its final sample after reaching its duration.
	pub fn once(resource_id: impl Into<String>) -> Self {
		Self {
			lease: AnimationLease::new(resource_id),
			playback: AnimationPlayback::Once,
		}
	}

	/// Returns the resource ID requested by an [`AnimationPool`].
	pub fn resource_id(&self) -> &str {
		self.lease.resource_id()
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

/// Distinguishes persistent clips from one-shot clips that complete into another state.
enum AnimationGraphStateKind {
	Persistent,
	Transition { completion: AnimationStateId },
}

struct AnimationGraphState<I> {
	name: String,
	clip: AnimationClip,
	kind: AnimationGraphStateKind,
	transitions: Vec<StateTransition<I>>,
}

impl<I> AnimationGraphState<I> {
	/// Returns the fallback target entered after a transition-state clip finishes.
	fn completion_target(&self) -> Option<AnimationStateId> {
		match self.kind {
			AnimationGraphStateKind::Persistent => None,
			AnimationGraphStateKind::Transition { completion } => Some(completion),
		}
	}

	/// Selects the first authored exit, then falls back to transition-state completion.
	fn select_transition(&self, input: &I, source_finished: bool) -> Option<(AnimationStateId, MediaTime)> {
		self.transitions
			.iter()
			.find(|transition| transition.transition.matches(input, source_finished))
			.map(|transition| (transition.target, transition.transition.duration))
			.or_else(|| {
				source_finished
					.then(|| self.completion_target().map(|target| (target, MediaTime::ZERO)))
					.flatten()
			})
	}
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
			kind: AnimationGraphStateKind::Persistent,
			transitions: Vec::new(),
		});
		id
	}

	/// Adds a one-shot state that falls through to `completion` after it finishes.
	///
	/// Use this for authored movement starts, stops, turns, and other clips that
	/// bridge states. `clip` must use [`AnimationPlayback::Once`]. Authored
	/// transitions from this state run before its completion, so they can cancel
	/// or redirect the transient animation.
	pub fn transition_state(
		&mut self,
		name: impl Into<String>,
		clip: AnimationClip,
		completion: AnimationStateId,
	) -> AnimationStateId {
		let id = AnimationStateId(self.states.len());
		self.states.push(AnimationGraphState {
			name: name.into(),
			clip,
			kind: AnimationGraphStateKind::Transition { completion },
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
			if state.clip.resource_id().trim().is_empty() {
				return Err(AnimationGraphBuildError::EmptyResourceId { state: state_index });
			}
			if self.states[..state_index].iter().any(|other| other.name == state.name) {
				return Err(AnimationGraphBuildError::DuplicateStateName {
					name: state.name.clone(),
				});
			}
			if let AnimationGraphStateKind::Transition { completion } = state.kind {
				if state.clip.playback() != AnimationPlayback::Once {
					return Err(AnimationGraphBuildError::TransitionStateMustPlayOnce { state: state_index });
				}
				if completion.0 >= self.states.len() {
					return Err(AnimationGraphBuildError::TransitionStateCompletionOutOfRange {
						state: state_index,
						completion: completion.0,
						state_count: self.states.len(),
					});
				}
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
	InitialStateOutOfRange {
		state: usize,
		state_count: usize,
	},
	EmptyStateName {
		state: usize,
	},
	EmptyResourceId {
		state: usize,
	},
	DuplicateStateName {
		name: String,
	},
	TransitionSourceOutOfRange {
		state: usize,
		state_count: usize,
	},
	TransitionTargetOutOfRange {
		state: usize,
		state_count: usize,
	},
	TransitionStateMustPlayOnce {
		state: usize,
	},
	TransitionStateCompletionOutOfRange {
		state: usize,
		completion: usize,
		state_count: usize,
	},
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AnimationArenaRegion {
	offset: usize,
	word_count: usize,
}

impl AnimationArenaRegion {
	fn end(self) -> usize {
		self.offset + self.word_count
	}
}

#[derive(Debug)]
struct CachedAnimation {
	resource_id: String,
	skeleton: Reference<Skeleton>,
	region: AnimationArenaRegion,
	last_used: std::cell::Cell<u64>,
	lease_count: std::cell::Cell<usize>,
}

/// Pins one resident arena region for the duration of an animation evaluation.
struct ResidentAnimationLease<'a> {
	entry: &'a CachedAnimation,
	words: &'a [u32],
}

impl ResidentAnimationLease<'_> {
	fn packed(&self) -> PackedAnimation<'_> {
		PackedAnimation::from_words(self.words)
	}

	fn skeleton(&self) -> &Skeleton {
		self.entry.skeleton.resource()
	}
}

impl Drop for ResidentAnimationLease<'_> {
	fn drop(&mut self) {
		self.entry.lease_count.set(
			self.entry
				.lease_count
				.get()
				.checked_sub(1)
				.expect("Resident animation lease count must match evaluation borrows."),
		);
	}
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
	animation: Animation,
}

enum AnimationLoadCommand {
	Load { resource_id: String },
}

enum AnimationLoadCompletion {
	Ready { resource_id: String, animation: Animation },
	Failed { resource_id: String, error: String },
}

/// The `AnimationPoolRequest` enum reports whether a lease can be sampled immediately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationPoolRequest {
	Ready,
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

/// The `AnimationPool` struct owns a preallocated word arena and byte-bounded LRU clip cache.
///
/// Graph clips keep stable [`AnimationLease`] handles across eviction. During
/// evaluation, the player pins resident arena regions so admission cannot reuse
/// their words until sampling completes.
pub struct AnimationPool {
	commands: kanal::Sender<AnimationLoadCommand>,
	completions: kanal::Receiver<AnimationLoadCompletion>,
	storage: Box<[u32]>,
	free_regions: Vec<AnimationArenaRegion>,
	cache: Vec<CachedAnimation>,
	pending: Vec<PendingAnimationLoad>,
	failed: Vec<FailedAnimationLoad>,
	blocked: Vec<BlockedAnimationLoad>,
	events: VecDeque<AnimationPoolEvent>,
	byte_budget: usize,
	resident_bytes: usize,
	next_use: std::cell::Cell<u64>,
	commands_closed: bool,
	completions_closed: bool,
}

impl AnimationPool {
	/// Creates the pool, preallocates its complete word arena, and returns its load worker.
	pub fn new(resource_manager: EntityHandle<ResourceManager>, config: AnimationPoolConfig) -> (Self, AnimationLoadWorker) {
		let (commands, command_receiver) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
		let (completion_sender, completions) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
		let word_capacity = config.byte_budget() / std::mem::size_of::<u32>();
		let free_regions = (word_capacity > 0)
			.then_some(AnimationArenaRegion {
				offset: 0,
				word_count: word_capacity,
			})
			.into_iter()
			.collect();
		(
			Self {
				commands: commands.to_sync(),
				completions: completions.to_sync(),
				storage: vec![0; word_capacity].into_boxed_slice(),
				free_regions,
				cache: Vec::with_capacity(ANIMATION_LOAD_QUEUE_CAPACITY),
				pending: Vec::with_capacity(ANIMATION_LOAD_QUEUE_CAPACITY),
				failed: Vec::with_capacity(ANIMATION_LOAD_QUEUE_CAPACITY),
				blocked: Vec::with_capacity(ANIMATION_LOAD_QUEUE_CAPACITY),
				events: VecDeque::with_capacity(ANIMATION_POOL_EVENT_CAPACITY),
				byte_budget: config.byte_budget(),
				resident_bytes: 0,
				next_use: std::cell::Cell::new(0),
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

	/// Returns lease residency or queues an asynchronous reload after eviction.
	pub fn request(&mut self, lease: &AnimationLease) -> AnimationPoolRequest {
		let resource_id = lease.resource_id();
		if self.cache.iter().any(|entry| entry.resource_id == resource_id) {
			return AnimationPoolRequest::Ready;
		}
		if self.failed.iter().any(|failure| failure.resource_id == resource_id) {
			return AnimationPoolRequest::Failed;
		}
		if let Some(index) = self.blocked.iter().position(|blocked| blocked.resource_id == resource_id) {
			let resident_bytes = self.blocked[index].resident_bytes;
			if !self.make_room(resident_bytes) {
				return AnimationPoolRequest::WaitingForCapacity;
			}
			let blocked = self.blocked.swap_remove(index);
			self.write_animation(blocked.resource_id, blocked.animation);
			return AnimationPoolRequest::Ready;
		} else if self.blocked.len() == ANIMATION_LOAD_QUEUE_CAPACITY {
			return AnimationPoolRequest::WaitingForCapacity;
		}
		if self.pending.iter().any(|pending| pending.resource_id == resource_id) {
			return AnimationPoolRequest::Loading;
		}
		if self.queue_load(resource_id) {
			AnimationPoolRequest::Loading
		} else {
			AnimationPoolRequest::WaitingForCapacity
		}
	}

	/// Pins a resident clip until the returned evaluation lease is dropped.
	fn acquire(&self, lease: &AnimationLease) -> Option<ResidentAnimationLease<'_>> {
		let entry = self.cache.iter().find(|entry| entry.resource_id == lease.resource_id())?;
		entry.last_used.set(self.next_use());
		entry.lease_count.set(entry.lease_count.get() + 1);
		Some(ResidentAnimationLease {
			entry,
			words: &self.storage[entry.region.offset..entry.region.end()],
		})
	}

	/// Clears one recorded load failure and requests that lease again.
	pub fn retry(&mut self, lease: &AnimationLease) -> AnimationPoolRequest {
		if let Some(index) = self
			.failed
			.iter()
			.position(|failure| failure.resource_id == lease.resource_id())
		{
			self.failed.swap_remove(index);
		}
		self.request(lease)
	}

	/// Returns bytes occupied by resident packed clip regions.
	pub const fn resident_bytes(&self) -> usize {
		self.resident_bytes
	}

	/// Returns the configured arena byte budget.
	pub const fn byte_budget(&self) -> usize {
		self.byte_budget
	}

	/// Drains asynchronous load and eviction events without allocating a new event list.
	pub fn drain_events(&mut self) -> std::collections::vec_deque::Drain<'_, AnimationPoolEvent> {
		self.events.drain(..)
	}

	fn next_use(&self) -> u64 {
		let value = self.next_use.get();
		self.next_use.set(value.wrapping_add(1));
		value
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

	fn remember_failure(&mut self, resource_id: String) {
		self.failed.push(FailedAnimationLoad { resource_id });
	}
	fn block_admission(&mut self, resource_id: String, resident_bytes: usize, animation: Animation) {
		debug_assert!(self.blocked.len() < ANIMATION_LOAD_QUEUE_CAPACITY);
		self.blocked.push(BlockedAnimationLoad {
			resource_id,
			resident_bytes,
			animation,
		});
	}
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
			if pending.command.is_none() {
				continue;
			}
			match self.commands.try_send_option_realtime(&mut pending.command) {
				Ok(_) => {}
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
		let resident_bytes = PackedAnimationData::resident_bytes(&animation);
		if resident_bytes > self.byte_budget || resident_bytes / std::mem::size_of::<u32>() > self.storage.len() {
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
				self.block_admission(resource_id, resident_bytes, animation);
			}
			return;
		}
		self.write_animation(resource_id, animation);
	}

	/// Packs a completed load only after admission owns a contiguous arena range.
	fn write_animation(&mut self, resource_id: String, animation: Animation) {
		let packed = PackedAnimationData::from_resource(animation);
		let resident_bytes = packed.data.len() * std::mem::size_of::<u32>();
		let region = self
			.take_region(packed.data.len())
			.expect("Animation admission reserved one contiguous arena region.");
		self.storage[region.offset..region.end()].copy_from_slice(&packed.data);
		self.resident_bytes += resident_bytes;
		self.cache.push(CachedAnimation {
			resource_id,
			skeleton: packed.skeleton,
			region,
			last_used: std::cell::Cell::new(self.next_use()),
			lease_count: std::cell::Cell::new(0),
		});
	}

	/// Evicts unleased LRU entries until one contiguous arena range can hold the requested words.
	fn make_room(&mut self, required_bytes: usize) -> bool {
		let required_words = required_bytes.div_ceil(std::mem::size_of::<u32>());
		if required_bytes > self.byte_budget || required_words > self.storage.len() {
			return false;
		}
		while !self.free_regions.iter().any(|region| region.word_count >= required_words) {
			let Some((index, _)) = self
				.cache
				.iter()
				.enumerate()
				.filter(|(_, entry)| entry.lease_count.get() == 0)
				.min_by_key(|(_, entry)| entry.last_used.get())
			else {
				return false;
			};
			let evicted = self.cache.swap_remove(index);
			self.resident_bytes = self
				.resident_bytes
				.saturating_sub(evicted.region.word_count * std::mem::size_of::<u32>());
			self.return_region(evicted.region);
			self.push_event(AnimationPoolEvent::Evicted {
				resource_id: evicted.resource_id,
			});
		}
		true
	}

	fn take_region(&mut self, word_count: usize) -> Option<AnimationArenaRegion> {
		let index = self.free_regions.iter().position(|region| region.word_count >= word_count)?;
		let available = self.free_regions[index];
		let region = AnimationArenaRegion {
			offset: available.offset,
			word_count,
		};
		if available.word_count == word_count {
			self.free_regions.swap_remove(index);
		} else {
			self.free_regions[index] = AnimationArenaRegion {
				offset: available.offset + word_count,
				word_count: available.word_count - word_count,
			};
		}
		Some(region)
	}

	/// Returns and coalesces an arena region so fragmented evictions can satisfy later clips.
	fn return_region(&mut self, region: AnimationArenaRegion) {
		let index = self.free_regions.partition_point(|free| free.offset < region.offset);
		self.free_regions.insert(index, region);
		let mut index = index.saturating_sub(1);
		while index + 1 < self.free_regions.len() {
			if self.free_regions[index].end() != self.free_regions[index + 1].offset {
				index += 1;
				continue;
			}
			let right = self.free_regions.remove(index + 1);
			self.free_regions[index].word_count += right.word_count;
		}
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
	lease: AnimationLease,
	pose_map: SkeletonPoseMap,
	playback: AnimationPlayback,
	duration: f32,
	source_node_count: usize,
	time_seconds: f32,
}

impl RuntimeClip {
	fn new(
		state: AnimationStateId,
		lease: AnimationLease,
		resident: &ResidentAnimationLease<'_>,
		playback: AnimationPlayback,
		target: &Skeleton,
	) -> Self {
		let pose_map = SkeletonPoseMap::by_name(resident.skeleton(), target);
		Self {
			state,
			lease,
			pose_map,
			playback,
			duration: resident.packed().duration(),
			source_node_count: resident.skeleton().nodes.len(),
			time_seconds: 0.0,
		}
	}

	fn is_finished(&self) -> bool {
		self.playback == AnimationPlayback::Once && self.time_seconds >= self.duration
	}

	fn advance(&mut self, delta: MediaTime) -> ClipAdvance {
		let previous_time = self.time_seconds;
		let duration = self.duration;
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

	const fn contains(self, component: usize) -> bool {
		self.0 & (1 << component) != 0
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

/// The `RootMotionSettings` struct defines which motion a target-skeleton node contributes to its owning object.
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
	/// `root_motion` selects a uniquely named target-skeleton node and the node-local
	/// channels delivered in object space and removed from the visual pose. Prefer
	/// all channels from a dedicated root. For locomotion authored on hips, select
	/// only the travel axes so the pose retains its vertical sway and rotation.
	pub fn new(
		graph: &'graph AnimationGraph<I>,
		target: &'target Skeleton,
		root_motion: Option<RootMotionSettings<'_>>,
	) -> Result<Self, AnimationGraphPlayerError> {
		Self::with_target(graph, PlayerTargetSkeleton::Borrowed(target), root_motion)
	}

	/// Initializes a player after selecting its borrowed or owned skeleton storage.
	fn with_target(
		graph: &'graph AnimationGraph<I>,
		target: PlayerTargetSkeleton<'target>,
		root_motion: Option<RootMotionSettings<'_>>,
	) -> Result<Self, AnimationGraphPlayerError> {
		let root_motion = resolve_root_motion_target(&target, root_motion)?;
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
	/// While a selected target loads, the player retains the source clip and
	/// reevaluates its exits so stale input can cancel or retarget that request.
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
		self.refresh_pending_transition(input);
		self.start_pending(pool);

		if self.transition.is_none() && self.active.is_some() && self.pending.is_none() {
			self.select_transition(input);
			self.start_pending(pool);
		}

		// Resolve every clip before borrowing arena regions. The resulting leases
		// pin those regions until this evaluation and all root-motion samples finish.
		let root_motion = if let Some(transition) = &self.transition {
			let source_handle = transition.source.lease.clone();
			let destination_handle = transition.destination.lease.clone();
			let source_ready = pool.request(&source_handle) == AnimationPoolRequest::Ready;
			let destination_ready = pool.request(&destination_handle) == AnimationPoolRequest::Ready;
			if source_ready && destination_ready {
				let source = pool.acquire(&source_handle).expect("ready source lease must remain resident");
				let destination = pool
					.acquire(&destination_handle)
					.expect("ready destination lease must remain resident");
				self.advance_transition(delta, &source, &destination)
			} else {
				RootMotionDelta::IDENTITY
			}
		} else if let Some(active) = &self.active {
			let handle = active.lease.clone();
			if pool.request(&handle) == AnimationPoolRequest::Ready {
				let resident = pool.acquire(&handle).expect("ready active lease must remain resident");
				self.advance_active(delta, &resident)
			} else {
				RootMotionDelta::IDENTITY
			}
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

	fn start_pending(&mut self, pool: &mut AnimationPool) {
		let Some(pending) = self.pending.as_ref() else {
			return;
		};
		let target_state = self.graph.state(pending.target);
		let lease = target_state.clip.lease.clone();
		if pool.request(&lease) != AnimationPoolRequest::Ready {
			return;
		}
		let resident = pool.acquire(&lease).expect("ready pending lease must remain resident");
		let pending = self.pending.take().expect("pending state transition was checked above");
		let destination = RuntimeClip::new(pending.target, lease, &resident, target_state.clip.playback(), &self.target);

		if let Some(duration) = pending.duration {
			let source = self.active.take().expect("only a loaded state can start a graph transition");
			reserve_source_pose(&source, &mut self.loop_source);
			reserve_source_pose(&destination, &mut self.destination_source);
			sample_target_pose(
				&destination,
				&resident,
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
			sample_target_pose(
				&destination,
				&resident,
				&self.target,
				&mut self.active_source,
				&mut self.active_current,
			);
			self.active_previous.copy_from_slice(&self.active_current);
			self.active = Some(destination);
		}
	}

	/// Cancels or retargets a loading state when its source's selected edge changes.
	fn refresh_pending_transition(&mut self, input: &I) {
		let Some(pending) = self.pending.as_ref() else {
			return;
		};
		if pending.duration.is_none() || self.transition.is_some() {
			return;
		}
		let selected = self
			.active
			.as_ref()
			.and_then(|active| self.selected_transition(active, input));
		self.pending = selected;
	}

	fn select_transition(&mut self, input: &I) {
		let selected = self.active.as_ref().expect("transition selection requires an active clip");
		self.pending = self.selected_transition(selected, input);
	}

	/// Resolves one source state to its highest-priority current destination.
	fn selected_transition(&self, active: &RuntimeClip, input: &I) -> Option<PendingPlayerTransition> {
		let source_state = self.graph.state(active.state);
		source_state
			.select_transition(input, active.is_finished())
			.map(|(target, duration)| PendingPlayerTransition {
				target,
				duration: Some(duration),
			})
	}

	fn advance_active(&mut self, delta: MediaTime, resident: &ResidentAnimationLease<'_>) -> RootMotionDelta {
		let active = self.active.as_mut().expect("active clip was checked before advancing");
		let advance = active.advance(delta);
		std::mem::swap(&mut self.active_previous, &mut self.active_current);
		sample_target_pose(
			active,
			resident,
			&self.target,
			&mut self.active_source,
			&mut self.active_current,
		);
		let root_motion = root_delta(
			self.root_motion,
			active,
			&self.active_previous,
			&self.active_current,
			advance,
			resident,
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

	fn advance_transition(
		&mut self,
		delta: MediaTime,
		source_resident: &ResidentAnimationLease<'_>,
		destination_resident: &ResidentAnimationLease<'_>,
	) -> RootMotionDelta {
		let root_motion_target = self.root_motion;
		let target = &self.target;
		let (root_motion, completed) = {
			let transition = self.transition.as_mut().expect("transition was checked before advancing");
			let source_advance = transition.source.advance(delta);
			let destination_advance = transition.destination.advance(delta);
			std::mem::swap(&mut self.active_previous, &mut self.active_current);
			std::mem::swap(&mut self.destination_previous, &mut self.destination_current);
			sample_target_pose(
				&transition.source,
				source_resident,
				target,
				&mut self.active_source,
				&mut self.active_current,
			);
			sample_target_pose(
				&transition.destination,
				destination_resident,
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
				source_resident,
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
				destination_resident,
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
		for component in 0..3 {
			if root_motion.translation.contains(component) {
				self.local_pose[root_motion.node].translation[component] = root_motion.reference.translation[component];
			}
		}
		if root_motion.rotation == RootMotionRotation::Full {
			self.local_pose[root_motion.node].rotation = root_motion.reference.rotation;
		}
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
		root_motion: Option<RootMotionSettings<'_>>,
	) -> Result<Self, AnimationGraphPlayerError> {
		Self::with_target(graph, PlayerTargetSkeleton::Owned(target), root_motion)
	}
}

/// The `AnimationGraphPlayerError` enum reports invalid player inputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnimationGraphPlayerError {
	RootMotionNodeNotFound { name: String },
	DuplicateRootMotionNodeName { name: String },
	NegativeDelta,
}

impl fmt::Display for AnimationGraphPlayerError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::RootMotionNodeNotFound { name } => write!(
				formatter,
				"Animation root-motion node was not found. The most likely cause is that the target skeleton has no node named '{name}'."
			),
			Self::DuplicateRootMotionNodeName { name } => write!(
				formatter,
				"Animation root-motion node name is ambiguous. The most likely cause is that the target skeleton has more than one node named '{name}'."
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

/// Samples one loaded clip into target-skeleton local transforms using retained scratch buffers.
fn sample_target_pose(
	clip: &RuntimeClip,
	resident: &ResidentAnimationLease<'_>,
	target: &Skeleton,
	source_output: &mut Vec<LocalTransform>,
	target_output: &mut Vec<LocalTransform>,
) {
	resident
		.packed()
		.sample_local_pose(resident.skeleton(), clip.time_seconds, source_output);
	clip.pose_map
		.write_target_local_pose(source_output, target, target_output)
		.expect("animation pose maps are built from the source clip skeleton");
}

/// Reserves source-skeleton sampling storage when a clip becomes active, never during steady evaluation.
fn reserve_source_pose(clip: &RuntimeClip, output: &mut Vec<LocalTransform>) {
	let node_count = clip.source_node_count;
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
	resident: &ResidentAnimationLease<'_>,
	target: &Skeleton,
	loop_source: &mut Vec<LocalTransform>,
	loop_start: &mut Vec<LocalTransform>,
	loop_end: &mut Vec<LocalTransform>,
) -> RootMotionDelta {
	let Some(root_motion) = root_motion else {
		return RootMotionDelta::IDENTITY;
	};
	if advance.wrapped_loops == 0 {
		return object_space_root_delta(root_motion, target, previous, current);
	}

	// Sample the clip ends only for a loop crossing. This keeps the common
	// steady-state path to one clip sample while preserving forward root motion.
	resident
		.packed()
		.sample_local_pose(resident.skeleton(), clip.duration, loop_source);
	clip.pose_map
		.write_target_local_pose(loop_source, target, loop_end)
		.expect("animation pose maps are built from the source clip skeleton");
	resident.packed().sample_local_pose(resident.skeleton(), 0.0, loop_source);
	clip.pose_map
		.write_target_local_pose(loop_source, target, loop_start)
		.expect("animation pose maps are built from the source clip skeleton");

	let mut delta = object_space_root_delta(root_motion, target, previous, loop_end);
	let full_loop = object_space_root_delta(root_motion, target, loop_start, loop_end);
	for _ in 1..advance.wrapped_loops {
		delta = delta.then(full_loop);
	}
	delta.then(object_space_root_delta(root_motion, target, loop_start, current))
}

/// Calculates a root delta after converting both local poses into the owning object's space.
fn object_space_root_delta(
	root_motion: RootMotionTarget,
	target: &Skeleton,
	previous: &[LocalTransform],
	current: &[LocalTransform],
) -> RootMotionDelta {
	// Replace unselected channels with their reference values before hierarchy
	// composition. The selected local axes then inherit authored parent scale and
	// rotation while pose-only hip motion cannot leak into the owning object.
	let previous_root = extracted_root_transform(root_motion, previous[root_motion.node]);
	let current_root = extracted_root_transform(root_motion, current[root_motion.node]);
	RootMotionDelta::between(
		object_space_transform_with_node(target, previous, root_motion.node, previous_root),
		object_space_transform_with_node(target, current, root_motion.node, current_root),
	)
}

/// Keeps selected root-motion channels and restores all other channels to the reference pose.
fn extracted_root_transform(root_motion: RootMotionTarget, mut transform: LocalTransform) -> LocalTransform {
	for component in 0..3 {
		if !root_motion.translation.contains(component) {
			transform.translation[component] = root_motion.reference.translation[component];
		}
	}
	if root_motion.rotation == RootMotionRotation::None {
		transform.rotation = root_motion.reference.rotation;
	}
	transform
}

/// Composes a substituted node transform with its unchanged ancestors.
fn object_space_transform_with_node(
	skeleton: &Skeleton,
	local_pose: &[LocalTransform],
	node: usize,
	mut result: LocalTransform,
) -> LocalTransform {
	let mut parent = skeleton.nodes[node].parent;
	while let Some(parent_index) = parent {
		let parent_node = parent_index as usize;
		result = compose_local_transform(local_pose[parent_node], result);
		parent = skeleton.nodes[parent_node].parent;
	}
	result
}

/// Prepends `parent` to `child`, preserving the hierarchy's scale-rotate-translate order.
fn compose_local_transform(parent: LocalTransform, child: LocalTransform) -> LocalTransform {
	let scaled_translation = std::array::from_fn(|component| child.translation[component] * parent.scale[component]);
	let rotated_translation = rotate_vector(parent.rotation, scaled_translation);
	LocalTransform {
		translation: std::array::from_fn(|component| parent.translation[component] + rotated_translation[component]),
		rotation: multiply_quaternion(parent.rotation, child.rotation),
		scale: std::array::from_fn(|component| parent.scale[component] * child.scale[component]),
	}
}

/// Rotates one translation vector by a normalized quaternion without changing its magnitude.
fn rotate_vector([x, y, z, w]: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
	let quaternion_vector = [x, y, z];
	let twice_cross = [
		2.0 * (quaternion_vector[1] * vector[2] - quaternion_vector[2] * vector[1]),
		2.0 * (quaternion_vector[2] * vector[0] - quaternion_vector[0] * vector[2]),
		2.0 * (quaternion_vector[0] * vector[1] - quaternion_vector[1] * vector[0]),
	];
	let cross_again = [
		quaternion_vector[1] * twice_cross[2] - quaternion_vector[2] * twice_cross[1],
		quaternion_vector[2] * twice_cross[0] - quaternion_vector[0] * twice_cross[2],
		quaternion_vector[0] * twice_cross[1] - quaternion_vector[1] * twice_cross[0],
	];
	std::array::from_fn(|component| vector[component] + w * twice_cross[component] + cross_again[component])
}

#[cfg(test)]
mod tests {
	use std::{collections::VecDeque, num::NonZeroUsize};

	use resource_management::{
		resources::{
			animation::{Animation, NodeTrack, QuaternionCurve, Vector3Curve},
			skeleton::{LocalTransform, Skeleton, SkeletonNode},
		},
		Reference,
	};

	use super::{
		AnimationArenaRegion, AnimationClip, AnimationGraph, AnimationGraphBuildError, AnimationGraphPlayer, AnimationLease,
		AnimationPool, AnimationPoolConfig, AnimationPoolEvent, AnimationPoolRequest, AnimationTransition, PackedAnimationData,
		RootMotionRotation, RootMotionSettings, RootMotionTranslation,
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

	/// Measures the representation retained by the pool rather than the transient resource representation.
	fn packed_test_animation_bytes(name: &str, end_translation: f32) -> usize {
		PackedAnimationData::resident_bytes(&test_animation(name, end_translation))
	}

	fn pool(byte_budget: usize) -> AnimationPool {
		let (commands, _command_receiver) = kanal::bounded_async(super::ANIMATION_LOAD_QUEUE_CAPACITY);
		let (_completion_sender, completions) = kanal::bounded_async(super::ANIMATION_LOAD_QUEUE_CAPACITY);
		let word_capacity = byte_budget / std::mem::size_of::<u32>();
		AnimationPool {
			commands: commands.to_sync(),
			completions: completions.to_sync(),
			storage: vec![0; word_capacity].into_boxed_slice(),
			free_regions: vec![AnimationArenaRegion {
				offset: 0,
				word_count: word_capacity,
			}],
			cache: Vec::new(),
			pending: Vec::with_capacity(super::ANIMATION_LOAD_QUEUE_CAPACITY),
			failed: Vec::with_capacity(super::ANIMATION_LOAD_QUEUE_CAPACITY),
			blocked: Vec::with_capacity(super::ANIMATION_LOAD_QUEUE_CAPACITY),
			events: VecDeque::with_capacity(super::ANIMATION_POOL_EVENT_CAPACITY),
			byte_budget,
			resident_bytes: 0,
			next_use: std::cell::Cell::new(0),
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
	fn transition_states_complete_after_authored_exits_and_validate_their_configuration() {
		let mut builder = AnimationGraph::<bool>::builder();
		let idle = builder.state("idle", AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk", AnimationClip::looping("walk.animation"));
		let start_walk = builder.transition_state("start walk", AnimationClip::once("start.animation"), walk);
		builder.transition(start_walk, idle, AnimationTransition::when(|moving: &bool| !*moving));
		let graph = builder.build(idle).expect("transition state should be valid");

		assert_eq!(
			graph.state(start_walk).select_transition(&false, true),
			Some((idle, MediaTime::ZERO)),
			"authored cancellation must take priority over completion"
		);
		assert_eq!(
			graph.state(start_walk).select_transition(&true, true),
			Some((walk, MediaTime::ZERO)),
			"a finished transition state must fall through to its completion"
		);

		let mut looping_state = AnimationGraph::<()>::builder();
		let idle = looping_state.state("idle", AnimationClip::looping("idle.animation"));
		looping_state.transition_state("invalid", AnimationClip::looping("invalid.animation"), idle);
		assert!(matches!(
			looping_state.build(idle),
			Err(AnimationGraphBuildError::TransitionStateMustPlayOnce { state: 1 })
		));

		let mut missing_completion = AnimationGraph::<()>::builder();
		let idle = missing_completion.state("idle", AnimationClip::looping("idle.animation"));
		missing_completion.transition_state(
			"invalid",
			AnimationClip::once("invalid.animation"),
			super::AnimationStateId(2),
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
	fn pool_evicts_lru_entries_and_evaluation_leases_pin_arena_regions() {
		let idle = test_animation("idle", 1.0);
		let walk = test_animation("walk", 2.0);
		let budget = packed_test_animation_bytes("idle", 1.0).max(packed_test_animation_bytes("walk", 2.0));
		let mut first_pool = pool(budget);

		first_pool.admit("idle.animation".into(), idle);
		first_pool.admit("walk.animation".into(), walk);
		assert!(first_pool.cache.iter().any(|entry| entry.resource_id == "walk.animation"));
		assert!(first_pool.cache.iter().all(|entry| entry.resource_id != "idle.animation"));
		assert!(first_pool
			.drain_events()
			.any(|event| matches!(event, AnimationPoolEvent::Evicted { resource_id } if resource_id == "idle.animation")));
		let evicted_idle = AnimationLease::new("idle.animation");
		assert_eq!(first_pool.request(&evicted_idle), AnimationPoolRequest::Loading);

		let idle = test_animation("idle", 1.0);
		let walk = test_animation("walk", 2.0);
		let budget = packed_test_animation_bytes("idle", 1.0).max(packed_test_animation_bytes("walk", 2.0));
		let mut pool = pool(budget);
		pool.admit("idle.animation".into(), idle);
		let idle_lease = AnimationLease::new("idle.animation");
		let walk_lease = AnimationLease::new("walk.animation");
		assert_eq!(pool.request(&idle_lease), AnimationPoolRequest::Ready);
		let pinned = pool.acquire(&idle_lease).expect("expected cached idle animation");
		assert_eq!(pinned.entry.lease_count.get(), 1);
		drop(pinned);
		assert_eq!(pool.cache[0].lease_count.get(), 0);

		pool.admit("walk.animation".into(), walk);
		assert!(pool.cache.iter().all(|entry| entry.resource_id != "idle.animation"));
		assert_eq!(pool.request(&walk_lease), AnimationPoolRequest::Ready);
	}

	#[test]
	fn resident_clips_occupy_disjoint_ranges_of_one_preallocated_arena() {
		let clip_bytes = packed_test_animation_bytes("first", 1.0);
		let mut pool = pool(clip_bytes * 2);
		pool.admit("first.animation".into(), test_animation("first", 1.0));
		pool.admit("second.animation".into(), test_animation("second", 2.0));

		assert_eq!(pool.storage.len() * std::mem::size_of::<u32>(), clip_bytes * 2);
		assert_eq!(pool.cache.len(), 2);
		let first = pool.cache[0].region;
		let second = pool.cache[1].region;
		assert!(first.end() <= second.offset || second.end() <= first.offset);
	}

	#[test]
	fn oversized_clips_fail_once_until_the_caller_explicitly_retries_them() {
		let animation = test_animation("oversized", 1.0);
		let mut pool = pool(packed_test_animation_bytes("oversized", 1.0) - 1);
		pool.admit("oversized.animation".into(), animation);

		assert!(matches!(
			pool.request(&AnimationLease::new("oversized.animation")),
			AnimationPoolRequest::Failed
		));
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
		let byte_budget = packed_test_animation_bytes("idle", 1.0).saturating_add(packed_test_animation_bytes("run", 3.0));
		let mut pool = pool(byte_budget);
		pool.admit("idle.animation".into(), idle);
		pool.admit("run.animation".into(), run);

		let mut builder = AnimationGraph::<bool>::builder();
		let idle = builder.state("idle", AnimationClip::looping("idle.animation"));
		let run = builder.state("run", AnimationClip::looping("run.animation"));
		builder.transition(idle, run, AnimationTransition::when(|running| *running));
		let graph = builder.build(idle).expect("graph should build");
		let mut player =
			AnimationGraphPlayer::new(&graph, &target, Some(RootMotionSettings::full("root"))).expect("player should build");

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

	#[test]
	fn player_cancels_loading_transition_states_and_completes_loaded_ones() {
		let idle_animation = test_animation("idle", 0.0);
		let start_animation = test_animation("start", 1.0);
		let walk_animation = test_animation("walk", 2.0);
		let byte_budget = packed_test_animation_bytes("idle", 0.0)
			+ packed_test_animation_bytes("start", 1.0)
			+ packed_test_animation_bytes("walk", 2.0);
		let mut pool = pool(byte_budget);
		pool.admit("idle.animation".into(), idle_animation);

		let mut builder = AnimationGraph::<bool>::builder();
		let idle = builder.state("idle", AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk", AnimationClip::looping("walk.animation"));
		let start_walk = builder.transition_state("start walk", AnimationClip::once("start.animation"), walk);
		builder.transition(idle, start_walk, AnimationTransition::when(|moving| *moving));
		let graph = builder.build(idle).expect("graph should build");
		let target = test_skeleton();
		let mut player = AnimationGraphPlayer::new(&graph, &target, None).expect("player should build");

		player.advance(MediaTime::ZERO, &false, &mut pool).expect("initial idle pose");
		player
			.advance(MediaTime::ZERO, &true, &mut pool)
			.expect("queues start-walk clip");
		assert_eq!(player.state(), Some(idle));
		player
			.advance(MediaTime::ZERO, &false, &mut pool)
			.expect("cancels stale start-walk request");
		assert_eq!(player.state(), Some(idle));

		pool.admit("start.animation".into(), start_animation);
		pool.admit("walk.animation".into(), walk_animation);
		player
			.advance(MediaTime::ZERO, &true, &mut pool)
			.expect("starts transition state");
		assert_eq!(player.state(), Some(start_walk));
		player
			.advance(MediaTime::from_seconds(1), &true, &mut pool)
			.expect("finishes transition-state clip");
		assert_eq!(player.state(), Some(start_walk));
		player
			.advance(MediaTime::ZERO, &true, &mut pool)
			.expect("enters completion state");
		assert_eq!(player.state(), Some(walk));
	}

	#[test]
	fn player_requires_one_uniquely_named_root_motion_node() {
		let mut builder = AnimationGraph::<()>::builder();
		let state = builder.state("idle", AnimationClip::looping("idle.animation"));
		let graph = builder.build(state).expect("graph should build");
		let target = test_skeleton();

		assert!(matches!(
			AnimationGraphPlayer::new(&graph, &target, Some(RootMotionSettings::full("Hips"))),
			Err(super::AnimationGraphPlayerError::RootMotionNodeNotFound { name }) if name == "Hips"
		));

		let duplicate_target = Skeleton {
			nodes: vec![
				SkeletonNode {
					name: Some("Hips".into()),
					parent: None,
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("Hips".into()),
					parent: Some(0),
					rest_local: LocalTransform::identity(),
				},
			],
		};
		assert!(matches!(
			AnimationGraphPlayer::new(&graph, &duplicate_target, Some(RootMotionSettings::full("Hips"))),
			Err(super::AnimationGraphPlayerError::DuplicateRootMotionNodeName { name }) if name == "Hips"
		));
	}

	#[test]
	fn player_selectively_extracts_object_space_root_motion_across_a_remapped_scaled_loop() {
		let root_rotation = crate::animation::math::quaternion_exp([0.0, std::f32::consts::FRAC_PI_2, 0.0]);
		let source = Skeleton {
			nodes: vec![
				SkeletonNode {
					name: Some("Root".into()),
					parent: None,
					rest_local: LocalTransform {
						rotation: root_rotation,
						scale: [0.01; 3],
						..LocalTransform::identity()
					},
				},
				SkeletonNode {
					name: Some("Hips".into()),
					parent: Some(0),
					rest_local: LocalTransform {
						translation: [0.0, 100.0, 0.0],
						..LocalTransform::identity()
					},
				},
			],
		};
		// The target inserts helper nodes before Hips, matching FBX rigs whose
		// compatible named joints do not share source indices.
		let target = Skeleton {
			nodes: vec![
				SkeletonNode {
					name: None,
					parent: None,
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("Root".into()),
					parent: Some(0),
					rest_local: LocalTransform {
						rotation: root_rotation,
						scale: [0.01; 3],
						..LocalTransform::identity()
					},
				},
				SkeletonNode {
					name: Some("IK Helper".into()),
					parent: Some(1),
					rest_local: LocalTransform::identity(),
				},
				SkeletonNode {
					name: Some("Hips".into()),
					parent: Some(1),
					rest_local: LocalTransform {
						translation: [0.0, 100.0, 0.0],
						..LocalTransform::identity()
					},
				},
			],
		};
		let animation = Animation {
			name: Some("walk".into()),
			skeleton: Reference::in_memory("scaled.skeleton", source),
			duration: 1.0,
			tracks: vec![NodeTrack {
				node: 1,
				translation: Some(Vector3Curve::Linear {
					times: vec![0.0, 1.0],
					values: vec![[0.0, 100.0, 0.0], [20.0, 110.0, -100.0]],
				}),
				rotation: Some(QuaternionCurve::Linear {
					times: vec![0.0, 1.0],
					values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.382_683_43, 0.0, 0.923_879_5]],
				}),
				scale: None,
			}],
		};
		let mut pool = pool(animation.estimated_resident_bytes());
		pool.admit("walk.animation".into(), animation);
		let mut builder = AnimationGraph::<()>::builder();
		let walk = builder.state("walk", AnimationClip::looping("walk.animation"));
		let graph = builder.build(walk).expect("graph should build");
		let mut player = AnimationGraphPlayer::new(
			&graph,
			&target,
			Some(RootMotionSettings {
				node_name: "Hips",
				translation: RootMotionTranslation::Z,
				rotation: RootMotionRotation::None,
			}),
		)
		.expect("player should build");

		let initial = player.advance(MediaTime::ZERO, &(), &mut pool).expect("initial pose");
		assert_eq!(initial.root_motion().translation, [0.0; 3]);
		let first = player
			.advance(MediaTime::from_millis(750), &(), &mut pool)
			.expect("walk pose");
		math::assert_float_eq!(first.root_motion().translation[0], -0.75);
		math::assert_float_eq!(first.root_motion().translation[1], 0.0);
		math::assert_float_eq!(first.root_motion().translation[2], 0.0);
		assert_eq!(first.local_pose()[3].translation, [15.0, 107.5, 0.0]);
		assert_ne!(first.local_pose()[3].rotation, LocalTransform::identity().rotation);

		let wrapped = player
			.advance(MediaTime::from_millis(500), &(), &mut pool)
			.expect("wrapped walk pose");
		math::assert_float_eq!(wrapped.root_motion().translation[0], -0.5);
		math::assert_float_eq!(wrapped.root_motion().translation[1], 0.0);
		math::assert_float_eq!(wrapped.root_motion().translation[2], 0.0);
		assert_eq!(wrapped.local_pose()[3].translation, [5.0, 102.5, 0.0]);
		assert_ne!(wrapped.local_pose()[3].rotation, LocalTransform::identity().rotation);
	}
}
