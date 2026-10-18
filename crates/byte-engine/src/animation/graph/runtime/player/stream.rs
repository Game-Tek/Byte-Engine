//! The animation record a player writes to its [`DataStream`], as the
//! [animation reference](/docs/reference/animation#data-stream) specifies.
//!
//! The record borrows the player's state, so writing one allocates nothing once the stream's line has grown.

use serde::{Serialize, Serializer};

use super::*;
use crate::inspector::DataStream;

#[derive(Serialize)]
struct Record<'a> {
	status: &'static str,
	#[serde(serialize_with = "message")]
	error: Option<&'a AnimationGraphPlayerError>,
	missing: &'a [&'a str],
	request: Request<'a>,
	motion: Option<Motion<'a>>,
	transition: Option<Transition<'a>>,
	pending: Option<Pending<'a>>,
	root_motion: RootMotion,
}

#[derive(Serialize)]
struct Request<'a> {
	state: &'a str,
	position: Position,
	inertialization: f32,
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

impl<'a> Motion<'a> {
	fn new(graph: &'a AnimationGraph, motion: &'a RuntimeMotion<'a>) -> Self {
		let duration = motion.duration();
		Self {
			state: graph.state(motion.state).name.as_str(),
			playback: match motion.playback {
				AnimationPlayback::Loop => "loop",
				AnimationPlayback::Once => "once",
			},
			phase: motion.phase,
			time: motion.phase * duration,
			duration,
			clips: Clips(motion),
		}
	}
}

/// A motion's weighted clips, serialized as a sequence without collecting them.
struct Clips<'a>(&'a RuntimeMotion<'a>);

#[derive(Serialize)]
struct ClipWeight<'a> {
	clip: &'a str,
	weight: f32,
}

#[derive(Serialize)]
struct Transition<'a> {
	from: Motion<'a>,
	elapsed: f32,
	duration: f32,
}

#[derive(Serialize)]
struct Pending<'a> {
	requested: &'a str,
	target: &'a str,
	blend: Option<f32>,
	starts_in: Option<f32>,
}

#[derive(Serialize)]
struct RootMotion {
	translation: [f32; 3],
	rotation: [f32; 4],
}

impl Serialize for Clips<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let motion = self.0;
		serializer.collect_seq(motion.weights.iter().map(|(sample, weight)| ClipWeight {
			clip: &motion.samples[sample].resource_id,
			weight,
		}))
	}
}

/// Serializes a rejection as its message.
fn message<S: Serializer>(error: &Option<&AnimationGraphPlayerError>, serializer: S) -> Result<S::Ok, S::Error> {
	match error {
		Some(error) => serializer.collect_str(error),
		None => serializer.serialize_none(),
	}
}

