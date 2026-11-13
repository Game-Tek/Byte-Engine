//! Per-entity debugging data streams, written as NDJSON.
//!
//! Create a [`DebugDataStream`] through the world's message bus to request records for an entity. The output
//! service supplies writers to subscribed systems without giving gameplay a reference to those systems.
//! The [debug data streams reference](/docs/reference/debug-data-streams) specifies the destinations and records.

use std::{
	borrow::Cow,
	io::Write,
	sync::{
		Arc,
		atomic::{AtomicU64, Ordering},
	},
	time::Instant,
};

use serde::Serialize;
use utils::{hash::HashMap, sync::Mutex};

use crate::core::{
	factory::{CreateMessage, Factory, Handle},
	listener::{DefaultListener, Listener as _},
	message::DeleteMessage,
	message_bus::MessageScope,
};

/// The `DebugDataStream` struct requests debugging records for one entity from systems that hold its state.
///
/// Spawn it with `world.create(DebugDataStream::new(entity, "Character"))` after creating the consuming systems.
/// The `debug.streams` destination enables reporting. Deleting this DDS entity stops its records.
#[derive(Clone)]
pub struct DebugDataStream {
	entity: Handle,
	name: Cow<'static, str>,
}

impl DebugDataStream {
	/// Requests records for `entity`, labeled `name` in every record.
	pub fn new(entity: Handle, name: impl Into<Cow<'static, str>>) -> Self {
		Self {
			entity,
			name: name.into(),
		}
	}
}

/// The `DebugDataStreams` struct connects world-spawned DDS entities to the configured output destination.
pub(crate) struct DebugDataStreams {
	creations: DefaultListener<CreateMessage<DebugDataStream>>,
	writers: Factory<DebugDataStreamWriter>,
	output: Option<Arc<StreamOutput>>,
	deletions: DefaultListener<DeleteMessage>,
	/// Reused to suppress requests deleted before this service observed their creation.
	deleted: Vec<Handle>,
}

impl DebugDataStreams {
	/// Subscribes before gameplay creates DDS entities, using no writers when output is disabled.
	pub(crate) fn new(world: &MessageScope, output: Option<Arc<StreamOutput>>) -> Self {
		Self {
			creations: world.factory().listener(),
			writers: world.factory(),
			output,
			deletions: world.channel::<DeleteMessage>().listener(),
			deleted: Vec::new(),
		}
	}

	/// Publishes a writer under each DDS entity's handle; the writer names the entity whose state it reports.
	pub(crate) fn update(&mut self) {
		self.deleted.clear();
		while let Some(deletion) = self.deletions.read() {
			if self.output.is_some() {
				self.deleted.push(*deletion.handle());
			}
		}
		while let Some(created) = self.creations.read() {
			let Some(output) = &self.output else { continue };
			let request = created.data();
			// This scans the frame's deletions for every creation. It runs only in development builds with stream
			// output enabled, and a frame that deletes many entities, such as a level unload, creates almost none, so
			// the two lists rarely grow together.
			if self.deleted.contains(&created.handle()) {
				continue;
			}
			self.writers.derive(
				created.handle(),
				DebugDataStreamWriter::new(Arc::clone(output), request.entity, request.name.clone()),
			);
		}
	}

	/// Sets the tick attached to every subsequent record.
	pub(crate) fn set_tick(&self, tick: u64) {
		if let Some(output) = &self.output {
			output.set_tick(tick);
		}
	}
}

/// The `StreamOutput` struct is where every data stream of an application writes: the inspector, which keeps each
/// stream's latest record, and the `debug.streams` destination, when one is set.
pub(crate) struct StreamOutput {
	/// The `debug.streams` destination; `None` when only the inspector reads the streams, such as for the console.
	destination: Option<Destination>,
	/// Keeps each stream's latest record for the inspector.
	retained: Arc<RetainedStreams>,
	/// Whether streams skip records whose key matches the last one they wrote, from `debug.streams.changes-only`.
	changes_only: bool,
	/// The application tick in progress, which every record carries; see [`Self::set_tick`].
	tick: AtomicU64,
	/// The application's start, which record times count from, like the inspector's metrics.
	start: Instant,
}

