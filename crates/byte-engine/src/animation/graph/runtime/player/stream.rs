//! The animation record a player writes to its [`DebugDataStreamWriter`], as the
//! [animation reference](/docs/reference/animation#data-stream) specifies.
//!
//! The record borrows the player's state, so writing one allocates nothing once the stream's line has grown.

use std::fmt;

use serde::{Serialize, Serializer};

use super::*;
use crate::inspector::DebugDataStreamWriter;

#[derive(Serialize)]
struct Record<'a> {
	status: &'static str,
	#[serde(serialize_with = "message")]
	error: Option<&'a AnimationGraphPlayerError>,
	#[serde(serialize_with = "resource_ids")]
	missing: &'a [Arc<str>],
	request: Request<'a>,
	motion: Option<Motion<'a>>,
	transition: Option<Transition<'a>>,
	pending: Option<Pending<'a>>,
	#[serde(rename = "match")]
	matched: Option<Match<'a>>,
	root_motion: RootMotion,
	look: Option<super::look::LookReport>,
}

/// The part of a [`Record`] whose change warrants a record in a changes-only stream.
#[derive(Serialize)]
struct Key<'a> {
	status: &'static str,
	#[serde(serialize_with = "resource_ids")]
	missing: &'a [Arc<str>],
	request: &'a str,
	motion: Option<(&'a str, &'a Clips<'a>)>,
	transition: Option<(&'a str, &'a Route<'a>)>,
	pending: Option<(&'a str, &'a str, &'a Route<'a>, Option<ClipName<'a>>)>,
}

#[derive(Serialize)]
struct Request<'a> {
	state: &'a str,
	position: Position,
	velocity: Option<[f32; 2]>,
	facing: Option<f32>,
	inertialization: f32,
}

/// A matched state's latest search.
#[derive(Serialize)]
struct Match<'a> {
	searched: bool,
	bar: Option<f32>,
	found: Option<Frame<'a>>,
	cost: Option<f32>,
	trajectory: &'a [crate::animation::matching::search::TrajectorySample],
}

/// A frame of a motion database: its clip and its time in the clip.
#[derive(Serialize)]
struct Frame<'a> {
	clip: ClipName<'a>,
	time: f32,
}

/// A blend position, which serializes as null, a number, or a pair of numbers.
#[derive(Serialize)]
#[serde(untagged)]
enum Position {
	None,
	Line(f32),
	Plane([f32; 2]),
}

#[derive(Serialize)]
struct Motion<'a> {
	state: &'a str,
	playback: &'static str,
	phase: f32,
	time: f32,
	duration: f32,
	clips: Clips<'a>,
}

/// A motion's weighted clips, serialized as a sequence without collecting them.
struct Clips<'a>(&'a AnimationGraph, &'a RuntimeMotion);

#[derive(Serialize)]
struct ClipWeight<'a> {
	clip: ClipName<'a>,
	weight: f32,
}

/// A clip's short name: its resource ID's file stem, then `#` and the clip it picks when its fragment names one.
#[derive(Clone, Copy)]
struct ClipName<'a>(&'a str);

#[derive(Serialize)]
struct Transition<'a> {
	from: Motion<'a>,
	route: Route<'a>,
	elapsed: f32,
	duration: f32,
}

#[derive(Serialize)]
struct Pending<'a> {
	requested: &'a str,
	target: &'a str,
	blend: Option<f32>,
	route: Route<'a>,
	wait: Option<Wait<'a>>,
}

#[derive(Serialize)]
struct Route<'a> {
	kind: &'static str,
	owner: Option<&'a str>,
}

#[derive(Serialize)]
struct Wait<'a> {
	clip: ClipName<'a>,
	weight: f32,
	point: Option<Point>,
	starts_in: Option<f32>,
}

/// A wait point as authored, which serializes as `{"frame": n}` or `{"seconds": s}`.
#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum Point {
	Frame(u32),
	Seconds(f32),
}

#[derive(Serialize)]
struct RootMotion {
	translation: [f32; 3],
	rotation: [f32; 4],
}

