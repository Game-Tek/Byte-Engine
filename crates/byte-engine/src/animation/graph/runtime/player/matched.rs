//! Matched-state playback: the motion a player holds for its matched states, the queries it searches with, and how a
//! matched clip's pose and root motion follow its ground root, as the animation reference's Matched states section
//! specifies.

use super::*;
use crate::animation::{
	matching::{
		GroundRoot, MAX_DIMENSIONS,
		search::{DatabaseInfo, Query, RequestedMotion, SearchResult, TrajectorySample, build_query},
		turn_back,
	},
	math::{NodeLookup, animated_yaw, object_transform, unique_node, wrap_angle, yaw_rotation},
};
use crate::math::Orientation;

/// The `HeldMotion` struct is the velocity and facing a player holds for every matched state of its graph, which the
/// latest request that carried them set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct HeldMotion {
	/// X and Z, in object-space units per second.
	pub(super) velocity: [f32; 2],
	/// A yaw about +Y, in radians, within -π..π.
	pub(super) facing: f32,
}

impl HeldMotion {
	/// Takes the velocity and facing a request carries, when they are finite; any other request leaves them.
	pub(super) fn hold(&mut self, value: BlendValue) {
		if let BlendValue::Motion { velocity, facing } = value
			&& velocity.iter().chain([&facing]).all(|component| component.is_finite())
		{
			*self = Self {
				velocity,
				facing: wrap_angle(facing),
			};
		}
	}

	/// Returns the held motion turned about +Y the other way by `root_motion`'s yaw, so it keeps its direction in the
	/// world while the entity turns by root motion.
	pub(super) fn turned(self, root_motion: RootMotionDelta) -> Self {
		// Most animators never request a matched state, and nothing turns a motion that holds still.
		if self == Self::default() {
			return self;
		}
		let yaw = yaw_of(root_motion.rotation);
		let [x, z] = self.velocity;
		let (x, z) = turn_back(yaw, x, z);
		Self {
			velocity: [x, z],
			facing: wrap_angle(self.facing - yaw),
		}
	}
}

/// The `MatchRecord` struct is what a player's latest search measured, which its data stream reports.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct MatchRecord {
	/// Whether the latest advance searched.
	pub(super) searched: bool,
	pub(super) result: SearchResult,
	pub(super) trajectory: [TrajectorySample; MAX_DIMENSIONS / 4],
	pub(super) trajectory_len: usize,
}

impl MatchRecord {
	/// Records a search's result and the trajectory its query predicted.
	pub(super) fn record(&mut self, query: &Query, result: SearchResult) {
		*self = Self {
			searched: true,
			result,
			trajectory: query.trajectory,
			trajectory_len: query.trajectory_len,
		};
	}
}

/// The `QueryPoses` struct is what a query measures the animator's pose from: the latest pose, the one before it,
/// and the root motion and time between them.
#[derive(Clone, Copy)]
pub(super) struct QueryPoses<'a> {
	pub(super) current: &'a [LocalTransform],
	pub(super) previous: &'a [LocalTransform],
	pub(super) root_motion: RootMotionDelta,
	pub(super) interval: MediaTime,
	/// Whether both poses are ones the animator produced, which the node features need.
	pub(super) produced: bool,
}

/// Builds the query a matched state searches its database with, from the animator's poses and its held motion.
pub(super) fn matched_query(
	info: &DatabaseInfo,
	nodes: &[u32],
	skeleton: &Skeleton,
	poses: QueryPoses<'_>,
	held: HeldMotion,
) -> Query {
	let seconds = poses.interval.as_seconds_f32();
	let RootMotionDelta { translation, rotation } = poses.root_motion;
	let mut current = [[0.0; 3]; MAX_DIMENSIONS / 3];
	let mut previous = [[0.0; 3]; MAX_DIMENSIONS / 3];
	let node_poses = (poses.produced && seconds > 0.0).then(|| {
		let undo = rotation.inverse();
		for ((&node, current), previous) in nodes.iter().zip(&mut current).zip(&mut previous) {
			*current = object_transform(skeleton, poses.current, node as usize).translation.to_array();
			// The previous pose is in the object space before the step's root motion moved the entity.
			let earlier = object_transform(skeleton, poses.previous, node as usize).translation - translation;
			*previous = undo.rotate_vector(earlier).to_array();
		}
		(&current[..nodes.len()], &previous[..nodes.len()], seconds)
	});
	let yaw = yaw_of(rotation);
	let (start_x, start_z) = turn_back(yaw, translation.x(), translation.z());
	let per_second = |value: f32| if seconds > 0.0 { value / seconds } else { 0.0 };
	build_query(
		info,
		node_poses,
		RequestedMotion {
			velocity: held.velocity,
			facing: held.facing,
			start_velocity: [per_second(start_x), per_second(start_z)],
			start_turn_rate: per_second(yaw),
		},
	)
}

