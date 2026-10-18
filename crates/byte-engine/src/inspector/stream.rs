//! Per-entity debugging data streams, written as NDJSON.
//!
//! The application opens a [`DataStream`] for an entity with
//! [`GraphicsApplication::data_stream`](crate::application::graphics::GraphicsApplication::data_stream) and hands it
//! to the system that holds the entity's state. The `debug.streams` parameter selects where every stream writes; the
//! [data streams reference](/docs/reference/data-streams) specifies the destinations and the records.

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
use utils::sync::Mutex;

use crate::core::factory::Handle;

/// The `StreamOutput` struct is the one destination every data stream of an application shares.
pub(crate) struct StreamOutput {
	/// `None` writes to the log; otherwise the writer is locked so lines from streams on different threads never
	/// interleave, which a pipe only guarantees for short writes.
	writer: Option<Mutex<Box<dyn Write + Send>>>,
	/// Whether streams skip records whose key matches the last one they wrote, from `debug.streams.changes-only`.
	changes_only: bool,
	/// The application tick in progress, which every record carries; see [`Self::set_tick`].
	tick: AtomicU64,
	/// The application's start, which record times count from, like the inspector's metrics.
	start: Instant,
}

impl StreamOutput {
	/// Opens the destination a `debug.streams` value names: `log`, `stdout`, `stderr`, or a file path, which it creates
	/// or truncates.
	pub(crate) fn open(destination: &str, changes_only: bool, start: Instant) -> std::io::Result<Self> {
		let writer: Option<Box<dyn Write + Send>> = match destination {
			"log" => None,
			"stdout" => Some(Box::new(std::io::stdout())),
			"stderr" => Some(Box::new(std::io::stderr())),
			path => Some(Box::new(std::fs::File::create(path)?)),
		};
		Ok(Self {
			writer: writer.map(Mutex::new),
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

/// The `DataStream` struct lets the system that holds an entity's state report that state, one record per update,
/// for agents and tools that debug the entity.
///
/// Open one with [`GraphicsApplication::data_stream`](crate::application::graphics::GraphicsApplication::data_stream)
/// and pass it to the system, which calls [`Self::write`] after each update.
pub struct DataStream {
	output: Arc<StreamOutput>,
	entity: Handle,
	name: Cow<'static, str>,
	/// The line being written, kept so steady-state records allocate nothing.
	line: Vec<u8>,
	/// The key of the last record written, which changes-only streams compare the next key with.
	key: Vec<u8>,
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

impl DataStream {
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
		let Some(writer) = &self.output.writer else {
			let line = std::str::from_utf8(&self.line).expect("serde_json writes UTF-8");
			log::info!(target: "byte_engine::stream", "{line}");
			return;
		};
		self.line.push(b'\n');
		if let Err(error) = writer.lock().write_all(&self.line) {
			log::warn!(
				"Data stream record could not be written. The most likely cause is that the reader of `debug.streams` closed it: {error}"
			);
		}
	}
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::*;

	/// Opens a stream for a new entity named `Box` to a temporary file, returned with its output and the file's path.
	fn stream(changes_only: bool) -> (DataStream, Arc<StreamOutput>, PathBuf) {
		let entity = Handle::new();
		let path = std::env::temp_dir().join(format!("byte-engine-stream-{}-{}.ndjson", std::process::id(), entity.id()));
		let output = Arc::new(StreamOutput::open(path.to_str().unwrap(), changes_only, Instant::now()).expect("file opens"));
		(DataStream::new(Arc::clone(&output), entity, "Box".into()), output, path)
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
}