/// The `Destination` enum is where a `debug.streams` value sends records.
enum Destination {
	Log,
	/// Locked so lines from streams on different threads never interleave, which a pipe only guarantees for short
	/// writes.
	Writer(Mutex<Box<dyn Write + Send>>),
}

impl StreamOutput {
	/// Opens the destination a `debug.streams` value names, if any: `log`, `stdout`, `stderr`, or a file path, which it
	/// creates or truncates. Every record also reaches `retained`.
	pub(crate) fn open(
		destination: Option<&str>,
		changes_only: bool,
		start: Instant,
		retained: Arc<RetainedStreams>,
	) -> std::io::Result<Self> {
		let destination = match destination {
			None => None,
			Some("log") => Some(Destination::Log),
			Some("stdout") => Some(Destination::Writer(Mutex::new(Box::new(std::io::stdout())))),
			Some("stderr") => Some(Destination::Writer(Mutex::new(Box::new(std::io::stderr())))),
			Some(path) => Some(Destination::Writer(Mutex::new(Box::new(std::fs::File::create(path)?)))),
		};
		Ok(Self {
			destination,
			retained,
			changes_only,
			tick: AtomicU64::new(0),
			start,
		})
	}

	/// Sets the tick that records written from now on carry. The application calls it as each tick starts.
	pub(crate) fn set_tick(&self, tick: u64) {
		self.tick.store(tick, Ordering::Relaxed);
	}
}

/// The `DebugDataStreamWriter` struct keeps a system's record buffers and its connection to the debug destination.
///
/// Listen for its creation on the world scope after a [`DebugDataStream`] is spawned. Use [`Self::entity`] to
/// match it to your system's state, then call [`Self::write`] after each update. Each clone has independent
/// record and change-detection buffers.
pub struct DebugDataStreamWriter {
	output: Arc<StreamOutput>,
	entity: Handle,
	name: Cow<'static, str>,
	/// The line being written, kept so steady-state records allocate nothing.
	line: Vec<u8>,
	/// The key of the last record written, which changes-only streams compare the next key with.
	key: Vec<u8>,
}

impl Clone for DebugDataStreamWriter {
	fn clone(&self) -> Self {
		// Each subscriber starts fresh; cloning a writer must not copy its scratch buffers or suppress its first record.
		Self::new(Arc::clone(&self.output), self.entity, self.name.clone())
	}
}

/// The fields every record carries, around the system's `data`.
#[derive(Serialize)]
struct Record<'a, T> {
	tick: u64,
	time: f64,
	entity: u32,
	name: &'a str,
	stream: &'a str,
	data: &'a T,
}

impl DebugDataStreamWriter {
	/// Returns the entity whose state this writer reports.
	pub fn entity(&self) -> Handle {
		self.entity
	}

	/// Creates a stream for `entity`, labeled `name` in every record, that writes to `output`.
	pub(crate) fn new(output: Arc<StreamOutput>, entity: Handle, name: Cow<'static, str>) -> Self {
		Self {
			output,
			entity,
			name,
			line: Vec::new(),
			key: Vec::new(),
		}
	}