/// Places a sampled pose relative to its ground root: the top-level nodes `roots` take the inverse of the ground
/// root's transform, so the ground root sits at the object origin, facing -Z.
pub(super) fn place_on_ground(pose: &mut [LocalTransform], roots: &[u32], ground: GroundRoot) {
	let turn = yaw_rotation(-ground.yaw);
	for &root in roots {
		let local = &mut pose[root as usize];
		let offset = local.translation - Vector::new(ground.x, 0.0, ground.z);
		local.translation = turn.rotate_vector(offset);
		local.rotation = turn.compose(local.rotation);
	}
}

/// Returns the root motion between two ground roots of one clip: the later one's position in the earlier one's root
/// frame, on X and Z, and the turn between their yaws about +Y.
pub(super) fn ground_delta(earlier: GroundRoot, later: GroundRoot) -> RootMotionDelta {
	let (x, z) = earlier.to_root_frame(later.x, later.z);
	RootMotionDelta {
		translation: Vector::new(x, 0.0, z),
		rotation: yaw_rotation(wrap_angle(later.yaw - earlier.yaw)),
	}
}

/// Returns how far `rotation` turns -Z about +Y, projected onto the XZ plane.
fn yaw_of(rotation: Orientation) -> f32 {
	animated_yaw(rotation, Orientation::identity())
}

/// Resolves a matched state's database nodes against the canonical skeleton, for its [`StateBinding`].
pub(super) fn resolve_nodes(info: &DatabaseInfo, skeleton: &Skeleton) -> Result<Box<[u32]>, AnimationGraphPlayerError> {
	let find = |name: &str| {
		unique_node(skeleton, name).map(|node| node as u32).map_err(|lookup| match lookup {
			NodeLookup::Missing => AnimationGraphPlayerError::MatchNodeNotFound { name: name.into() },
			NodeLookup::Duplicate => AnimationGraphPlayerError::DuplicateMatchNodeName { name: name.into() },
		})
	};
	find(&info.search.root)?;
	info.search.nodes.iter().map(|node| find(&node.name)).collect()
}

#[cfg(test)]
pub(super) mod tests {
	use super::*;
	use crate::animation::matching::{
		database_from_clips,
		tests::{idle, walk},
	};

	pub(in super::super) const DATABASE: &str = "locomotion.motion.bead";

