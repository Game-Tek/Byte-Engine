//! Code-authored skeletal animation state machines and asynchronous clip pooling.
//!
//! Build an [`AnimationGraph`] with [`AnimationGraphBuilder`], then create an
//! [`AnimationGraphPlayer`] for each animated skeleton. The player evaluates
//! synchronously from retained pose buffers while [`AnimationPool`] loads clip
//! resources on an application-owned async worker. Connect state handles with
//! [`AnimationGraphState::to`], then assign the one-shot clip and choose
//! [`AnimationGraphTransitionConditionBuilder::when`] or
//! [`AnimationGraphTransitionConditionBuilder::anytime`] as its trigger policy.
//! The clip completes into the target state unless another any-time transition
//! interrupts it.
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
//! 4. Each application tick, call [`AnimationPool::update`] once, create typed
//!    input, call [`AnimationGraphPlayer::advance`], apply its root motion to
//!    the owning object, and send [`AnimationGraphPose::global_pose`] to
//!    rendering.
//!
//! The graph player owns its evaluation buffers. The existing
//! [`crate::rendering::UpdatePose`] message owns its matrix vector, so copying
//! at that cross-system boundary is currently intentional. See
//! `crates/byte-engine/examples/animation_graph.rs` for the complete headed
//! application sequence.

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
	// Only `.anytime` edges can interrupt an active transition clip.
	can_interrupt_transition: bool,
}

/// Distinguishes persistent clips from one-shot clips that complete into another state.
enum AnimationGraphStateKind {
	Persistent,
	Transition { completion: AnimationStateId },
}

struct AnimationGraphStateData<I> {
	name: String,
	clip: AnimationClip,
	kind: AnimationGraphStateKind,
	transitions: Vec<StateTransition<I>>,
}

impl<I> AnimationGraphStateData<I> {
	/// Returns the fallback target entered after a transition-state clip finishes.
	fn completion_target(&self) -> Option<AnimationStateId> {
		match self.kind {
			AnimationGraphStateKind::Persistent => None,
			AnimationGraphStateKind::Transition { completion } => Some(completion),
		}
	}

	/// Selects the first authored exit that matches the active state's input and playback state.
	fn select_authored_transition(&self, input: &I, source_finished: bool) -> Option<(AnimationStateId, MediaTime)> {
		self.transitions
			.iter()
			.find(|transition| transition.transition.matches(input, source_finished))
			.map(|transition| (transition.target, transition.transition.duration))
	}

	/// Selects the first any-time exit that can interrupt another active transition clip.
	fn select_anytime_transition(&self, input: &I) -> Option<(AnimationStateId, MediaTime)> {
		self.transitions
			.iter()
			.find(|transition| transition.can_interrupt_transition && transition.transition.matches(input, false))
			.map(|transition| (transition.target, transition.transition.duration))
	}
}

/// The `AnimationGraph` struct stores an immutable, typed animation state machine.
///
/// Build the graph once with [`AnimationGraphBuilder`], then share it between
/// any number of [`AnimationGraphPlayer`] instances that use the same input type.
pub struct AnimationGraph<I> {
	states: Vec<AnimationGraphStateData<I>>,
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

	fn state(&self, id: AnimationStateId) -> &AnimationGraphStateData<I> {
		// Builder validation keeps every runtime state ID in range.
		&self.states[id.0]
	}

	/// Selects a state-specific exit, then an interrupting exit from a transition clip's completion state.
	fn select_transition(
		&self,
		source: AnimationStateId,
		input: &I,
		source_finished: bool,
	) -> Option<(AnimationStateId, MediaTime)> {
		let source_state = self.state(source);
		if let Some(transition) = source_state.select_authored_transition(input, source_finished) {
			return Some(transition);
		}

		let completion = source_state.completion_target()?;
		if let Some(transition) = self.state(completion).select_anytime_transition(input) {
			return Some(transition);
		}

		source_finished.then_some((completion, MediaTime::ZERO))
	}
}

struct PendingStateTransition<I> {
	source: AnimationStateId,
	target: AnimationStateId,
	transition: AnimationTransition<I>,
	// This value becomes `StateTransition::can_interrupt_transition` during graph construction.
	can_interrupt_transition: bool,
}

/// The `AnimationGraphBuilder` struct assembles named clip states and their ordered transitions.
pub struct AnimationGraphBuilder<I> {
	data: RefCell<Option<AnimationGraphBuilderData<I>>>,
}

struct AnimationGraphBuilderData<I> {
	states: Vec<AnimationGraphStateData<I>>,
	transitions: Vec<PendingStateTransition<I>>,
}

/// The `AnimationGraphStateBuilder` struct assigns a clip to a named graph state.
pub struct AnimationGraphStateBuilder<'builder, I> {
	data: &'builder RefCell<Option<AnimationGraphBuilderData<I>>>,
	name: String,
}

/// The `AnimationGraphState` struct connects a state to other states during graph authoring.
pub struct AnimationGraphState<'builder, I> {
	data: &'builder RefCell<Option<AnimationGraphBuilderData<I>>>,
	id: AnimationStateId,
}

