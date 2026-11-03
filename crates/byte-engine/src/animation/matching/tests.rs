//! Tests of motion databases and their search, against the animation reference's Motion databases and Matched states
//! sections.

use resource_management::{
	Reference,
	resources::{
		animation::{Animation, NodeTrack, RotationCurve, TranslationCurve},
		motion_database::MotionDatabase,
		skeleton::{LocalTransform, Skeleton, SkeletonNode},
	},
};
use crate::animation::{
	math::{yaw_of_direction, yaw_rotation},
	test_node,
};

use super::{
	MAX_DIMENSIONS, PredictedVelocity, database_from_clips,
	features::DatabaseError,
	search::{DatabaseInfo, Found, Playing, Query, RequestedMotion, TrajectorySample, build_query, pack_database, search},
};
use crate::math::Vector;

/// The frames per second the test clips are keyed at.
const KEY_RATE: f32 = 30.0;

fn node(name: &str, parent: Option<u32>, translation: [f32; 3]) -> SkeletonNode {
	let rest_local = LocalTransform {
		translation: Vector::from_array(translation),
		..LocalTransform::identity()
	};
	test_node(Some(name), parent, rest_local)
}

/// A skeleton of hips a unit above the ground and a foot a unit below the hips.
pub(crate) fn skeleton() -> Skeleton {
	Skeleton {
		nodes: vec![node("hips", None, [0.0, 1.0, 0.0]), node("foot", Some(0), [0.0, -1.0, 0.0])],
	}
}

/// A clip of `duration` seconds keyed [`KEY_RATE`] times a second: the hips at the position and yaw `hips` gives for a
/// time, and the foot where `foot` places it below the hips.
pub(crate) fn clip(duration: f32, hips: impl Fn(f32) -> ([f32; 3], f32), foot: impl Fn(f32) -> [f32; 3]) -> Animation {
	let keys = (duration * KEY_RATE).round() as usize + 1;
	let times: Vec<f32> = (0..keys).map(|key| (key as f32 / KEY_RATE).min(duration)).collect();
	Animation {
		name: None,
		skeleton: Reference::in_memory("test.skeleton", skeleton()),
		duration,
		frame_rate: Some(KEY_RATE),
		tracks: vec![
			NodeTrack {
				node: 0,
				translation: Some(TranslationCurve::Linear {
					times: times.clone(),
					values: times.iter().map(|&time| Vector::from_array(hips(time).0)).collect(),
				}),
				rotation: Some(RotationCurve::Linear {
					times: times.clone(),
					values: times.iter().map(|&time| yaw_rotation(hips(time).1)).collect(),
				}),
				scale: None,
			},
			NodeTrack {
				node: 1,
				translation: Some(TranslationCurve::Linear {
					times: times.clone(),
					values: times.iter().map(|&time| Vector::from_array(foot(time))).collect(),
				}),
				rotation: None,
				scale: None,
			},
		],
	}
}

/// A clip whose hips walk along -Z at `speed` units a second, facing -Z, with the foot swinging.
pub(crate) fn walk(duration: f32, speed: f32) -> Animation {
	clip(
		duration,
		|time| ([0.0, 1.0, -speed * time], 0.0),
		|time| [0.0, -1.0, 0.3 * (std::f32::consts::TAU * time).sin()],
	)
}

/// A clip whose hips walk a circle at `speed` units a second, turning right at `turn_rate` radians a second from
/// facing -Z, with the foot swinging.
pub(crate) fn circle_walk(duration: f32, speed: f32, turn_rate: f32) -> Animation {
	let radius = speed / turn_rate;
	clip(
		duration,
		|time| {
			let yaw = turn_rate * time;
			([-radius * (1.0 - yaw.cos()), 1.0, -radius * yaw.sin()], yaw)
		},
		|time| [0.0, -1.0, 0.3 * (std::f32::consts::TAU * time).sin()],
	)
}

/// `animation` with its skeleton under a `root` node half a unit up, which no track moves, so its hips have a parent.
pub(crate) fn rooted(mut animation: Animation) -> Animation {
	let mut nodes = vec![node("root", None, [0.0, 0.5, 0.0])];
	nodes.extend(skeleton().nodes.into_iter().map(|mut node| {
		node.parent = Some(node.parent.map_or(0, |parent| parent + 1));
		node
	}));
	animation.skeleton = Reference::in_memory("rooted.skeleton", Skeleton { nodes });
	for track in &mut animation.tracks {
		track.node += 1;
	}
	animation
}