	/// A pool holding a database of a standing and a walking clip, three seconds each, beside the clips themselves.
	/// `settings` adds to the database's settings.
	pub(in super::super) fn locomotion_pool(settings: &str) -> AnimationPool {
		let (standing, walking) = (idle(3.0), walk(3.0, 1.4));
		let database = database_from_clips(
			&format!(r#"{{ clips: ["placeholder"], root: {{ node: "hips" }}, {settings} }}"#),
			&[("idle.animation".into(), &standing), ("walk.animation".into(), &walking)],
		)
		.expect("the clips should form a database");
		let entries = [
			(DATABASE, PackedAnimationData::from_database(database)),
			("idle.animation", PackedAnimationData::from_resource(standing)),
			("walk.animation", PackedAnimationData::from_resource(walking)),
		];
		// Room to spare for clips a test admits later.
		let budget = entries.iter().map(|(_, packed)| packed.resident_bytes()).sum::<usize>() + 64 * 1024;
		let mut pool = AnimationPool::detached(budget);
		for (id, packed) in entries {
			pool.admit(id.into(), packed);
		}
		pool
	}

	fn step() -> MediaTime {
		MediaTime::from_frames(1, 30).expect("the engine timebase represents 30 Hz exactly")
	}

	/// Advances `player` `steps` steps toward `request` and returns the root motion along Z of the last ten.
	fn travel(
		player: &mut AnimationGraphPlayer,
		graph: &AnimationGraph,
		pool: &mut AnimationPool,
		request: AnimationRequest,
		steps: usize,
	) -> f32 {
		(0..steps)
			.map(|frame| {
				let pose = player
					.advance(graph, step(), request, pool)
					.expect("the player should advance")
					.expect("every clip is resident");
				if frame + 10 >= steps { pose.root_motion.translation.z() } else { 0.0 }
			})
			.sum()
	}

	#[test]
	fn a_matched_state_walks_toward_its_requested_velocity_and_stands_when_it_drops() {
		let mut pool = locomotion_pool("");
		let builder = AnimationGraph::builder();
		let locomotion = builder.state("locomotion").matched(DATABASE);
		let id = locomotion.id();
		let graph = builder.build(locomotion).expect("the graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("hips")));

		let walked = travel(&mut player, &graph, &mut pool, id.toward([0.0, -1.4], 0.0), 45);
		assert!((walked + 1.4 * 10.0 / 30.0).abs() < 0.05, "ten steps of walking moved {walked}");
		let stood = travel(&mut player, &graph, &mut pool, id.toward([0.0, 0.0], 0.0), 60);
		assert!(stood.abs() < 0.01, "ten steps of standing moved {stood}");
	}

	#[test]
	fn a_route_into_a_matched_state_blends_from_the_pose_on_screen_and_walks_on() {
		let mut pool = locomotion_pool("");
		let builder = AnimationGraph::builder();
		let stand = builder.state("stand").with(AnimationClip::looping("idle.animation"));
		let locomotion = builder.state("locomotion").matched(DATABASE);
		stand.to(locomotion).when(MediaTime::from_millis(200));
		let graph = builder.build(stand).expect("the graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("hips")));
		travel(&mut player, &graph, &mut pool, stand.id().into(), 5);

		// The walk swings its foot up to 0.3 along Z, but the blend starts from the standing foot at 0.
		let request = locomotion.id().toward([0.0, -1.4], 0.0);
		let pose = player
			.advance(&graph, step(), request, &mut pool)
			.expect("the player should advance")
			.expect("every clip is resident");
		let foot = pose.local_pose[1].translation.z();
		assert!(foot.abs() < 0.05, "the foot moved to {foot} in the step the route started");
		let walked = travel(&mut player, &graph, &mut pool, request, 45);
		assert!((walked + 1.4 * 10.0 / 30.0).abs() < 0.05, "ten steps of walking moved {walked}");
	}

	#[test]
	fn a_database_node_the_canonical_skeleton_lacks_rejects_the_animator_as_it_enters() {
		let mut pool = locomotion_pool("");
		// The initial clip's skeleton, which becomes the canonical one, has no node named `hips`.
		pool.admit(
			"stand.animation".into(),
			super::super::super::tests::test_animation("stand", 0.0),
		);
		let builder = AnimationGraph::builder();
		let stand = builder.state("stand").with(AnimationClip::looping("stand.animation"));
		let locomotion = builder.state("locomotion").matched(DATABASE);
		stand.to(locomotion).when(MediaTime::ZERO);
		let graph = builder.build(stand).expect("the graph should build");
		let mut player = pool.create_player(&graph, None);
		travel(&mut player, &graph, &mut pool, stand.id().into(), 1);

		let error = player
			.advance(&graph, step(), locomotion.id().toward([0.0, -1.4], 0.0), &mut pool)
			.err();
		assert_eq!(error, Some(AnimationGraphPlayerError::MatchNodeNotFound { name: "hips".into() }));
		assert!(player.pose().is_none(), "a rejected animator posts nothing");
	}

	#[test]
	fn matched_states_join_no_space_and_take_no_route_points() {
		let builder = AnimationGraph::builder();
		let locomotion = builder.planar_space("moving").state("locomotion").matched(DATABASE);
		assert_eq!(
			builder.build(locomotion).err(),
			Some(AnimationGraphBuildError::MatchedStateInSpace { state: 0 })
		);

		let builder = AnimationGraph::builder();
		let locomotion = builder.state("locomotion").matched(DATABASE);
		let jump = builder.state("jump").with(AnimationClip::once("jump.animation"));
		locomotion.to(jump).at_frame(3).when(MediaTime::ZERO);
		assert_eq!(
			builder.build(locomotion).err(),
			Some(AnimationGraphBuildError::RoutePointOnMatchedState { state: 0 })
		);
	}
}