	/// Writes `data` as one record of `stream`, the name of the system writing it.
	///
	/// `key` is the part of the system's state whose change warrants a record: with `debug.streams.changes-only`
	/// set, a record whose key matches the last one written is skipped. It blocks until the destination takes the
	/// whole line, so a slow reader slows the system instead of losing records.
	pub fn write(&mut self, stream: &str, key: &impl Serialize, data: &impl Serialize) {
		if self.output.changes_only {
			self.line.clear();
			serde_json::to_writer(&mut self.line, key).expect("stream keys serialize to JSON");
			if self.line == self.key {
				return;
			}
			std::mem::swap(&mut self.line, &mut self.key);
		}
		self.line.clear();
		let record = Record {
			tick: self.output.tick.load(Ordering::Relaxed),
			time: self.output.start.elapsed().as_secs_f64() * 1000.0,
			entity: self.entity.id(),
			name: &self.name,
			stream,
			data,
		};
		serde_json::to_writer(&mut self.line, &record).expect("stream records serialize to JSON");
		self.output.retained.retain(self.entity.id(), stream, &self.line);
		let writer = match &self.output.destination {
			None => return,
			Some(Destination::Log) => {
				let line = std::str::from_utf8(&self.line).expect("serde_json writes UTF-8");
				log::info!(target: "byte_engine::stream", "{line}");
				return;
			}
			Some(Destination::Writer(writer)) => writer,
		};
		self.line.push(b'\n');
		if let Err(error) = writer.lock().write_all(&self.line) {
			log::warn!(
				"Data stream record could not be written. The most likely cause is that the reader of `debug.streams` closed it: {error}"
			);
		}
	}
}

/// The `RetainedStreams` struct keeps the latest record of each entity's stream, so the inspector can show the state
/// systems last reported, such as the console's bakes view.
#[derive(Default)]
pub(crate) struct RetainedStreams {
	/// Each entity's streams, by entity handle; an entity has a few streams at most, so they're searched in order.
	records: Mutex<HashMap<u32, Vec<RetainedRecord>>>,
	/// Counts every record retained, so readers redraw only when one arrived.
	revision: AtomicU64,
}

/// One stream's latest record.
struct RetainedRecord {
	stream: String,
	/// The whole record, as the destination receives it, without its line feed. Reused, so steady state allocates
	/// nothing.
	line: Vec<u8>,
	received: Instant,
}

impl RetainedStreams {
	/// Keeps `line` as the latest record of `entity`'s `stream`.
	fn retain(&self, entity: u32, stream: &str, line: &[u8]) {
		let mut records = self.records.lock();
		let streams = records.entry(entity).or_default();
		let index = match streams.iter().position(|record| record.stream == stream) {
			Some(index) => index,
			None => {
				streams.push(RetainedRecord {
					stream: stream.to_string(),
					line: Vec::new(),
					received: Instant::now(),
				});
				streams.len() - 1
			}
		};
		let record = &mut streams[index];
		record.line.clear();
		record.line.extend_from_slice(line);
		record.received = Instant::now();
		self.revision.fetch_add(1, Ordering::SeqCst);
	}

	/// Returns the newest record any entity wrote to `stream`, parsed, with when it arrived.
	#[cfg_attr(not(feature = "headed"), allow(dead_code))]
	pub(crate) fn latest(&self, stream: &str) -> Option<(Instant, serde_json::Value)> {
		let records = self.records.lock();
		let newest = records
			.values()
			.flatten()
			.filter(|record| record.stream == stream)
			.max_by_key(|record| record.received)?;
		let value = serde_json::from_slice(&newest.line).expect("retained records are the JSON the writer serialized");
		Some((newest.received, value))
	}

	/// Returns a number that changes whenever a record is retained.
	#[cfg_attr(not(feature = "headed"), allow(dead_code))]
	pub(crate) fn revision(&self) -> u64 {
		self.revision.load(Ordering::SeqCst)
	}
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::*;

	/// Opens a stream for a new entity named `Box` to a temporary file, returned with its output and the file's path.
	fn stream(changes_only: bool) -> (DebugDataStreamWriter, Arc<StreamOutput>, PathBuf) {
		let entity = Handle::new();
		let path = std::env::temp_dir().join(format!("byte-engine-stream-{}-{}.ndjson", std::process::id(), entity.id()));
		let output =
			Arc::new(StreamOutput::open(path.to_str(), changes_only, Instant::now(), Arc::default()).expect("file opens"));
		(
			DebugDataStreamWriter::new(Arc::clone(&output), entity, "Box".into()),
			output,
			path,
		)
	}

