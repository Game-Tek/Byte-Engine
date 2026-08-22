//! Code-authored skeletal animation state machines and asynchronous clip pooling.
//!
//! Build an [`AnimationGraph`] with [`AnimationGraphBuilder`], then create an
//! [`AnimationGraphPlayer`] for each animated skeleton. The player evaluates
//! synchronously from retained pose buffers while [`AnimationPool`] loads clip
//! resources on an application-owned async worker. Client code selects one
//! durable [`AnimationStateId`] with normal Rust control flow, then the player
//! reconciles its current playback toward that requested state. Connect state
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
//!    authored [`AnimationStateId`], call [`AnimationGraphPlayer::advance`],
//!    apply its root motion to the owning object, and send
//!    [`AnimationGraphPose::global_pose`] to rendering.
//!
//! The graph player owns its evaluation buffers. The renderer's `UpdatePose`
//! message owns its matrix vector, so copying
//! at that cross-system boundary is currently intentional. See
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

/// The `AnimationLease` struct keeps a stable clip identity across arena residency and eviction.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AnimationLease {
	resource_id: Arc<str>,
}

impl AnimationLease {
	/// Creates a lease handle for a clip that the pool may load, evict, and load again.
	pub fn new(resource_id: impl Into<Arc<str>>) -> Self {
		Self {
			resource_id: resource_id.into(),
		}
	}

	/// Returns the resource ID used when an evicted lease needs another asynchronous load.
	pub fn resource_id(&self) -> &str {
		self.resource_id.as_ref()
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
	pub fn looping(resource_id: impl Into<Arc<str>>) -> Self {
		Self {
			lease: AnimationLease::new(resource_id),
			playback: AnimationPlayback::Loop,
		}
	}

	/// Creates a clip that holds its final sample after reaching its duration.
	pub fn once(resource_id: impl Into<Arc<str>>) -> Self {
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

/// The `AnimationTransition` struct configures how the player blends toward a requested state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnimationTransition {
	duration: MediaTime,
}

impl AnimationTransition {
	/// Creates an immediate transition toward a requested state.
	pub const fn new() -> Self {
		Self {
			duration: MediaTime::ZERO,
		}
	}

	/// Smooths the transition with critically damped inertialization for the supplied duration.
	pub const fn inertialize(mut self, duration: MediaTime) -> Self {
		self.duration = duration;
		self
	}
}

impl Default for AnimationTransition {
	fn default() -> Self {
		Self::new()
	}
}

struct StateTransition {
	// `target` may be an internal one-shot state, while `destination` is the
	// persistent state requested by client code.
	target: AnimationStateId,
	destination: AnimationStateId,
	transition: AnimationTransition,
	// Only `.anytime` edges can interrupt an active transition clip.
	can_interrupt_transition: bool,
}

/// Distinguishes persistent clips from one-shot clips that complete into another state.
enum AnimationGraphStateKind {
	Persistent,
	Transition { completion: AnimationStateId },
}

struct AnimationGraphStateData {
	name: String,
	clip: AnimationClip,
	kind: AnimationGraphStateKind,
	transitions: Vec<StateTransition>,
}

impl AnimationGraphStateData {
	/// Returns the fallback target entered after a transition-state clip finishes.
	fn completion_target(&self) -> Option<AnimationStateId> {
		match self.kind {
			AnimationGraphStateKind::Persistent => None,
			AnimationGraphStateKind::Transition { completion } => Some(completion),
		}
	}

	/// Selects the first authored route whose persistent destination was requested.
	fn select_authored_transition(&self, requested: AnimationStateId) -> Option<(AnimationStateId, MediaTime)> {
		self.transitions
			.iter()
			.find(|transition| transition.destination == requested)
			.map(|transition| (transition.target, transition.transition.duration))
	}

	/// Selects the first interruptible route whose persistent destination was requested.
	fn select_anytime_transition(&self, requested: AnimationStateId) -> Option<(AnimationStateId, MediaTime)> {
		self.transitions
			.iter()
			.find(|transition| transition.can_interrupt_transition && transition.destination == requested)
			.map(|transition| (transition.target, transition.transition.duration))
	}
}

/// The `AnimationGraph` struct stores immutable routes used to reconcile requested animation states.
///
/// Build the graph once with [`AnimationGraphBuilder`], then share it between
/// any number of [`AnimationGraphPlayer`] instances. Client code selects one of
/// the graph's durable [`AnimationStateId`] values before each evaluation.
pub struct AnimationGraph {
	id: u64,
	states: Vec<AnimationGraphStateData>,
	initial: AnimationStateId,
}

impl AnimationGraph {
	/// Starts a builder for an animation reconciliation graph.
	pub fn builder() -> AnimationGraphBuilder {
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
	) -> Option<(AnimationStateId, MediaTime)> {
		let source_state = self.state(source);
		if source_state.completion_target().is_none() {
			return if source == requested {
				None
			} else {
				source_state.select_authored_transition(requested)
			};
		}

		if let Some(transition) = source_state.select_authored_transition(requested) {
			return Some(transition);
		}

		let completion = source_state
			.completion_target()
			.expect("transition state completion was checked above");
		if requested != completion {
			if let Some(transition) = self.state(completion).select_anytime_transition(requested) {
				return Some(transition);
			}
		}

		source_finished.then_some((completion, MediaTime::ZERO))
	}
}

struct PendingStateTransition {
	source: AnimationStateId,
	target: AnimationStateId,
	destination: AnimationStateId,
	transition: AnimationTransition,
	// This value becomes `StateTransition::can_interrupt_transition` during graph construction.
	can_interrupt_transition: bool,
}

/// The `AnimationGraphBuilder` struct assembles named clip states and their ordered reconciliation routes.
pub struct AnimationGraphBuilder {
	graph: u64,
	data: RefCell<Option<AnimationGraphBuilderData>>,
}

struct AnimationGraphBuilderData {
	states: Vec<AnimationGraphStateData>,
	transitions: Vec<PendingStateTransition>,
}

/// The `AnimationGraphStateBuilder` struct assigns a clip to a named graph state.
pub struct AnimationGraphStateBuilder<'builder> {
	graph: u64,
	data: &'builder RefCell<Option<AnimationGraphBuilderData>>,
	name: String,
}

/// The `AnimationGraphState` struct connects a state to other states during graph authoring.
pub struct AnimationGraphState<'builder> {
	data: &'builder RefCell<Option<AnimationGraphBuilderData>>,
	id: AnimationStateId,
}