impl<I> Clone for AnimationGraphState<'_, I> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<I> Copy for AnimationGraphState<'_, I> {}

/// The `AnimationGraphTransitionBuilder` struct assigns a clip to a transition between two states.
pub struct AnimationGraphTransitionBuilder<'builder, I> {
	source: AnimationGraphState<'builder, I>,
	target: AnimationGraphState<'builder, I>,
}

/// The `AnimationGraphTransitionConditionBuilder` struct holds unfinished one-shot transition authoring so callers can choose its trigger policy.
pub struct AnimationGraphTransitionConditionBuilder<'builder, I> {
	source: AnimationGraphState<'builder, I>,
	target: AnimationGraphState<'builder, I>,
	clip: AnimationClip,
}

impl<'builder, I> AnimationGraphStateBuilder<'builder, I> {
	/// Assigns the clip played while this state is active.
	pub fn with(self, clip: AnimationClip) -> AnimationGraphState<'builder, I> {
		let mut builder = self.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		let id = AnimationStateId(builder.states.len());
		builder.states.push(AnimationGraphStateData {
			name: self.name,
			clip,
			kind: AnimationGraphStateKind::Persistent,
			transitions: Vec::new(),
		});
		AnimationGraphState { data: self.data, id }
	}
}

impl<'builder, I> AnimationGraphState<'builder, I> {
	/// Starts an authored transition that completes in `target`.
	pub fn to(self, target: Self) -> AnimationGraphTransitionBuilder<'builder, I> {
		assert!(
			std::ptr::eq(self.data, target.data),
			"animation graph states must use the same builder"
		);
		AnimationGraphTransitionBuilder { source: self, target }
	}
}

impl<'builder, I> AnimationGraphTransitionBuilder<'builder, I> {
	/// Adds a direct transition without playing an intermediate clip.
	///
	/// The returned handle is the target state, so it can start another fluent
	/// transition when that keeps the graph definition easier to read.
	pub fn when(self, transition: AnimationTransition<I>) -> AnimationGraphState<'builder, I> {
		let mut builder = self.source.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		builder.transitions.push(PendingStateTransition {
			source: self.source.id,
			target: self.target.id,
			transition,
			can_interrupt_transition: false,
		});
		self.target
	}

	/// Assigns the one-shot clip played between the source and target states.
	pub fn with(self, clip: AnimationClip) -> AnimationGraphTransitionConditionBuilder<'builder, I> {
		AnimationGraphTransitionConditionBuilder {
			source: self.source,
			target: self.target,
			clip,
		}
	}
}