	/// Reads and removes the file at `path`, checking that it holds one JSON object per line.
	fn records(path: PathBuf) -> Vec<serde_json::Value> {
		let text = std::fs::read_to_string(&path).expect("file reads");
		std::fs::remove_file(&path).expect("file removes");
		assert!(text.ends_with('\n'));
		text.lines()
			.map(|line| serde_json::from_str(line).expect("each line is one JSON object"))
			.collect()
	}

	#[test]
	fn records_are_one_json_object_per_line_with_the_entity_envelope() {
		let (mut stream, output, path) = stream(false);

		output.set_tick(7);
		stream.write("physics", &(), &serde_json::json!({ "speed": 0.5 }));
		stream.write("physics", &(), &serde_json::json!({ "speed": f32::NAN }));

		let records = records(path);
		assert_eq!(records.len(), 2);
		assert_eq!(records[0]["tick"], 7);
		assert_eq!(records[0]["entity"], stream.entity.id());
		assert_eq!(records[0]["name"], "Box");
		assert_eq!(records[0]["stream"], "physics");
		assert_eq!(records[0]["data"]["speed"], 0.5);
		assert!(records[0]["time"].as_f64().is_some_and(|time| time >= 0.0));
		assert!(records[1]["data"]["speed"].is_null());
	}

	#[test]
	fn changes_only_streams_skip_records_whose_key_repeats() {
		let (mut stream, _, path) = stream(true);

		for (key, speed) in [("rest", 0.0), ("rest", 0.1), ("moving", 0.2), ("moving", 0.3), ("rest", 0.4)] {
			stream.write("physics", &key, &serde_json::json!({ "speed": speed }));
		}

		let speeds: Vec<_> = records(path)
			.iter()
			.map(|record| record["data"]["speed"].as_f64().unwrap())
			.collect();
		assert_eq!(speeds, [0.0, 0.2, 0.4]);
	}

	#[test]
	fn the_latest_record_of_each_stream_is_retained_without_a_destination() {
		let retained = Arc::new(RetainedStreams::default());
		let output = Arc::new(StreamOutput::open(None, false, Instant::now(), Arc::clone(&retained)).expect("opens"));
		let mut first = DebugDataStreamWriter::new(Arc::clone(&output), Handle::new(), "A".into());
		let mut second = DebugDataStreamWriter::new(Arc::clone(&output), Handle::new(), "B".into());
		assert!(retained.latest("bakes").is_none());

		first.write("bakes", &(), &serde_json::json!({ "baked": 1 }));
		first.write("bakes", &(), &serde_json::json!({ "baked": 2 }));
		first.write("physics", &(), &serde_json::json!({ "speed": 0.5 }));
		let revision = retained.revision();
		second.write("bakes", &(), &serde_json::json!({ "baked": 7 }));

		assert!(retained.revision() > revision);
		let (_, latest) = retained.latest("bakes").expect("a retained record");
		assert_eq!(latest["name"], "B");
		assert_eq!(latest["data"]["baked"], 7);
		assert_eq!(retained.latest("physics").expect("a retained record").1["data"]["speed"], 0.5);
	}

	#[test]
	fn changes_only_streams_retain_only_records_whose_key_changed() {
		let retained = Arc::new(RetainedStreams::default());
		let output = Arc::new(StreamOutput::open(None, true, Instant::now(), Arc::clone(&retained)).expect("opens"));
		let mut stream = DebugDataStreamWriter::new(output, Handle::new(), "A".into());

		stream.write("bakes", &"idle", &serde_json::json!({ "elapsed": 1 }));
		stream.write("bakes", &"idle", &serde_json::json!({ "elapsed": 2 }));

		assert_eq!(retained.latest("bakes").expect("a retained record").1["data"]["elapsed"], 1);
	}
}