impl<'a> Motion<'a> {
	fn new(graph: &'a AnimationGraph, motion: &'a RuntimeMotion) -> Self {
		let duration = motion.duration(motion.samples(graph));
		Self {
			state: graph.state(motion.state).name.as_str(),
			playback: match motion.playback {
				AnimationPlayback::Loop => "loop",
				AnimationPlayback::Once => "once",
			},
			phase: motion.phase,
			time: motion.phase * duration,
			duration,
			clips: Clips(graph, motion),
		}
	}
}

impl Serialize for Clips<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let Self(graph, motion) = *self;
		let samples = motion.samples(graph);
		serializer.collect_seq(motion.weights.iter().map(|(sample, weight)| ClipWeight {
			clip: ClipName(&samples[sample].resource_id),
			weight,
		}))
	}
}

impl fmt::Display for ClipName<'_> {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		let (path, fragment) = self
			.0
			.split_once('#')
			.map_or((self.0, None), |(path, fragment)| (path, Some(fragment)));
		let file = path.rsplit_once('/').map_or(path, |(_, file)| file);
		formatter.write_str(file.rsplit_once('.').map_or(file, |(stem, _)| stem))?;
		// `#animation` picks the file's first clip, so only a fragment that names one adds to the stem.
		match fragment.and_then(|fragment| fragment.strip_prefix("animations/")) {
			Some(clip) => write!(formatter, "#{clip}"),
			None => Ok(()),
		}
	}
}

impl Serialize for ClipName<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.collect_str(self)
	}
}

impl<'a> Route<'a> {
	fn new(graph: &'a AnimationGraph, origin: RouteOrigin) -> Self {
		let state = |index: usize| Some(graph.states[index].name.as_str());
		let (kind, owner) = match origin {
			RouteOrigin::State(index) => ("state", state(index)),
			RouteOrigin::Space(index) => ("space", Some(graph.spaces[index].name.as_str())),
			RouteOrigin::AnyState => ("any-state", None),
			RouteOrigin::Completion(index) => ("completion", state(index)),
			RouteOrigin::Position => ("position", None),
			RouteOrigin::Initial => ("initial", None),
		};
		Self { kind, owner }
	}
}

impl<'a> Wait<'a> {
	/// Describes the wait of a route that waits for `points` of the playing `motion`.
	fn new(graph: &'a AnimationGraph, motion: &'a RuntimeMotion, points: &[ClipPoint]) -> Option<Self> {
		let clip = motion.heaviest(points)?;
		let next = motion.time_until(motion.samples(graph), points);
		Some(Self {
			clip: ClipName(&motion.samples(graph)[clip].resource_id),
			weight: motion.weight_of(clip),
			point: next.map(|(_, point)| match point.time {
				ClipTime::Frame(frame) => Point::Frame(frame),
				ClipTime::Seconds(seconds) => Point::Seconds(seconds),
			}),
			starts_in: next.map(|(time, _)| time.as_seconds_f32()),
		})
	}
}

impl<'a> Record<'a> {
	/// Borrows the parts of the record a changes-only stream compares.
	fn key(&'a self) -> Key<'a> {
		Key {
			status: self.status,
			missing: self.missing,
			request: self.request.state,
			motion: self.motion.as_ref().map(|motion| (motion.state, &motion.clips)),
			transition: self
				.transition
				.as_ref()
				.map(|transition| (transition.from.state, &transition.route)),
			pending: self.pending.as_ref().map(|pending| {
				(
					pending.requested,
					pending.target,
					&pending.route,
					pending.wait.as_ref().map(|wait| wait.clip),
				)
			}),
		}
	}
}

/// Serializes a rejection as its message.
/// Writes the resource IDs of missing clips as a list of strings.
fn resource_ids<S: Serializer>(ids: &&[Arc<str>], serializer: S) -> Result<S::Ok, S::Error> {
	serializer.collect_seq(ids.iter().map(|id| &**id))
}

fn message<S: Serializer>(error: &Option<&AnimationGraphPlayerError>, serializer: S) -> Result<S::Ok, S::Error> {
	match error {
		Some(error) => serializer.collect_str(error),
		None => serializer.serialize_none(),
	}
}

impl AnimationGraphPlayer {
	/// Attaches a stream the player writes a record to after every advance, or detaches it with `None`.
	///
	/// The animation system attaches writers created from world-spawned
	/// [`DebugDataStream`](crate::inspector::DebugDataStream) requests. A player without a writer does no reporting work.
	pub fn set_stream(&mut self, stream: Option<DebugDataStreamWriter>) {
		self.stream = stream;
	}

