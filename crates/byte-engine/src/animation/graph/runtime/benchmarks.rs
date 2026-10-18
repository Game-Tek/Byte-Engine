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
	/// Switches between two states along zero-length routes, so every frame enters a state.
	StateSwitching,
	/// Steers through a planar blend of five clips keyed at 30 frames per second, with root motion, the way a
	/// third-person character walks most frames.
	Locomotion,
}

/// The clips of the locomotion blend: resource ID, blend position, and duration in seconds. Durations differ, as
/// authored walk cycles do, so the blend's duration changes with its weights.
const LOCOMOTION_CLIPS: [(&str, [f32; 2], f32); 5] = [
	("benchmark-idle.animation", [0.0, 0.0], 2.0),
	("benchmark-forward.animation", [0.0, 1.0], 1.0),
	("benchmark-right.animation", [1.0, 0.0], 1.1),
	("benchmark-backward.animation", [0.0, -1.0], 0.9),
	("benchmark-left.animation", [-1.0, 0.0], 1.05),
];
/// The frame rate the locomotion clips are keyed at, like clips baked from FBX.
const LOCOMOTION_FRAME_RATE: f32 = 30.0;

/// The `AnimationGraphBenchmarkFixture` struct owns graph and clip resources outside the measured loop.
pub struct AnimationGraphBenchmarkFixture {
	graph: AnimationGraph,
	initial: AnimationStateId,
	destination: AnimationStateId,
	/// The blend the locomotion benchmark steers through.
	locomotion: Option<PlanarStateId>,
	pool: AnimationPool,
	benchmark: AnimationGraphBenchmark,
}

impl AnimationGraphBenchmarkFixture {
	/// Creates a graph and admits every required clip before measurement starts.
	pub fn new(benchmark: AnimationGraphBenchmark, node_count: usize) -> Self {
		assert!(node_count > 0, "Animation graph benchmarks need at least one skeleton node.");

		let animations: Vec<(&str, PackedAnimationData)> = match benchmark {
			AnimationGraphBenchmark::Locomotion => LOCOMOTION_CLIPS
				.iter()
				.map(|&(resource_id, direction, duration)| {
					(resource_id, locomotion_animation(resource_id, node_count, direction, duration))
				})
				.collect(),
			AnimationGraphBenchmark::ActivePose | AnimationGraphBenchmark::ActiveRootMotion => {
				vec![(ACTIVE_CLIP_ID, benchmark_animation("active", node_count, 0.25))]
			}
			AnimationGraphBenchmark::InertializedTransition | AnimationGraphBenchmark::StateSwitching => vec![
				(ACTIVE_CLIP_ID, benchmark_animation("active", node_count, 0.25)),
				(DESTINATION_CLIP_ID, benchmark_animation("destination", node_count, 0.75)),
			],
		};
		// Preallocate one arena large enough to keep every benchmark clip resident.
		let mut pool = AnimationPool::detached(animations.iter().map(|(_, packed)| packed.resident_bytes()).sum());
		for (resource_id, packed) in animations {
			pool.admit(resource_id.into(), packed);
		}
		let (graph, initial, destination, locomotion) = benchmark_graph(benchmark);
		Self {
			graph,
			initial,
			destination,
			locomotion,
			pool,
			benchmark,
		}
	}

	/// Creates retained player buffers and selects the path measured by [`AnimationGraphBenchmarkState::advance`].
	pub fn prepare(&mut self) -> AnimationGraphBenchmarkState<'_> {
		let player = ready_player(
			&self.graph,
			&mut self.pool,
			self.benchmark,
			self.initial,
			self.destination,
			MediaTime::ZERO,
		);
		AnimationGraphBenchmarkState {
			player,
			pool: &mut self.pool,
			requested: self.destination,
		}
	}

	/// Creates `count` players that share this fixture's graph and pool, the way several characters play one graph.
	///
	/// Each player starts at its own phase, so the characters don't sample their clips in lockstep. Time them with
	/// [`AnimationGraphCharactersState::advance`].
	pub fn prepare_characters(&mut self, count: usize) -> AnimationGraphCharactersState<'_> {
		let players = (0..count)
			.map(|character| {
				// Spread the characters over one second of their clips, a frame apart.
				let offset = MediaTime::from_frames((character % 60) as i64, 60)
					.expect("the engine timebase must represent 60 Hz exactly");
				ready_player(
					&self.graph,
					&mut self.pool,
					self.benchmark,
					self.initial,
					self.destination,
					offset,
				)
			})
			.collect();
		AnimationGraphCharactersState {
			players,
			pool: &mut self.pool,
			alley: crate::core::alley::Alley::new(),
			requests: CharacterRequests {
				benchmark: self.benchmark,
				initial: self.initial,
				destination: self.destination,
				locomotion: self.locomotion,
				frame: 0,
			},
		}
	}
}