impl<'builder, I> AnimationGraphTransitionConditionBuilder<'builder, I> {
	/// Adds a transition that can begin only while its source state is active.
	///
	/// The transition clip runs through to its target unless it has an authored
	/// exit. Use [`Self::anytime`] when another transition can interrupt the clip.
	pub fn when(self, transition: AnimationTransition<I>) -> AnimationGraphState<'builder, I> {
		self.add(transition, false)
	}

	/// Adds a transition that can interrupt another active transition clip.
	///
	/// While this clip is active, matching any-time transitions authored from its
	/// completion state take priority over its normal completion. Use [`Self::when`]
	/// when the clip must run through to its target.
	pub fn anytime(self, transition: AnimationTransition<I>) -> AnimationGraphState<'builder, I> {
		self.add(transition, true)
	}

	/// Adds the one-shot state and its source edge with the selected interruption policy.
	fn add(self, transition: AnimationTransition<I>, can_interrupt_transition: bool) -> AnimationGraphState<'builder, I> {
		let mut builder = self.source.data.borrow_mut();
		let builder = builder.as_mut().expect("the animation graph has already been built");
		let id = AnimationStateId(builder.states.len());
		let name = format!(
			"{} -> {} [{id}]",
			builder.states[self.source.id.0].name,
			builder.states[self.target.id.0].name,
			id = id.0,
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
			transition,
			can_interrupt_transition,
		});
		AnimationGraphState {
			data: self.source.data,
			id,
		}
	}
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
			data: RefCell::new(Some(AnimationGraphBuilderData {
				states: Vec::new(),
				transitions: Vec::new(),
			})),
		}
	}

	/// Names a state. Call [`AnimationGraphStateBuilder::with`] next to assign its clip.
	pub fn state(&self, name: impl Into<String>) -> AnimationGraphStateBuilder<'_, I> {
		AnimationGraphStateBuilder {
			data: &self.data,
			name: name.into(),
		}
	}

	/// Adds a one-shot state that falls through to `completion` after it finishes.
	///
	/// Use this for authored movement starts, stops, turns, and other clips that
	/// bridge states. `clip` must use [`AnimationPlayback::Once`]. Authored
	/// transitions from this state run before its completion, so they can cancel
	/// or redirect the transient animation.
	fn transition_state(&self, name: impl Into<String>, clip: AnimationClip, completion: AnimationStateId) -> AnimationStateId {
		let mut data = self.data.borrow_mut();
		let data = data.as_mut().expect("the animation graph has already been built");
		let id = AnimationStateId(data.states.len());
		data.states.push(AnimationGraphStateData {
			name: name.into(),
			clip,
			kind: AnimationGraphStateKind::Transition { completion },
			transitions: Vec::new(),
		});
		id
	}

	/// Validates the graph and selects the initial state.
	pub fn build(&self, initial: AnimationGraphState<'_, I>) -> Result<AnimationGraph<I>, AnimationGraphBuildError> {
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
		if initial.0 >= data.states.len() {
			return Err(AnimationGraphBuildError::InitialStateOutOfRange {
				state: initial.0,
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
				if completion.0 >= data.states.len() {
					return Err(AnimationGraphBuildError::TransitionStateCompletionOutOfRange {
						state: state_index,
						completion: completion.0,
						state_count: data.states.len(),
					});
				}
			}
		}

		for pending in data.transitions {
			if pending.source.0 >= data.states.len() {
				return Err(AnimationGraphBuildError::TransitionSourceOutOfRange {
					state: pending.source.0,
					state_count: data.states.len(),
				});
			}
			if pending.target.0 >= data.states.len() {
				return Err(AnimationGraphBuildError::TransitionTargetOutOfRange {
					state: pending.target.0,
					state_count: data.states.len(),
				});
			}
			if pending.transition.duration < MediaTime::ZERO {
				return Err(AnimationGraphBuildError::NegativeTransitionDuration);
			}
			data.states[pending.source.0].transitions.push(StateTransition {
				target: pending.target,
				transition: pending.transition,
				can_interrupt_transition: pending.can_interrupt_transition,
			});
		}

		Ok(AnimationGraph {
			states: data.states,
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

mod runtime;

use std::{
	cell::RefCell,
	collections::{HashMap, VecDeque},
	fmt,
	num::NonZeroUsize,
	ops::Deref,
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
	AnimationGraphPlayer, AnimationGraphPlayerError, AnimationGraphPose, AnimationLoadWorker, AnimationPool,
	AnimationPoolConfig, AnimationPoolEvent, AnimationPoolRequest, RootMotionRotation, RootMotionSettings,
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
		let builder = AnimationGraph::<bool>::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk).when(AnimationTransition::when(|input| *input));
		idle.to(idle)
			.with(AnimationClip::once("restart.animation"))
			.when(AnimationTransition::always().inertialize(MediaTime::from_millis(100)));
		let graph = builder.build(idle).expect("expected graph value");

		assert_eq!(graph.state_count(), 3);
		assert_eq!(graph.state(idle.id).transitions.len(), 2);

		let invalid = AnimationGraph::<()>::builder();
		let state = invalid.state("").with(AnimationClip::once("clip.animation"));

		assert!(matches!(
			invalid.build(state),
			Err(AnimationGraphBuildError::EmptyStateName { state: 0 })
		));
	}

	#[test]
	fn anytime_transition_states_interrupt_toward_their_completion_state() {
		let builder = AnimationGraph::<bool>::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let start_walk = idle
			.to(walk)
			.with(AnimationClip::once("start.animation"))
			.anytime(AnimationTransition::when(|moving| *moving));
		let stop_walk = walk
			.to(idle)
			.with(AnimationClip::once("stop.animation"))
			.anytime(AnimationTransition::when(|moving: &bool| !*moving));
		let graph = builder.build(idle).expect("transition state should be valid");

		assert_eq!(
			graph.select_transition(start_walk.id, &false, false),
			Some((stop_walk.id, MediaTime::ZERO)),
			"a matching any-time transition must interrupt an active transition clip"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, &true, false),
			None,
			"the active any-time transition must not restart itself"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, &true, true),
			Some((walk.id, MediaTime::ZERO)),
			"an uninterrupted transition state must fall through to its completion"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, &false, true),
			Some((stop_walk.id, MediaTime::ZERO)),
			"a matching any-time transition must take priority over completion"
		);

		let looping_state = AnimationGraph::<()>::builder();
		let idle = looping_state.state("idle").with(AnimationClip::looping("idle.animation"));
		looping_state.transition_state("invalid", AnimationClip::looping("invalid.animation"), idle.id);

		assert!(matches!(
			looping_state.build(idle),
			Err(AnimationGraphBuildError::TransitionStateMustPlayOnce { state: 1 })
		));

		let missing_completion = AnimationGraph::<()>::builder();
		let idle = missing_completion
			.state("idle")
			.with(AnimationClip::looping("idle.animation"));
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
	fn transition_states_prioritize_authored_exits_over_completion() {
		let builder = AnimationGraph::<bool>::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		let start_walk = idle
			.to(walk)
			.with(AnimationClip::once("start.animation"))
			.when(AnimationTransition::always());
		start_walk.to(idle).when(AnimationTransition::when(|moving: &bool| !*moving));
		let graph = builder.build(idle).expect("transition state should be valid");

		assert_eq!(
			graph.select_transition(start_walk.id, &false, true),
			Some((idle.id, MediaTime::ZERO)),
			"an authored exit must take priority over transition-state completion"
		);
		assert_eq!(
			graph.select_transition(start_walk.id, &true, true),
			Some((walk.id, MediaTime::ZERO)),
			"a transition state without a matching exit must complete normally"
		);
	}
}
