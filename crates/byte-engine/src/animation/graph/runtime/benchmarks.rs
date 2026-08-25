//! Runtime-only animation graph benchmark fixtures.

use std::collections::{HashMap, VecDeque};

use resource_management::{
	Reference,
	resources::{
		animation::{Animation, NodeTrack, QuaternionCurve, Vector3Curve},
		skeleton::{LocalTransform, Skeleton, SkeletonNode},
	},
};

use super::*;
use crate::MediaTime;

const ACTIVE_CLIP_ID: &str = "benchmark-active.animation";
const DESTINATION_CLIP_ID: &str = "benchmark-destination.animation";
const CLIP_DURATION_SECONDS: f32 = 1.0;
// Keep the fixture on the transition path even for very fast, long-running samples.
const TRANSITION_DURATION_SECONDS: i64 = 31_536_000;

/// The `AnimationGraphBenchmark` enum selects one animation graph evaluation path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationGraphBenchmark {
	ActivePose,
	ActiveRootMotion,
	InertializedTransition,
}

/// The `AnimationGraphBenchmarkFixture` struct owns graph and clip resources outside the measured loop.
pub struct AnimationGraphBenchmarkFixture {
	graph: AnimationGraph,
	initial: AnimationStateId,
	destination: AnimationStateId,
	pool: AnimationPool,
	benchmark: AnimationGraphBenchmark,
	node_count: usize,
}

impl AnimationGraphBenchmarkFixture {
	/// Creates a graph and admits every required clip before measurement starts.
	pub fn new(benchmark: AnimationGraphBenchmark, node_count: usize) -> Self {
		assert!(node_count > 0, "Animation graph benchmarks need at least one skeleton node.");

		let active_animation = benchmark_animation("active", node_count, 0.25);
		let mut animations = vec![(ACTIVE_CLIP_ID, active_animation)];
		if benchmark == AnimationGraphBenchmark::InertializedTransition {
			animations.push((DESTINATION_CLIP_ID, benchmark_animation("destination", node_count, 0.75)));
		}
		let mut pool = benchmark_pool(&animations);
		for (resource_id, animation) in animations {
			pool.admit_lease(AnimationLease::new(resource_id), animation);
		}
		let (graph, initial, destination) = benchmark_graph(benchmark);
		Self {
			graph,
			initial,
			destination,
			pool,
			benchmark,
			node_count,
		}
	}

	/// Creates retained player buffers and selects the path measured by [`AnimationGraphBenchmarkState::advance`].
	pub fn prepare(&mut self) -> AnimationGraphBenchmarkState<'_> {
		let Self {
			graph,
			initial,
			destination,
			pool,
			benchmark,
			node_count,
		} = self;
		let benchmark = *benchmark;
		let root_motion = (benchmark == AnimationGraphBenchmark::ActiveRootMotion).then(|| RootMotionSettings::full("joint-0"));
		let mut player = pool.create_player(graph, root_motion);

		// Resolve the initial state and, for the transition case, enter the
		// inertialized path before Divan starts the measured loop.
		let AnimationEvaluation::Ready(_) = player
			.advance(MediaTime::ZERO, *initial, pool)
			.expect("resident benchmark clip must initialize")
		else {
			panic!("resident benchmark clip must initialize immediately");
		};
		let requested = if benchmark == AnimationGraphBenchmark::InertializedTransition {
			*destination
		} else {
			*initial
		};
		if requested != *initial {
			let AnimationEvaluation::Ready(_) = player
				.advance(benchmark_frame_delta(), requested, pool)
				.expect("resident benchmark clips must start their transition")
			else {
				panic!("resident benchmark clips must remain ready");
			};
		}

		AnimationGraphBenchmarkState { player, pool, requested }
	}
}

/// The `AnimationGraphBenchmarkState` struct retains the player state used to time animation graph evaluation.
pub struct AnimationGraphBenchmarkState<'fixture> {
	player: AnimationGraphPlayer<'fixture>,
	pool: &'fixture mut AnimationPool,
	requested: AnimationStateId,
}