/// Creates a player, plays its initial state for `offset`, and, for the transition benchmark, starts the
/// inertialized transition, all before Divan starts the measured loop.
fn ready_player<'graph>(
	graph: &'graph AnimationGraph,
	pool: &mut AnimationPool,
	benchmark: AnimationGraphBenchmark,
	initial: AnimationStateId,
	destination: AnimationStateId,
	offset: MediaTime,
) -> AnimationGraphPlayer<'graph> {
	let root_motion = match benchmark {
		AnimationGraphBenchmark::ActiveRootMotion => Some(RootMotionSettings::full("joint-0")),
		// A walking character takes its horizontal travel and leaves the body's bob and turn in the pose.
		AnimationGraphBenchmark::Locomotion => Some(RootMotionSettings {
			node_name: "joint-0",
			translation: RootMotionTranslation::X.union(RootMotionTranslation::Z),
			rotation: RootMotionRotation::None,
		}),
		_ => None,
	};
	let mut player = pool.create_player(graph, root_motion);

	// `benchmark_graph` returns a destination equal to the initial state unless the benchmark measures a transition.
	player
		.advance(offset, initial, pool)
		.expect("resident benchmark clip must initialize")
		.expect("resident benchmark clip must initialize immediately");
	if destination != initial {
		player
			.advance(benchmark_frame_delta(), destination, pool)
			.expect("resident benchmark clips must start their transition")
			.expect("resident benchmark clips must remain ready");
	}
	player
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

/// The `AnimationGraphCharactersState` struct retains every character's player for timing a frame of a crowd.
pub struct AnimationGraphCharactersState<'fixture> {
	players: Vec<AnimationGraphPlayer<'fixture>>,
	pool: &'fixture mut AnimationPool,
	requests: CharacterRequests,
	/// The lanes a crowd spreads over, one for each hardware thread as an application's alley has.
	alley: crate::core::alley::Alley,
}

impl AnimationGraphCharactersState<'_> {
	/// Requests every character's state for one prepared frame, then advances them together.
	pub fn advance(&mut self) {
		for (character, player) in self.players.iter_mut().enumerate() {
			player
				.request(self.requests.request(character))
				.expect("benchmark requests come from the benchmark graph");
		}
		self.pool
			.advance_players(&mut self.players, benchmark_frame_delta(), &mut self.alley)
			.expect("resident benchmark clips must advance");
		std::hint::black_box(&self.players);
		self.requests.frame += 1;
	}
}

/// The `CharacterRequests` struct picks what each character of a benchmark crowd asks for on each frame.
struct CharacterRequests {
	benchmark: AnimationGraphBenchmark,
	initial: AnimationStateId,
	destination: AnimationStateId,
	locomotion: Option<PlanarStateId>,
	frame: usize,
}

impl CharacterRequests {
	/// Returns what `character` asks for this frame.
	///
	/// A switching crowd asks for the state it isn't in, a walking crowd steers around a circle, each character
	/// facing its own way, and the others keep asking for the destination.
	fn request(&self, character: usize) -> AnimationRequest {
		match (self.benchmark, self.locomotion) {
			(AnimationGraphBenchmark::StateSwitching, _) if self.frame.is_multiple_of(2) => self.initial.into(),
			(AnimationGraphBenchmark::Locomotion, Some(locomotion)) => {
				let angle = self.frame as f32 * 0.02 + character as f32 * 0.37;
				locomotion.at([0.8 * angle.cos(), 0.8 * angle.sin()])
			}
			_ => self.destination.into(),
		}
	}
}