impl Clone for AnimationGraphState<'_> {
	fn clone(&self) -> Self {
		*self
	}
}

impl Copy for AnimationGraphState<'_> {}

/// The `AnimationGraphTransitionBuilder` struct assigns a clip to a route between two states.
pub struct AnimationGraphTransitionBuilder<'builder> {
	source: AnimationGraphState<'builder>,
	target: AnimationGraphState<'builder>,
}

/// The `AnimationGraphTransitionConditionBuilder` struct holds unfinished one-shot route authoring so callers can choose its interruption policy.
pub struct AnimationGraphTransitionConditionBuilder<'builder> {
	source: AnimationGraphState<'builder>,
	target: AnimationGraphState<'builder>,
	clip: AnimationClip,
}

impl<'builder> AnimationGraphStateBuilder<'builder> {
	/// Assigns the clip played while this state is active.
	pub fn with(self, clip: AnimationClip) -> AnimationGraphState<'builder> {
		let mut builder = self.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		let id = AnimationStateId {
			graph: self.graph,
			index: builder.states.len(),
		};
		builder.states.push(AnimationGraphStateData {
			name: self.name,
			clip,
			kind: AnimationGraphStateKind::Persistent,
			transitions: Vec::new(),
		});
		AnimationGraphState { data: self.data, id }
	}
}

impl<'builder> AnimationGraphState<'builder> {
	/// Returns the durable state ID that client code can request after the graph is built.
	pub const fn id(self) -> AnimationStateId {
		self.id
	}