/// A clip whose hips stand still, facing -Z.
pub(crate) fn idle(duration: f32) -> Animation {
	clip(duration, |_| ([0.0, 1.0, 0.0], 0.0), |_| [0.0, -1.0, 0.0])
}

/// The settings of a database over `clips` with an unsmoothed ground root, so frames measure exactly what they sample.
fn settings(extra: &str) -> String {
	format!(r#"{{ clips: ["placeholder"], root: {{ node: "hips", position_smoothing: 0, facing_smoothing: 0 }}, {extra} }}"#)
}

fn database(extra: &str, clips: &[(&str, &Animation)]) -> Result<MotionDatabase, String> {
	let clips: Vec<(String, &Animation)> = clips.iter().map(|(id, clip)| (id.to_string(), *clip)).collect();
	database_from_clips(&settings(extra), &clips)
}

/// Returns a frame's feature vector before normalization.
fn raw_features(database: &MotionDatabase, frame: usize) -> Vec<f32> {
	let dimensions = database.search.dimensions();
	database.features[frame * dimensions..][..dimensions]
		.iter()
		.zip(&database.search.mean)
		.zip(&database.search.scale)
		.map(|((value, mean), scale)| value / scale + mean)
		.collect()
}

fn assert_near(actual: f32, expected: f32, what: &str) {
	assert!(
		(actual - expected).abs() < 1.0e-3,
		"{what}: expected {expected}, found {actual}"
	);
}

#[test]
fn frames_are_numbered_across_clips_and_pickable_while_the_whole_trajectory_fits() {
	let (long, short) = (idle(2.0), walk(1.5, 1.0));
	let database = database(
		"trajectory: { times: [0.5, 1.0] }",
		&[("long.animation", &long), ("short.animation", &short)],
	)
	.expect("the clips should form a database");

	let clips: Vec<_> = database
		.clips
		.iter()
		.map(|clip| (clip.id.as_str(), clip.frames, clip.pickable))
		.collect();
	assert_eq!(clips, [("long.animation", 61, 31), ("short.animation", 46, 16)]);
	assert_eq!(database.frame_count(), 107);
	assert_eq!(database.features.len(), 107 * database.search.dimensions());
}

#[test]
fn ground_roots_project_the_root_node_and_follow_its_yaw() {
	let turning = clip(2.0, |time| ([0.5 * time, 1.0, -time], 0.8 * time), |_| [0.0, -1.0, 0.0]);
	let database = database("", &[("turn.animation", &turning)]).expect("the clip should form a database");

	for frame in [0, 15, 60] {
		let time = frame as f32 / 30.0;
		let [x, z, yaw] = database.ground[frame];
		assert_near(x, 0.5 * time, "ground x");
		assert_near(z, -time, "ground z");
		assert_near(yaw, 0.8 * time, "ground yaw");
	}
}

#[test]
fn smoothing_keeps_steady_motion_and_pulls_the_clip_ends_inward() {
	let walking = walk(2.0, 1.0);
	let clips: Vec<(String, &Animation)> = vec![("walk.animation".into(), &walking)];
	let database = database_from_clips(
		r#"{ clips: ["walk.animation"], root: { node: "hips", position_smoothing: 0.1 } }"#,
		&clips,
	)
	.expect("the clip should form a database");

	// A Gaussian of 0.1 s reaches 0.3 s, nine frames, to either side.
	assert_near(database.ground[30][1], -1.0, "the middle of a steady walk");
	assert!(database.ground[0][1] < -1.0e-3, "the first frame averages in the frames after it");
	assert!(database.ground[60][1] > -2.0 + 1.0e-3, "the last frame averages in the frames before it");
}

#[test]
fn features_measure_nodes_and_the_trajectory_in_each_frames_root_frame() {
	// The hips face +X turned a quarter turn, -X, and walk that way, so in their root frame they walk straight ahead.
	let quarter = std::f32::consts::FRAC_PI_2;
	let walking = clip(3.0, |time| ([-1.4 * time, 1.0, 0.0], quarter), |_| [0.0, -1.0, 0.0]);
	let database = database(
		r#"nodes: { foot: { position: 1, velocity: 1 } }, trajectory: { times: [0.5, 1.0] }"#,
		&[("walk.animation", &walking)],
	)
	.expect("the clip should form a database");

	assert_eq!(database.search.dimensions(), 3 + 3 + 4 + 4);
	let features = raw_features(&database, 30);
	let expected = [
		// The foot's position, a unit below the hips, which sit a unit up over the ground root.
		0.0, 0.0, 0.0, //
		// The foot moves with the hips: 1.4 a second straight ahead.
		0.0, 0.0, -1.4, //
		// The trajectory positions, half a second and a second ahead.
		0.0, -0.7, 0.0, -1.4, //
		// The trajectory directions, straight ahead.
		0.0, -1.0, 0.0, -1.0,
	];
	for (dimension, (actual, expected)) in features.iter().zip(expected).enumerate() {
		assert_near(*actual, expected, &format!("dimension {dimension}"));
	}
}

#[test]
fn databases_that_cannot_be_searched_are_rejected() {
	let walking = walk(2.0, 1.0);
	let duplicated = Animation {
		skeleton: Reference::in_memory(
			"duplicate.skeleton",
			Skeleton {
				nodes: vec![node("hips", None, [0.0; 3]), node("hips", Some(0), [0.0; 3])],
			},
		),
		tracks: Vec::new(),
		..walk(2.0, 1.0)
	};
	let error = |extra: &str, clip: &Animation| {
		let clips: Vec<(String, &Animation)> = vec![("clip.animation".into(), clip)];
		super::features::build_database(
			&super::bead::MotionBead::parse(settings(extra).as_bytes()).expect("the settings should parse"),
			&clips,
		)
		.expect_err("the database should be rejected")
	};

	assert_eq!(
		error("nodes: { hand: { position: 1 } }", &walking),
		DatabaseError::NodeNotFound {
			clip: "clip.animation".into(),
			node: "hand".into()
		}
	);
	assert_eq!(
		error("", &duplicated),
		DatabaseError::DuplicateNode {
			clip: "clip.animation".into(),
			node: "hips".into()
		}
	);
	assert_eq!(
		error("trajectory: { position: 0, direction: 0 }", &walking),
		DatabaseError::NoDimensions
	);
	let times: Vec<String> = (1..=17).map(|time| format!("{}", time as f32 * 0.05)).collect();
	assert_eq!(
		error(&format!("trajectory: {{ times: [{}] }}", times.join(", ")), &walking),
		DatabaseError::TooManyDimensions(68)
	);
	assert_eq!(error("trajectory: { times: [2.5] }", &walking), DatabaseError::NoPickableFrame);
}

#[test]
fn settings_out_of_their_range_or_unknown_fail_to_parse() {
	for extra in [
		"sample_rate: 0",
		"nodes: { foot: {} }",
		"trajectory: { times: [1.0, 0.5] }",
		"search: { interval: 0 }",
		"halflife: { velocity: -1 }",
		"unknown: 1",
	] {
		assert!(
			super::bead::MotionBead::parse(settings(extra).as_bytes()).is_err(),
			"`{extra}` should be rejected"
		);
	}
	assert!(super::bead::MotionBead::parse(br#"{ clips: [], root: { node: "hips" } }"#).is_err());
}

/// The database a query is normalized with: trajectory positions and directions at 0.5 s and 1 s.
fn trajectory_database() -> MotionDatabase {
	let walking = walk(3.0, 1.4);
	database("trajectory: { times: [0.5, 1.0] }", &[("walk.animation", &walking)]).expect("the clip should form a database")
}

#[test]
fn a_query_that_starts_at_its_requested_velocity_predicts_a_straight_line() {
	let (info, _) = pack_database(trajectory_database());
	let query = build_query(
		&info,
		None,
		RequestedMotion {
			velocity: [0.6, -1.2],
			facing: 0.0,
			predicted: PredictedVelocity {
				velocity: [0.6, -1.2],
				acceleration: [0.0; 2],
			},
			start_turn_rate: 0.0,
		},
	);

	let trajectory = &query.trajectory[..query.trajectory_len];
	assert_eq!(trajectory.len(), 2);
	for sample in trajectory {
		assert_near(sample.position[0], 0.6 * sample.time, "x");
		assert_near(sample.position[1], -1.2 * sample.time, "z");
		assert_near(sample.direction[0], 0.0, "direction x");
		assert_near(sample.direction[1], -1.0, "direction z");
	}
	assert_eq!(query.first_dimension, 0, "without node features the cost compares every dimension");
}

#[test]
fn a_query_turns_toward_its_facing_and_eases_into_its_velocity() {
	let (info, _) = pack_database(trajectory_database());
	let query = build_query(
		&info,
		None,
		RequestedMotion {
			velocity: [0.0, -1.4],
			facing: 1.0,
			predicted: PredictedVelocity::default(),
			start_turn_rate: 0.0,
		},
	);

	let yaw = |sample: &TrajectorySample| yaw_of_direction(sample.direction[0], sample.direction[1]);
	let [half, whole] = [query.trajectory[0], query.trajectory[1]];
	assert!(0.0 < yaw(&half) && yaw(&half) < yaw(&whole) && yaw(&whole) < 1.0, "the facing turns toward 1 rad");
	assert!(
		-1.4 * half.time < half.position[1] && half.position[1] < 0.0,
		"starting from rest, the trajectory trails the requested velocity"
	);
}

/// Builds a database whose frames all differ: a clip that speeds up and turns, and a walk.
fn varied_database() -> MotionDatabase {
	let speeding = clip(
		4.0,
		|time| ([0.2 * time * time, 1.0, -0.5 * time * time], 0.3 * time),
		|time| [0.1 * time, -1.0, 0.4 * (3.0 * time).sin()],
	);
	let walking = walk(4.0, 1.4);
	database(
		"nodes: { foot: { position: 1, velocity: 0.5 } }",
		&[("speeding.animation", &speeding), ("walk.animation", &walking)],
	)
	.expect("the clips should form a database")
}

/// Returns a query whose normalized features are `features`.
fn query_of(features: &[f32], first_dimension: usize) -> Query {
	let mut query = Query {
		features: [0.0; MAX_DIMENSIONS],
		first_dimension,
		trajectory: Default::default(),
		trajectory_len: 0,
	};
	query.features[..features.len()].copy_from_slice(features);
	query
}

#[test]
fn searching_with_a_frames_features_finds_that_frame_at_no_cost() {
	let database = varied_database();
	let dimensions = database.search.dimensions();
	let features = database.features.clone();
	let (info, words) = pack_database(database);
	let packed = super::search::PackedDatabase::new(&info, &words);

	for frame in [0, 7, 40, 90] {
		let query = query_of(&features[frame * dimensions..][..dimensions], 0);
		let found = search(packed, &query, None, |_| true).found.expect("a frame should be found");
		assert_eq!((frame_of(&info, found), found.cost), (frame, 0.0));
	}
}

/// Returns the database frame a search found, from its clip and time.
fn frame_of(info: &DatabaseInfo, found: Found) -> usize {
	info.clips[found.clip].first + (found.time * info.search.sample_rate).round() as usize
}

/// The least-cost candidate frame and its cost, found by comparing every candidate as the reference specifies.
fn every_candidate(database: &MotionDatabase, query: &Query, playing: Option<Playing>, resident: &[bool]) -> Option<(usize, f32)> {
	let dimensions = database.search.dimensions();
	let cost = |frame: usize| {
		database.features[frame * dimensions..][query.first_dimension..dimensions]
			.iter()
			.zip(&query.features[query.first_dimension..dimensions])
			.map(|(value, query)| (value - query) * (value - query))
			.sum::<f32>()
	};
	let mut first = 0;
	let mut placed = Vec::new();
	for (index, clip) in database.clips.iter().enumerate() {
		placed.push((index, first, clip.frames as usize, clip.pickable as usize));
		first += clip.frames as usize;
	}
	let bar = playing.and_then(|playing| {
		let (_, first, frames, pickable) = placed[playing.clip];
		let frame = ((playing.time * database.search.sample_rate - 0.5).ceil().max(0.0) as usize).min(frames - 1);
		(frame < pickable).then(|| cost(first + frame))
	});
	let mut best: Option<(usize, f32)> = None;
	for &(index, first, _, pickable) in &placed {
		if !resident[index] {
			continue;
		}
		for frame in first..first + pickable {
			let time = (frame - first) as f32 / database.search.sample_rate;
			if playing.is_some_and(|playing| playing.clip == index && (time - playing.time).abs() < database.search.exclusion) {
				continue;
			}
			let cost = cost(frame);
			if bar.is_none_or(|bar| cost < bar) && best.is_none_or(|(_, best)| cost < best) {
				best = Some((frame, cost));
			}
		}
	}
	best
}

#[test]
fn the_search_finds_what_comparing_every_candidate_finds() {
	let database = varied_database();
	let dimensions = database.search.dimensions();
	let reference = database.clone();
	let (info, words) = pack_database(database);
	let packed = super::search::PackedDatabase::new(&info, &words);
	// A fixed linear congruential sequence keeps the queries the same on every run.
	let mut state = 0x2545_f491_u32;
	let mut random = move || {
		state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
		(state >> 8) as f32 / (1 << 24) as f32 * 4.0 - 2.0
	};
	for case in 0..200 {
		let features: Vec<f32> = (0..dimensions).map(|_| random()).collect();
		let query = query_of(&features, if case % 3 == 0 { info.node_dimensions } else { 0 });
		let playing = (case % 2 == 0).then(|| Playing {
			clip: case % 4 / 2,
			time: (random() + 2.0) * 0.6,
		});
		let resident = [true, case % 5 != 0];
		let found = search(packed, &query, playing, |clip| resident[clip]).found;
		assert_eq!(
			found.map(|found| (frame_of(&info, found), found.cost)),
			every_candidate(&reference, &query, playing, &resident),
			"case {case}"
		);
	}
}

#[test]
fn the_playing_frame_sets_the_bar_and_frames_near_it_are_no_candidates() {
	let database = varied_database();
	let dimensions = database.search.dimensions();
	let features = database.features.clone();
	let (info, words) = pack_database(database);
	let packed = super::search::PackedDatabase::new(&info, &words);
	let frame = 30;
	let query = query_of(&features[frame * dimensions..][..dimensions], 0);

	let playing_it = search(
		packed,
		&query,
		Some(Playing {
			clip: 0,
			time: frame as f32 / 30.0,
		}),
		|_| true,
	);
	assert_eq!(playing_it.bar, Some(0.0));
	assert_eq!(playing_it.found, None, "nothing beats the frame already playing");

	// Playing a tenth of a second later, the frame is within the exclusion time of the playing time.
	let near = search(
		packed,
		&query,
		Some(Playing {
			clip: 0,
			time: frame as f32 / 30.0 + 0.1,
		}),
		|_| true,
	);
	assert!(
		near.found
			.is_none_or(|found| found.clip != 0 || (found.time - (frame as f32 / 30.0 + 0.1)).abs() >= 0.333),
		"no frame within the exclusion time is a candidate, not even the query's own"
	);
}

/// A database whose only clip is a one-second walk loop, in which the foot swings through one whole stride.
fn walk_loop(extra: &str) -> MotionDatabase {
	let walking = walk(1.0, 1.4);
	let clips: Vec<(String, &Animation)> = vec![("walk.animation".into(), &walking)];
	database_from_clips(
		&format!(r#"{{ clips: ["placeholder"], loops: ["*.animation"], root: {{ node: "hips" }}, {extra} }}"#),
		&clips,
	)
	.expect("the loop should form a database")
}

#[test]
fn a_looping_clip_is_pickable_at_every_frame_and_its_trajectory_continues_past_its_end() {
	let database = walk_loop("trajectory: { times: [0.5] }");

	let clip = &database.clips[0];
	// The frames stop short of the duration, where the next cycle's first frame stands.
	assert_eq!((clip.frames, clip.pickable), (30, 30));
	let [x, z, yaw] = clip.cycle.expect("the clip loops");
	assert_near(x, 0.0, "cycle x");
	assert_near(z, -1.4, "cycle z");
	assert_near(yaw, 0.0, "cycle yaw");
	// The last frame's trajectory runs half a second into the next cycle, 0.7 straight ahead.
	let features = raw_features(&database, 29);
	assert_near(features[0], 0.0, "trajectory x");
	assert_near(features[1], -0.7, "trajectory z");
}

#[test]
fn a_looping_clip_smooths_its_ground_root_across_its_ends() {
	let database = walk_loop("");

	// Continued by its cycle, a steady walk stays steady at the ends a clip played once pulls inward.
	assert_near(database.ground[0][1], 0.0, "the first frame");
	assert_near(database.ground[29][1], -1.4 * 29.0 / 30.0, "the last frame");
}

#[test]
fn a_loops_entry_must_name_a_clip_of_the_database() {
	let walking = walk(1.0, 1.4);
	let error = database(r#"loops: ["run.animation"]"#, &[("walk.animation", &walking)])
		.expect_err("a loops entry that names no clip should fail");
	assert!(error.contains("'run.animation'"), "the error names the entry: {error}");
}

#[test]
fn a_looping_clip_excludes_and_bars_frames_around_its_wrap() {
	let database = walk_loop("");
	let dimensions = database.search.dimensions();
	let features = database.features.clone();
	let (info, words) = pack_database(database);
	let packed = super::search::PackedDatabase::new(&info, &words);
	let first_frame = query_of(&features[..dimensions], 0);

	// Past the last frame, at 29/30 s, the first frame is nearer, standing at the duration.
	let at_the_wrap = search(packed, &first_frame, Some(Playing { clip: 0, time: 0.99 }), |_| true);
	assert_eq!(at_the_wrap.bar, Some(0.0));
	assert_eq!(at_the_wrap.found, None, "nothing beats the first frame playing again");

	// A tenth of a second before the end, the first frame is a tenth of a second away around the loop.
	let before_the_wrap = search(packed, &first_frame, Some(Playing { clip: 0, time: 0.9 }), |_| true);
	let around = |time: f32| (time - 0.9).abs().min(1.0 - (time - 0.9).abs());
	assert!(
		before_the_wrap.found.is_none_or(|found| around(found.time) >= 0.333),
		"no frame within the exclusion time around the loop is a candidate"
	);
}

#[test]
fn a_prediction_stepped_toward_its_goal_travels_the_queried_trajectory() {
	let start = PredictedVelocity {
		velocity: [1.0, -0.5],
		acceleration: [-2.0, 3.0],
	};
	let (goal, damping) = ([-1.4, 0.2], super::search::damping(0.27));

	// Summing the velocity over small steps integrates the trajectory a query predicts from the same start.
	let steps = 3000;
	let seconds = 1.0 / steps as f32;
	let (mut prediction, mut travelled) = (start, [0.0f32; 2]);
	for _ in 0..steps {
		let next = prediction.toward(goal, damping, seconds);
		for axis in 0..2 {
			travelled[axis] += (prediction.velocity[axis] + next.velocity[axis]) * 0.5 * seconds;
		}
		prediction = next;
	}
	let trajectory = start.travel(goal, damping, 1.0);
	let stepped_at_once = start.toward(goal, damping, 1.0);
	for axis in 0..2 {
		assert_near(travelled[axis], trajectory[axis], "travel");
		assert_near(prediction.velocity[axis], stepped_at_once.velocity[axis], "velocity");
		assert_near(prediction.acceleration[axis], stepped_at_once.acceleration[axis], "acceleration");
	}
}

#[test]
fn ground_roots_face_where_the_facing_node_faces() {
	// The hips turn while a `root` node above them keeps facing -Z.
	let turning = rooted(clip(2.0, |time| ([0.5 * time, 1.0, -time], 0.8 * time), |_| [0.0, -1.0, 0.0]));
	let database = database_from_clips(
		r#"{ clips: ["placeholder"], root: { node: "hips", facing: "root", position_smoothing: 0, facing_smoothing: 0 } }"#,
		&[("turn.animation".into(), &turning)],
	)
	.expect("the clip should form a database");

	for frame in [0, 15, 60] {
		let time = frame as f32 / 30.0;
		let [x, z, yaw] = database.ground[frame];
		assert_near(x, 0.5 * time, "ground x follows the hips");
		assert_near(z, -time, "ground z follows the hips");
		assert_near(yaw, 0.0, "ground yaw follows the facing node");
	}
}