impl AnimationGraphBenchmarkState<'_> {
	/// Advances one prepared frame and returns the borrowed pose to the benchmark harness.
	pub fn advance(&mut self) -> AnimationGraphPose<'_> {
		let AnimationEvaluation::Ready(pose) = self
			.player
			.advance(benchmark_frame_delta(), self.requested, self.pool)
			.expect("resident benchmark clips must remain ready")
		else {
			panic!("resident benchmark clips must remain ready");
		};
		pose
	}
}

/// Builds a graph whose selected path stays stable throughout one benchmark run.
fn benchmark_graph(benchmark: AnimationGraphBenchmark) -> (AnimationGraph, AnimationStateId, AnimationStateId) {
	let builder = AnimationGraph::builder();
	let active = builder.state("active").with(AnimationClip::looping(ACTIVE_CLIP_ID));
	let destination = if benchmark == AnimationGraphBenchmark::InertializedTransition {
		let destination = builder.state("destination").with(AnimationClip::looping(DESTINATION_CLIP_ID));
		active
			.to(destination)
			.when(AnimationTransition::new().inertialize(MediaTime::from_seconds(TRANSITION_DURATION_SECONDS)));
		destination
	} else {
		active
	};
	let initial = active.id();
	let destination = destination.id();
	(
		builder.build(active).expect("benchmark graph must be valid"),
		initial,
		destination,
	)
}

/// Creates a parented chain so global-pose work scales with the benchmark argument.
fn benchmark_skeleton(node_count: usize) -> Skeleton {
	let nodes = (0..node_count)
		.map(|node| SkeletonNode {
			name: Some(format!("joint-{node}")),
			parent: node.checked_sub(1).map(|parent| parent as u32),
			rest_local: LocalTransform::identity(),
		})
		.collect();
	Skeleton { nodes }
}

/// Creates one fully animated track per node to represent normal runtime sampling work.
fn benchmark_animation(name: &str, node_count: usize, motion_scale: f32) -> Animation {
	let tracks = (0..node_count)
		.map(|node| {
			let node_phase = node as f32 / node_count as f32;
			NodeTrack {
				node: node as u32,
				translation: Some(Vector3Curve::Linear {
					times: vec![0.0, CLIP_DURATION_SECONDS],
					values: vec![[0.0; 3], [motion_scale + node_phase * 0.1, 0.05, 0.0]],
				}),
				rotation: Some(QuaternionCurve::Linear {
					times: vec![0.0, CLIP_DURATION_SECONDS],
					values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.099_833_42, 0.0, 0.995_004_2]],
				}),
				scale: Some(Vector3Curve::Linear {
					times: vec![0.0, CLIP_DURATION_SECONDS],
					values: vec![[1.0; 3], [1.0 + node_phase * 0.01; 3]],
				}),
			}
		})
		.collect();
	Animation {
		name: Some(name.into()),
		skeleton: Reference::in_memory(format!("benchmark-{name}.skeleton"), benchmark_skeleton(node_count)),
		duration: CLIP_DURATION_SECONDS,
		tracks,
	}
}

/// Preallocates one arena large enough to keep every benchmark clip resident.
fn benchmark_pool(animations: &[(&str, Animation)]) -> AnimationPool {
	let byte_budget = animations
		.iter()
		.map(|(_, animation)| PackedAnimationData::resident_bytes(animation))
		.sum();
	let word_capacity = byte_budget / std::mem::size_of::<u32>();
	let (commands, _command_receiver) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
	let (_completion_sender, completions) = kanal::bounded_async(ANIMATION_LOAD_QUEUE_CAPACITY);
	AnimationPool {
		commands: commands.to_sync(),
		completions: completions.to_sync(),
		storage: vec![0; word_capacity].into_boxed_slice(),
		free_regions: vec![AnimationArenaRegion {
			offset: 0,
			word_count: word_capacity,
		}],
		entries: HashMap::with_capacity(animations.len()),
		events: VecDeque::with_capacity(ANIMATION_POOL_EVENT_CAPACITY),
		byte_budget,
		resident_bytes: 0,
		next_use: std::cell::Cell::new(0),
		commands_closed: false,
		completions_closed: false,
	}
}

fn benchmark_frame_delta() -> MediaTime {
	MediaTime::from_frames(1, 60).expect("the engine timebase must represent 60 Hz exactly")
}