	/// Writes the player's state after an advance to its stream.
	// Debugging only: kept out of line so it does not weigh on the every-frame path.
	#[cold]
	#[inline(never)]
	pub(super) fn write_stream(&mut self, graph: &AnimationGraph) {
		// The record borrows the player, so the stream steps out while it writes.
		if let Some(mut stream) = self.stream.take() {
			let record = self.record(graph);
			stream.write("animation", &record.key(), &record);
			self.stream = Some(stream);
		}
	}

	/// Borrows the player's state as the record its stream carries.
	fn record<'a>(&'a self, graph: &'a AnimationGraph) -> Record<'a> {
		let name = |state: AnimationStateId| graph.state(state).name.as_str();
		let (status, error) = match &self.runtime {
			PlayerRuntime::Loading => ("loading", None),
			PlayerRuntime::Ready(_) => ("ready", None),
			PlayerRuntime::Rejected { error, .. } => ("rejected", Some(error)),
		};
		let ready = self.ready();
		let (playing, transition) = match ready.map(|ready| &ready.playback) {
			None | Some(Playback::AwaitingInitial) => (None, None),
			Some(Playback::Playing(active, _)) => (Some(active), None),
			Some(Playback::Transitioning(transition)) => (
				Some(&transition.destination),
				Some(Transition {
					from: Motion::new(graph, &transition.source),
					route: Route::new(graph, transition.origin),
					elapsed: transition.elapsed.as_seconds_f32(),
					duration: transition.duration.as_seconds_f32(),
				}),
			),
		};
		let pending = ready.and_then(|ready| match &ready.playback {
			// The initial state cuts in, with nothing to blend from, once its clips load.
			Playback::AwaitingInitial => Some(Pending {
				requested: name(graph.initial_state()),
				target: name(graph.initial_state()),
				blend: None,
				route: Route::new(graph, RouteOrigin::Initial),
				wait: None,
			}),
			Playback::Playing(active, Some(route)) => Some(Pending {
				requested: name(route.requested),
				target: name(route.target),
				blend: Some(route.duration.as_seconds_f32()),
				route: Route::new(graph, route.origin),
				wait: Wait::new(graph, active, graph.points(route.wait)),
			}),
			Playback::Playing(_, None) | Playback::Transitioning(_) => None,
		});
		let root_motion = ready.map_or(RootMotionDelta::IDENTITY, |ready| ready.root_motion);
		// Only a matched state, playing or blending in, reports its search.
		let matched = ready.zip(playing).and_then(|(ready, motion)| {
			let samples = motion.samples(graph);
			if !is_matched(samples) {
				return None;
			}
			let record = &ready.matched.as_deref()?.search;
			let found = record.result.found;
			Some(Match {
				searched: record.searched,
				bar: record.result.bar,
				found: found.map(|found| Frame {
					clip: ClipName(&samples[found.clip].resource_id),
					time: found.time,
				}),
				cost: found.map(|found| found.cost),
				trajectory: &record.trajectory[..record.trajectory_len],
			})
		});
		let (velocity, facing) = match self.requested.value {
			BlendValue::Motion { velocity, facing } => (Some(velocity), Some(facing)),
			_ => (None, None),
		};
		Record {
			status,
			error,
			missing: &self.missing,
			request: Request {
				state: name(self.requested.state),
				position: match self.requested.value {
					BlendValue::Unset | BlendValue::Motion { .. } => Position::None,
					BlendValue::Linear(value) => Position::Line(value),
					BlendValue::Planar(value) => Position::Plane(value),
				},
				velocity,
				facing,
				inertialization: self.requested.inertialization.as_seconds_f32(),
			},
			motion: playing.map(|motion| Motion::new(graph, motion)),
			transition,
			pending,
			matched,
			root_motion: RootMotion {
				translation: root_motion.translation.to_array(),
				rotation: root_motion.rotation.to_array(),
			},
			look: match &self.runtime {
				PlayerRuntime::Ready(ready) => ready.pose.look.as_ref().and_then(|chain| chain.report),
				_ => None,
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use std::{path::PathBuf, sync::Arc, time::Instant};

	use super::super::super::tests::{packed_test_animation_bytes, test_animation};
	use super::*;
	use crate::{core::factory::Handle, inspector::StreamOutput};

	/// Opens a stream for a new entity to a temporary file, returned with the file's path.
	fn stream(changes_only: bool) -> (DebugDataStreamWriter, PathBuf) {
		let entity = Handle::new();
		let path = std::env::temp_dir().join(format!(
			"byte-engine-animation-stream-{}-{}.ndjson",
			std::process::id(),
			entity.id()
		));
		let output = Arc::new(StreamOutput::open(path.to_str(), changes_only, Instant::now(), Arc::default()).expect("file opens"));
		(DebugDataStreamWriter::new(output, entity, "Character".into()), path)
	}

	/// Reads and removes the records at `path`, returning each one's `data`.
	fn records(path: PathBuf) -> Vec<serde_json::Value> {
		let text = std::fs::read_to_string(&path).expect("file reads");
		std::fs::remove_file(&path).expect("file removes");
		text.lines()
			.map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["data"].take())
			.collect()
	}

	/// Builds a pool holding a slow and a fast clip, under IDs of a single-clip file and of a pack, and an idle clip.
	fn speed_pool() -> AnimationPool {
		let clips = [
			("anims/slow.fbx#animation", 1.0),
			("pack.glb#animations/Fast", 2.0),
			("idle.animation", 0.0),
		];
		let mut pool = AnimationPool::detached(clips.iter().map(|&(id, end)| packed_test_animation_bytes(id, end)).sum());
		for (id, end) in clips {
			pool.admit(id.into(), test_animation(id, end));
		}
		pool
	}

	#[test]
	fn a_matched_search_starts_its_trajectory_from_the_predicted_velocity() {
		use super::super::matched::tests::{DATABASE, locomotion_pool};

		let (stream, path) = stream(false);
		let mut pool = locomotion_pool("");
		let builder = AnimationGraph::builder();
		let locomotion = builder.state("locomotion").matched(DATABASE);
		let graph = builder.build(locomotion).expect("graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("hips")));
		player.set_stream(Some(stream));
		let step = MediaTime::from_frames(1, 30).expect("the engine timebase represents 30 Hz exactly");
		// A second walking ahead, then a second asking to move sideways, which no clip of the database does.
		for velocity in [[0.0, -1.4], [-1.4, 0.0]] {
			for _ in 0..30 {
				player
					.advance(&graph, step, locomotion.id().toward(velocity, 0.0), &mut pool)
					.expect("plays the matched state");
			}
		}

		let records = records(path);
		let search = records
			.iter()
			.rev()
			.find(|record| record["match"]["searched"] == true)
			.expect("the state searches every tenth of a second");
		let first = &search["match"]["trajectory"][0];
		let number = |value: &serde_json::Value| value.as_f64().expect("a number");
		let (time, x, z) = (
			number(&first["time"]),
			number(&first["position"][0]),
			number(&first["position"][1]),
		);
		// Whatever the clips play, the prediction has nearly reached the sideways velocity.
		assert!(
			x < -1.4 * time * 0.9 && z.abs() < 0.03,
			"the trajectory starts toward ({x}, {z})"
		);
	}

	#[test]
	fn matched_states_report_their_last_search_and_the_requested_motion() {
		use super::super::matched::tests::{DATABASE, locomotion_pool};

		let (stream, path) = stream(false);
		let mut pool = locomotion_pool("");
		let builder = AnimationGraph::builder();
		let locomotion = builder.state("locomotion").matched(DATABASE);
		let graph = builder.build(locomotion).expect("graph should build");
		let mut player = pool.create_player(&graph, Some(RootMotionSettings::full("hips")));
		player.set_stream(Some(stream));
		let request = locomotion.id().toward([0.0, -1.5], 0.5);
		for _ in 0..2 {
			player
				.advance(&graph, MediaTime::from_millis(30), request, &mut pool)
				.expect("plays the matched state");
		}

		let records = records(path);
		let number = |value: &serde_json::Value| value.as_f64().expect("a number");
		let entered = &records[0];
		assert_eq!(entered["request"]["velocity"], serde_json::json!([0.0, -1.5]));
		assert_eq!(entered["request"]["facing"], 0.5);
		assert!(entered["request"]["position"].is_null());
		let search = &entered["match"];
		assert_eq!(search["searched"], true);
		assert!(search["bar"].is_null(), "entering the state sets no bar");
		assert_eq!(search["found"]["clip"], "walk");
		assert!(number(&search["cost"]) >= 0.0);
		let times: Vec<f64> = search["trajectory"]
			.as_array()
			.expect("a trajectory")
			.iter()
			.map(|sample| number(&sample["time"]))
			.collect();
		assert_eq!(times.len(), 3, "one sample for each trajectory time");
		assert!((times[2] - 1.0).abs() < 1.0e-6);
		assert_eq!(
			records[1]["match"]["searched"], false,
			"the next step searches only once the interval elapses"
		);
	}

	#[test]
	fn players_report_loading_playback_and_blends_after_each_advance() {
		let (stream, path) = stream(false);
		let mut pool = AnimationPool::detached(4096);
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk).when(MediaTime::from_millis(500));
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, None);
		player.set_stream(Some(stream));

		player
			.advance(&graph, MediaTime::ZERO, idle.id(), &mut pool)
			.expect("waits for the initial clip");
		pool.admit("idle.animation".into(), test_animation("idle", 1.0));
		pool.admit("walk.animation".into(), test_animation("walk", 2.0));
		player
			.advance(&graph, MediaTime::ZERO, idle.id(), &mut pool)
			.expect("enters idle");
		player
			.advance(&graph, MediaTime::from_millis(250), idle.id(), &mut pool)
			.expect("plays idle");
		player
			.advance(&graph, MediaTime::from_millis(100), walk.id(), &mut pool)
			.expect("blends to walk");

		let records = records(path);
		assert_eq!(records.len(), 4, "one record per advance");

		let loading = &records[0];
		assert_eq!(loading["status"], "loading");
		assert_eq!(loading["missing"], serde_json::json!(["idle.animation"]));
		assert!(loading["motion"].is_null());

		let playing = &records[2];
		assert_eq!(playing["status"], "ready");
		assert_eq!(playing["request"]["state"], "idle");
		assert_eq!(playing["motion"]["state"], "idle");
		assert_eq!(playing["motion"]["playback"], "loop");
		assert_eq!(playing["motion"]["phase"], 0.25);
		assert_eq!(playing["motion"]["duration"], 1.0);
		assert_eq!(
			playing["motion"]["clips"],
			serde_json::json!([{ "clip": "idle", "weight": 1.0 }])
		);
		assert!(playing["transition"].is_null());

		let blending = &records[3];
		assert_eq!(blending["request"]["state"], "walk");
		assert_eq!(blending["motion"]["state"], "walk");
		assert_eq!(blending["transition"]["from"]["state"], "idle");
		assert_eq!(
			blending["transition"]["route"],
			serde_json::json!({ "kind": "state", "owner": "idle" })
		);
		assert_eq!(blending["transition"]["duration"], 0.5);
		assert!(blending["pending"].is_null());
	}

	#[test]
	fn waiting_routes_report_the_clip_whose_frames_they_follow() {
		let (stream, path) = stream(false);
		let mut pool = speed_pool();
		let builder = AnimationGraph::builder();
		let speed = builder.state("speed").linear(
			AnimationBlend::new()
				.sample(0.0, AnimationClip::looping("anims/slow.fbx#animation"))
				.sample(1.0, AnimationClip::looping("pack.glb#animations/Fast")),
		);
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		// Only the fast clip names a frame, so at the slow end the route follows a clip that doesn't play.
		speed
			.to(idle)
			.at_frames("pack.glb#animations/Fast", [7])
			.anytime(MediaTime::ZERO);
		let graph = builder.build(speed).expect("graph should build");
		let mut player = pool.create_player(&graph, None);
		player.set_stream(Some(stream));

		player
			.advance(&graph, MediaTime::ZERO, speed.id().at(0.0), &mut pool)
			.expect("plays the slow end");
		player
			.advance(&graph, MediaTime::ZERO, idle.id(), &mut pool)
			.expect("waits for the fast clip's frame");

		let records = records(path);
		assert_eq!(
			records[0]["motion"]["clips"],
			serde_json::json!([{ "clip": "slow", "weight": 1.0 }])
		);
		let pending = &records[1]["pending"];
		assert_eq!(pending["requested"], "idle");
		assert_eq!(pending["route"], serde_json::json!({ "kind": "state", "owner": "speed" }));
		assert_eq!(
			pending["wait"],
			serde_json::json!({ "clip": "pack#Fast", "weight": 0.0, "point": { "frame": 7 }, "starts_in": 0.7 })
		);
	}

	#[test]
	fn changes_only_players_write_when_the_state_weights_or_pending_route_change() {
		let (stream, path) = stream(true);
		let mut pool = speed_pool();
		let builder = AnimationGraph::builder();
		let speed = builder.state("speed").linear(
			AnimationBlend::new()
				.sample(0.0, AnimationClip::looping("anims/slow.fbx#animation"))
				.sample(1.0, AnimationClip::looping("pack.glb#animations/Fast")),
		);
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		speed
			.to(idle)
			.at_frames("pack.glb#animations/Fast", [7])
			.anytime(MediaTime::ZERO);
		let graph = builder.build(speed).expect("graph should build");
		let mut player = pool.create_player(&graph, None);
		player.set_stream(Some(stream));
		let step = MediaTime::from_millis(100);

		player
			.advance(&graph, MediaTime::ZERO, speed.id().at(0.25), &mut pool)
			.expect("plays the blend");
		player
			.advance(&graph, step, speed.id().at(0.25), &mut pool)
			.expect("only the phase moves");
		player
			.advance(&graph, step, speed.id().at(0.75), &mut pool)
			.expect("the weights change");
		player.advance(&graph, step, idle.id(), &mut pool).expect("a route waits");
		player
			.advance(&graph, step, idle.id(), &mut pool)
			.expect("only the wait comes closer");

		let records = records(path);
		let weights: Vec<_> = records
			.iter()
			.map(|record| record["motion"]["clips"][0]["weight"].as_f64().unwrap())
			.collect();
		assert_eq!(weights, [0.75, 0.25, 0.25]);
		assert!(records[1]["pending"].is_null());
		assert_eq!(records[2]["pending"]["requested"], "idle");
	}

	#[test]
	fn one_shot_states_are_named_after_their_route() {
		let (stream, path) = stream(false);
		let mut pool = speed_pool();
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("anims/slow.fbx#animation"));
		idle.to(walk)
			.with(AnimationClip::once("pack.glb#animations/Fast"))
			.when(MediaTime::ZERO);
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, None);
		player.set_stream(Some(stream));

		player
			.advance(&graph, MediaTime::ZERO, idle.id(), &mut pool)
			.expect("plays idle");
		player
			.advance(&graph, MediaTime::ZERO, walk.id(), &mut pool)
			.expect("starts the one-shot");

		assert_eq!(records(path)[1]["motion"]["state"], "idle -> walk");
	}
}
