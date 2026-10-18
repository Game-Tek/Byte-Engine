//! Per-entity debugging data streams, written as NDJSON.
//!
//! The application opens a [`DataStream`] for an entity with
//! [`GraphicsApplication::data_stream`](crate::application::graphics::GraphicsApplication::data_stream) and hands it
//! to the system that holds the entity's state. The `debug.streams` parameter selects where every stream writes; the
//! [data streams reference](/docs/reference/data-streams) specifies the destinations and the records.

use std::{borrow::Cow, io::Write, sync::Arc, time::Instant};

use serde::Serialize;
use utils::sync::Mutex;

use crate::core::factory::Handle;

/// The `StreamOutput` struct is the one destination every data stream of an application shares.
pub(crate) struct StreamOutput {
	/// `None` writes to the log; otherwise the writer is locked so lines from streams on different threads never
	/// interleave, which a pipe only guarantees for short writes.
	writer: Option<Mutex<Box<dyn Write + Send>>>,
	/// The application's start, which record times count from, like the inspector's metrics.
	start: Instant,
}

impl StreamOutput {
	/// Opens the destination a `debug.streams` value names: `log`, `stdout`, `stderr`, or a file path, which it creates
	/// or truncates.
	pub(crate) fn open(destination: &str, start: Instant) -> std::io::Result<Self> {
		let writer: Box<dyn Write + Send> = match destination {
			"log" => return Ok(Self { writer: None, start }),
			"stdout" => Box::new(std::io::stdout()),
			"stderr" => Box::new(std::io::stderr()),
			path => Box::new(std::fs::File::create(path)?),
		};
		Ok(Self {
			writer: Some(Mutex::new(writer)),
			start,
		})
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
}

/// The fields every record carries, around the system's `data`.
#[derive(Serialize)]
struct Record<'a, T> {
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
		}
	}

	/// Writes `data` as one record of `stream`, the name of the system writing it.
	///
	/// It blocks until the destination takes the whole line, so a slow reader slows the system instead of losing
	/// records.
	pub fn write(&mut self, stream: &str, data: &impl Serialize) {
		self.line.clear();
		let record = Record {
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
	use super::*;

	#[test]
	fn records_are_one_json_object_per_line_with_the_entity_envelope() {
		let entity = Handle::new();
		let path = std::env::temp_dir().join(format!("byte-engine-stream-{}-{}.ndjson", std::process::id(), entity.id()));
		let output = Arc::new(StreamOutput::open(path.to_str().unwrap(), Instant::now()).expect("file opens"));
		let mut stream = DataStream::new(output, entity, "Box".into());

		stream.write("physics", &serde_json::json!({ "speed": 0.5 }));
		stream.write("physics", &serde_json::json!({ "speed": f32::NAN }));

		let text = std::fs::read_to_string(&path).expect("file reads");
		std::fs::remove_file(&path).expect("file removes");
		let records: Vec<serde_json::Value> = text
			.lines()
			.map(|line| serde_json::from_str(line).expect("each line is one JSON object"))
			.collect();
		assert!(text.ends_with('\n'));
		assert_eq!(records.len(), 2);
		assert_eq!(records[0]["entity"], entity.id());
		assert_eq!(records[0]["name"], "Box");
		assert_eq!(records[0]["stream"], "physics");
		assert_eq!(records[0]["data"]["speed"], 0.5);
		assert!(records[0]["time"].as_f64().is_some_and(|time| time >= 0.0));
		assert!(records[1]["data"]["speed"].is_null());
	}
}