impl AnimationGraphPlayer<'_> {
	/// Attaches a stream the player writes a record to after every advance, or detaches it with `None`.
	///
	/// Open the stream with
	/// [`GraphicsApplication::data_stream`](crate::application::graphics::GraphicsApplication::data_stream) for the
	/// entity the player animates. A player without a stream does no reporting work.
	pub fn set_stream(&mut self, stream: Option<DataStream>) {
		self.stream = stream;
	}

	/// Writes the player's state after an advance to its stream.
	// Debugging only: kept out of line so it does not weigh on the every-frame path.
	#[cold]
	#[inline(never)]
	pub(super) fn write_stream(&mut self) {
		// The record borrows the player, so the stream steps out while it writes.
		if let Some(mut stream) = self.stream.take() {
			stream.write("animation", &self.record());
			self.stream = Some(stream);
		}
	}

	/// Borrows the player's state as the record its stream carries.
	fn record(&self) -> Record<'_> {
		let graph = self.graph;
		let name = |state: AnimationStateId| graph.state(state).name.as_str();
		let (status, error) = match &self.runtime {
			PlayerRuntime::Loading => ("loading", None),
			PlayerRuntime::Ready(_) => ("ready", None),
			PlayerRuntime::Rejected { error, .. } => ("rejected", Some(error)),
		};
		let ready = self.ready();
		let (playing, transition) = match ready.map(|ready| &ready.playback) {
			None | Some(Playback::AwaitingInitial) => (None, None),
			Some(Playback::Playing(active)) => (Some(active), None),
			Some(Playback::Transitioning(transition)) => (
				Some(&transition.destination),
				Some(Transition {
					from: Motion::new(graph, &transition.source),
					elapsed: transition.elapsed.as_seconds_f32(),
					duration: transition.duration.as_seconds_f32(),
				}),
			),
		};
		let pending = ready.and_then(|ready| {
			let pending = ready.pending.as_ref()?;
			let waiting = match &ready.playback {
				Playback::Playing(active) if !pending.wait.is_empty() => active.time_until(pending.wait),
				_ => None,
			};
			Some(Pending {
				requested: name(pending.requested),
				target: name(pending.target),
				blend: pending.duration.map(MediaTime::as_seconds_f32),
				starts_in: waiting.map(MediaTime::as_seconds_f32),
			})
		});
		let root_motion = ready.map_or(RootMotionDelta::IDENTITY, |ready| ready.root_motion);
		Record {
			status,
			error,
			missing: &self.missing,
			request: Request {
				state: name(self.requested.state),
				position: match self.requested.value {
					BlendValue::Unset => Position::None,
					BlendValue::Linear(value) => Position::Line(value),
					BlendValue::Planar(value) => Position::Plane(value),
				},
				inertialization: self.requested.inertialization.as_seconds_f32(),
			},
			motion: playing.map(|motion| Motion::new(graph, motion)),
			transition,
			pending,
			root_motion: RootMotion {
				translation: root_motion.translation.to_array(),
				rotation: root_motion.rotation.to_array(),
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use std::{sync::Arc, time::Instant};

	use super::super::super::tests::test_animation;
	use super::*;
	use crate::{core::factory::Handle, inspector::StreamOutput};

	#[test]
	fn players_report_loading_playback_and_blends_after_each_advance() {
		let entity = Handle::new();
		let path = std::env::temp_dir().join(format!("byte-engine-animation-stream-{}-{}.ndjson", std::process::id(), entity.id()));
		let output = Arc::new(StreamOutput::open(path.to_str().unwrap(), Instant::now()).expect("file opens"));
		let mut pool = AnimationPool::detached(4096);
		let builder = AnimationGraph::builder();
		let idle = builder.state("idle").with(AnimationClip::looping("idle.animation"));
		let walk = builder.state("walk").with(AnimationClip::looping("walk.animation"));
		idle.to(walk).when(MediaTime::from_millis(500));
		let graph = builder.build(idle).expect("graph should build");
		let mut player = pool.create_player(&graph, None);
		player.set_stream(Some(DataStream::new(output, entity, "Character".into())));

		player.advance(MediaTime::ZERO, idle.id(), &mut pool).expect("waits for the initial clip");
		pool.admit("idle.animation".into(), test_animation("idle", 1.0));
		pool.admit("walk.animation".into(), test_animation("walk", 2.0));
		player.advance(MediaTime::ZERO, idle.id(), &mut pool).expect("enters idle");
		player.advance(MediaTime::from_millis(250), idle.id(), &mut pool).expect("plays idle");
		player.advance(MediaTime::from_millis(100), walk.id(), &mut pool).expect("blends to walk");

		let text = std::fs::read_to_string(&path).expect("file reads");
		std::fs::remove_file(&path).expect("file removes");
		let records: Vec<serde_json::Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
		assert_eq!(records.len(), 4, "one record per advance");
		assert!(records.iter().all(|record| record["stream"] == "animation" && record["name"] == "Character"));

		let loading = &records[0]["data"];
		assert_eq!(loading["status"], "loading");
		assert_eq!(loading["missing"], serde_json::json!(["idle.animation"]));
		assert!(loading["motion"].is_null());

		let playing = &records[2]["data"];
		assert_eq!(playing["status"], "ready");
		assert_eq!(playing["request"]["state"], "idle");
		assert_eq!(playing["motion"]["state"], "idle");
		assert_eq!(playing["motion"]["playback"], "loop");
		assert_eq!(playing["motion"]["phase"], 0.25);
		assert_eq!(playing["motion"]["duration"], 1.0);
		assert_eq!(
			playing["motion"]["clips"],
			serde_json::json!([{ "clip": "idle.animation", "weight": 1.0 }])
		);
		assert!(playing["transition"].is_null());

		let blending = &records[3]["data"];
		assert_eq!(blending["request"]["state"], "walk");
		assert_eq!(blending["motion"]["state"], "walk");
		assert_eq!(blending["transition"]["from"]["state"], "idle");
		assert_eq!(blending["transition"]["duration"], 0.5);
		assert!(blending["pending"].is_null());
	}
}