/// Builds a graph whose selected path stays stable throughout one benchmark run.
///
/// Returns the graph, its initial state, the state the benchmark requests, and the locomotion blend if it has one.
fn benchmark_graph(
	benchmark: AnimationGraphBenchmark,
) -> (AnimationGraph, AnimationStateId, AnimationStateId, Option<PlanarStateId>) {
	let builder = AnimationGraph::builder();
	if benchmark == AnimationGraphBenchmark::Locomotion {
		let blend = LOCOMOTION_CLIPS
			.iter()
			.fold(AnimationBlend::new(), |blend, &(resource_id, position, _)| {
				blend.sample(position, AnimationClip::looping(resource_id))
			});
		let locomotion = builder.state("locomotion").planar(blend);
		let id = locomotion.id();
		let graph = builder.build(locomotion).expect("benchmark graph must be valid");
		return (graph, id.into(), id.into(), Some(id));
	}
	let active = builder.state("active").with(AnimationClip::looping(ACTIVE_CLIP_ID));
	let destination = match benchmark {
		AnimationGraphBenchmark::InertializedTransition => {
			let destination = builder.state("destination").with(AnimationClip::looping(DESTINATION_CLIP_ID));
			active
				.to(destination)
				.when(MediaTime::from_seconds(TRANSITION_DURATION_SECONDS))
		}
		AnimationGraphBenchmark::StateSwitching => {
			let destination = builder.state("destination").with(AnimationClip::looping(DESTINATION_CLIP_ID));
			destination.to(active).when(MediaTime::ZERO);
			active.to(destination).when(MediaTime::ZERO)
		}
		_ => active,
	};
	(
		builder.build(active).expect("benchmark graph must be valid"),
		active.id(),
		destination.id(),
		None,
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

/// Packs one fully animated track per node to represent normal runtime sampling work.
fn benchmark_animation(name: &str, node_count: usize, motion_scale: f32) -> PackedAnimationData {
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
	pack_clip(name, node_count, CLIP_DURATION_SECONDS, None, tracks)
}

/// Packs one walk cycle keyed at [`LOCOMOTION_FRAME_RATE`]: the root travels along `direction` and bobs, and every
/// other node sways, so every key differs as in motion-captured clips.
fn locomotion_animation(name: &str, node_count: usize, direction: [f32; 2], duration: f32) -> PackedAnimationData {
	let keys = (duration * LOCOMOTION_FRAME_RATE).round() as usize + 1;
	let times: Vec<f32> = (0..keys).map(|key| key as f32 / LOCOMOTION_FRAME_RATE).collect();
	// One cycle of sway over the clip.
	let sway = |time: f32, node: usize| (std::f32::consts::TAU * time / duration + node as f32 * 0.3).sin();
	let tracks = (0..node_count)
		.map(|node| {
			let translation = times
				.iter()
				.map(|&time| {
					if node == 0 {
						// The root walks 1.4 m/s along its direction and bobs twice a cycle.
						let bob = 0.02 * (2.0 * std::f32::consts::TAU * time / duration).sin();
						math::Vector::new(direction[0] * 1.4 * time, 0.9 + bob, direction[1] * 1.4 * time)
					} else {
						math::Vector::new(0.0, 0.1, 0.01 * sway(time, node))
					}
				})
				.collect();
			let rotation = times
				.iter()
				.map(|&time| {
					math::Orientation::try_from_rotation_vector(math::Vector::<resource_management::resources::ParentSpace>::new(
						0.2 * sway(time, node),
						0.1 * sway(time, node + 7),
						0.05,
					))
					.expect("a finite rotation vector is a valid rotation")
				})
				.collect();
			NodeTrack {
				node: node as u32,
				translation: Some(TranslationCurve::Linear {
					times: times.clone(),
					values: translation,
				}),
				rotation: Some(RotationCurve::Linear {
					times: times.clone(),
					values: rotation,
				}),
				// Authored rigs rarely scale their bones; only the root keys its scale, as exporters write it.
				scale: (node == 0).then(|| ScaleCurve::Linear {
					times: times.clone(),
					values: vec![math::Scale::identity(); keys],
				}),
			}
		})
		.collect();
	pack_clip(name, node_count, duration, Some(LOCOMOTION_FRAME_RATE), tracks)
}

/// Packs `tracks` as a clip authored for the benchmark skeleton of `node_count` nodes.
fn pack_clip(
	name: &str,
	node_count: usize,
	duration: f32,
	frame_rate: Option<f32>,
	tracks: Vec<NodeTrack>,
) -> PackedAnimationData {
	PackedAnimationData::from_resource(Animation {
		name: Some(name.into()),
		skeleton: Reference::in_memory(format!("{name}.skeleton"), benchmark_skeleton(node_count)),
		duration,
		frame_rate,
		tracks,
	})
}

fn benchmark_frame_delta() -> MediaTime {
	MediaTime::from_frames(1, 60).expect("the engine timebase must represent 60 Hz exactly")
}