	/// Starts an authored route that reconciles toward `target`.
	pub fn to(self, target: Self) -> AnimationGraphTransitionBuilder<'builder> {
		assert!(
			std::ptr::eq(self.data, target.data),
			"animation graph states must use the same builder"
		);
		AnimationGraphTransitionBuilder { source: self, target }
	}
}

impl<'builder> AnimationGraphTransitionBuilder<'builder> {
	/// Adds a direct route without playing an intermediate clip.
	pub fn when(self, transition: AnimationTransition) -> AnimationGraphState<'builder> {
		let mut builder = self.source.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		builder.transitions.push(PendingStateTransition {
			source: self.source.id,
			target: self.target.id,
			destination: self.target.id,
			transition,
			can_interrupt_transition: false,
		});
		self.target
	}

	/// Assigns the one-shot clip played between the source and target states.
	pub fn with(self, clip: AnimationClip) -> AnimationGraphTransitionConditionBuilder<'builder> {
		AnimationGraphTransitionConditionBuilder {
			source: self.source,
			target: self.target,
			clip,
		}
	}
}

impl<'builder> AnimationGraphTransitionConditionBuilder<'builder> {
	/// Adds a one-shot route that must complete before reconciling another request.
	pub fn when(self, transition: AnimationTransition) -> AnimationGraphState<'builder> {
		self.add(transition, false)
	}

	/// Adds a one-shot route that can reconcile another request before it completes.
	pub fn anytime(self, transition: AnimationTransition) -> AnimationGraphState<'builder> {
		self.add(transition, true)
	}

	/// Adds the internal one-shot state and its source route with the selected interruption policy.
	fn add(self, transition: AnimationTransition, can_interrupt_transition: bool) -> AnimationGraphState<'builder> {
		let mut builder = self.source.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		let id = AnimationStateId {
			graph: self.source.id.graph,
			index: builder.states.len(),
		};
		let name = format!(
			"{} -> {} [{}]",
			builder.states[self.source.id.index].name, builder.states[self.target.id.index].name, id.index,
		);
		builder.states.push(AnimationGraphStateData {
			name,
			clip: self.clip,
			kind: AnimationGraphStateKind::Transition {
				completion: self.target.id,
			},
			transitions: Vec::new(),
		});
		builder.transitions.push(PendingStateTransition {
			source: self.source.id,
			target: id,
			destination: self.target.id,
			transition,
			can_interrupt_transition,
		});
		AnimationGraphState {
			data: self.source.data,
			id,
		}
	}
}

impl Default for AnimationGraphBuilder {
	fn default() -> Self {
		Self::new()
	}
}

impl AnimationGraphBuilder {
	/// Creates an empty animation graph builder.
	pub fn new() -> Self {
		static NEXT_GRAPH_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
		let graph = NEXT_GRAPH_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		assert_ne!(graph, 0, "animation graph identity space is exhausted");
		Self {
			graph,
			data: RefCell::new(Some(AnimationGraphBuilderData {
				states: Vec::new(),
				transitions: Vec::new(),
			})),
		}
	}

