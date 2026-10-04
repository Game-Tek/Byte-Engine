//! Runtime-only animation graph benchmark fixtures.

use resource_management::{
	Reference,
	resources::{
		animation::{Animation, NodeTrack, RotationCurve, ScaleCurve, TranslationCurve},
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
}

impl AnimationGraphBenchmarkFixture {
	/// Creates a graph and admits every required clip before measurement starts.
	pub fn new(benchmark: AnimationGraphBenchmark, node_count: usize) -> Self {
		assert!(node_count > 0, "Animation graph benchmarks need at least one skeleton node.");

		let mut animations = vec![(ACTIVE_CLIP_ID, benchmark_animation("active", node_count, 0.25))];
		if benchmark == AnimationGraphBenchmark::InertializedTransition {
			animations.push((DESTINATION_CLIP_ID, benchmark_animation("destination", node_count, 0.75)));
		}
		// Preallocate one arena large enough to keep every benchmark clip resident.
		let mut pool = AnimationPool::detached(
			animations
				.iter()
				.map(|(_, animation)| PackedAnimationData::resident_bytes(animation))
				.sum(),
		);
		for (resource_id, animation) in animations {
			pool.admit(resource_id.into(), animation);
		}
		let (graph, initial, destination) = benchmark_graph(benchmark);
		Self {
			graph,
			initial,
			destination,
			pool,
			benchmark,
		}
	}

	/// Creates retained player buffers and selects the path measured by [`AnimationGraphBenchmarkState::advance`].
	pub fn prepare(&mut self) -> AnimationGraphBenchmarkState<'_> {
		let root_motion =
			(self.benchmark == AnimationGraphBenchmark::ActiveRootMotion).then(|| RootMotionSettings::full("joint-0"));
		let mut player = self.pool.create_player(&self.graph, root_motion);

		// Resolve the initial state and, for the transition case, enter the
		// inertialized path before Divan starts the measured loop. `benchmark_graph`
		// returns a destination equal to the initial state unless the benchmark measures a transition.
		player
			.advance(MediaTime::ZERO, self.initial, &mut self.pool)
			.expect("resident benchmark clip must initialize")
			.expect("resident benchmark clip must initialize immediately");
		if self.destination != self.initial {
			player
				.advance(benchmark_frame_delta(), self.destination, &mut self.pool)
				.expect("resident benchmark clips must start their transition")
				.expect("resident benchmark clips must remain ready");
		}

		AnimationGraphBenchmarkState {
			player,
			pool: &mut self.pool,
			requested: self.destination,
		}
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
		self.player
			.advance(benchmark_frame_delta(), self.requested, self.pool)
			.expect("resident benchmark clips must advance")
			.expect("resident benchmark clips must remain ready")
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
			.when(MediaTime::from_seconds(TRANSITION_DURATION_SECONDS))
	} else {
		active
	};
	(
		builder.build(active).expect("benchmark graph must be valid"),
		active.id(),
		destination.id(),
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
				translation: Some(TranslationCurve::Linear {
					times: vec![0.0, CLIP_DURATION_SECONDS],
					values: vec![
						math::Vector::zero(),
						math::Vector::new(motion_scale + node_phase * 0.1, 0.05, 0.0),
					],
				}),
				rotation: Some(RotationCurve::Linear {
					times: vec![0.0, CLIP_DURATION_SECONDS],
					values: vec![
						math::Orientation::identity(),
						math::Orientation::try_from_array([0.0, 0.099_833_42, 0.0, 0.995_004_2])
							.expect("benchmark rotation is a unit quaternion"),
					],
				}),
				scale: Some(ScaleCurve::Linear {
					times: vec![0.0, CLIP_DURATION_SECONDS],
					values: vec![math::Scale::identity(), math::Scale::from_array([1.0 + node_phase * 0.01; 3])],
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

fn benchmark_frame_delta() -> MediaTime {
	MediaTime::from_frames(1, 60).expect("the engine timebase must represent 60 Hz exactly")
}