	/// Names a state. Call [`AnimationGraphStateBuilder::with`] next to assign its clip.
	pub fn state(&self, name: impl Into<String>) -> AnimationGraphStateBuilder<'_> {
		AnimationGraphStateBuilder {
			graph: self.graph,
			data: &self.data,
			name: name.into(),
		}
	}

	/// Adds an internal one-shot state that falls through to `completion` after it finishes.
	fn transition_state(&self, name: impl Into<String>, clip: AnimationClip, completion: AnimationStateId) -> AnimationStateId {
		let mut data = self.data.borrow_mut();
		let data = data.as_mut().expect("the animation graph has already been built");
		let id = AnimationStateId {
			graph: self.graph,
			index: data.states.len(),
		};
		data.states.push(AnimationGraphStateData {
			name: name.into(),
			clip,
			kind: AnimationGraphStateKind::Transition { completion },
			transitions: Vec::new(),
		});
		id
	}

	/// Validates the graph and selects the initial state.
	pub fn build(&self, initial: AnimationGraphState<'_>) -> Result<AnimationGraph, AnimationGraphBuildError> {
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
			if state.clip.resource_id().trim().is_empty() {
				return Err(AnimationGraphBuildError::EmptyResourceId { state: state_index });
			}
			if data.states[..state_index].iter().any(|other| other.name == state.name) {
				return Err(AnimationGraphBuildError::DuplicateStateName {
					name: state.name.clone(),
				});
			}
			if let AnimationGraphStateKind::Transition { completion } = state.kind {
				if state.clip.playback() != AnimationPlayback::Once {
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

		for pending in data.transitions {
			if pending.source.graph != self.graph || pending.source.index >= data.states.len() {
				return Err(AnimationGraphBuildError::TransitionSourceOutOfRange {
					state: pending.source.index,
					state_count: data.states.len(),
				});
			}
			if pending.target.graph != self.graph || pending.target.index >= data.states.len() {
				return Err(AnimationGraphBuildError::TransitionTargetOutOfRange {
					state: pending.target.index,
					state_count: data.states.len(),
				});
			}
			if pending.transition.duration < MediaTime::ZERO {
				return Err(AnimationGraphBuildError::NegativeTransitionDuration);
			}
			data.states[pending.source.index].transitions.push(StateTransition {
				target: pending.target,
				destination: pending.destination,
				transition: pending.transition,
				can_interrupt_transition: pending.can_interrupt_transition,
			});
		}

		Ok(AnimationGraph {
			id: self.graph,
			states: data.states,
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

mod runtime;

use std::{
	cell::RefCell,
	collections::{HashMap, VecDeque},
	fmt,
	num::NonZeroUsize,
	sync::Arc,
};

use math::Matrix;
use resource_management::{
	resource::resource_manager::ResourceManager,
	resources::{
		animation::Animation,
		skeleton::{LocalTransform, Skeleton, SkeletonPoseMap},
	},
	Reference,
};
#[doc(hidden)]
pub use runtime::benchmarks;
pub use runtime::{
	AnimationEvaluation, AnimationGraphPlayer, AnimationGraphPlayerError, AnimationGraphPose, AnimationLoadWorker,
	AnimationPool, AnimationPoolConfig, AnimationPoolEvent, AnimationPoolRequest, RootMotionRotation, RootMotionSettings,
	RootMotionTranslation,
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn builder_preserves_transition_order_and_rejects_invalid_graphs() {
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk).when(AnimationTransition::new());
		idle.to(idle)
			.with(AnimationClip::once("restart.animation"))
			.when(AnimationTransition::new().inertialize(MediaTime::from_millis(100)));
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
			.anytime(AnimationTransition::new());
		let stop_walk = walk
			.to(idle)
			.with(AnimationClip::once("stop.animation"))
			.anytime(AnimationTransition::new());
		let graph = builder.build(idle).expect("transition state should be valid");

		assert_eq!(
			graph.select_transition(start_walk.id, idle.id, false),
			Some((stop_walk.id, MediaTime::ZERO)),
			"a matching any-time transition must interrupt an active transition clip"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, walk.id, false),
			None,
			"the active any-time transition must not restart itself"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, walk.id, true),
			Some((walk.id, MediaTime::ZERO)),
			"an uninterrupted transition state must fall through to its completion"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, idle.id, true),
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
	fn transition_states_prioritize_authored_exits_over_completion() {
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let start_walk = idle
			.to(walk)
			.with(AnimationClip::once("start.animation"))
			.when(AnimationTransition::new());
		start_walk.to(idle).when(AnimationTransition::new());
		let graph = builder.build(idle).expect("transition state should be valid");

		assert_eq!(
			graph.select_transition(start_walk.id, idle.id, true),
			Some((idle.id, MediaTime::ZERO)),
			"an authored exit must take priority over transition-state completion"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, walk.id, true),
			Some((walk.id, MediaTime::ZERO)),
			"a transition state without a matching exit must complete normally"
		);
	}
}
