use resource_management::inspect::PayloadFormat;
use resource_management::{
	QueryableValue, SerializableResource,
	asset::ResourceId,
	resource::{
		ReDBStorageBackend, ReadStorageBackend, ReadTargets, ReadTargetsMut, ResourcePayloadEncoding,
		resource_handler::MultiResourceReader, storage_backend::QueryCursor,
	},
};
#[cfg(debug_assertions)]
use resource_management::{ResourceTraceItem, ResourceTraceLevel};
use serde_json::{Value, json};

use crate::{OutputFormat, Payload};

/// Reads persisted development messages for one resource ID.
#[cfg(debug_assertions)]
pub(super) async fn read_resource_trace(
	storage_backend: &impl ReadStorageBackend,
	id: &str,
) -> Result<Vec<ResourceTraceItem>, i32> {
	storage_backend.read_trace(ResourceId::new(id)).await.map_err(|error| {
		log::error!(
			"Failed to read the resource trace for '{}'. The most likely cause is an unreadable resources database. Error: {}",
			id,
			error
		);
		1
	})
}

/// Converts trace items to the stable JSON shape used by query and inspect.
#[cfg(debug_assertions)]
pub(super) fn resource_trace_json(items: &[ResourceTraceItem]) -> Value {
	Value::Array(
		items
			.iter()
			.map(|item| {
				json!({
					"level": match item.level() {
						ResourceTraceLevel::Info => "info",
						ResourceTraceLevel::Warn => "warn",
						ResourceTraceLevel::Error => "error",
					},
					"message": item.message(),
				})
			})
			.collect(),
	)
}

/// Adds trace JSON to a resource inspection or query result object.
#[cfg(debug_assertions)]
pub(super) fn insert_trace_json(value: &mut Value, items: &[ResourceTraceItem]) {
	let Value::Object(object) = value else {
		return;
	};
	object.insert("trace".to_string(), resource_trace_json(items));
}

/// Prints one trace using the same nested layout as resource inspection.
#[cfg(debug_assertions)]
pub(super) fn print_human_trace(items: &[ResourceTraceItem], indent: usize) {
	print_human_field("trace", &resource_trace_json(items), indent);
}

/// Converts indexed properties to JSON without reading the resource body.
pub(super) fn queryable_properties_json(properties: &[resource_management::QueryableProperty]) -> Value {
	let properties = properties
		.iter()
		.map(|property| {
			let value = match &property.value {
				QueryableValue::String(value) => Value::String(value.clone()),
			};

			(property.name.clone(), value)
		})
		.collect();

	Value::Object(properties)
}

/// Prints one queryable value without allocating an intermediate string.
pub(super) fn print_queryable_value(value: &QueryableValue) {
	match value {
		QueryableValue::String(value) => print!("{}", value),
	}
}

/// Encodes an opaque query cursor as shell-safe hexadecimal JSON.
pub(super) fn encode_query_cursor(cursor: &QueryCursor) -> String {
	let bytes = serde_json::to_vec(cursor).expect("query cursors should serialize");
	utils::hex::encode(&bytes)
}

/// Decodes a query cursor produced by [`encode_query_cursor`].
pub(super) fn decode_query_cursor(cursor: &str) -> Result<QueryCursor, i32> {
	let bytes = utils::hex::decode(cursor).ok_or_else(|| {
		log::error!(
			"Invalid query cursor '{}'. The most likely cause is that the cursor was not copied from a previous query result.",
			cursor
		);
		1
	})?;

	serde_json::from_slice(&bytes).map_err(|error| {
		log::error!(
			"Invalid query cursor '{}'. The most likely cause is that the cursor was not copied from a previous query result. Error: {}",
			cursor,
			error
		);
		1
	})
}

pub(super) fn print_human_value(value: &Value, indent: usize) {
	match value {
		Value::Object(object) => {
			for (key, value) in object {
				print_human_field(key, value, indent);
			}
		}
		_ => {
			print_indent(indent);
			print_human_inline(value);
			println!();
		}
	}
}

pub(super) fn print_human_field(key: &str, value: &Value, indent: usize) {
	print_indent(indent);
	match value {
		Value::Object(object) if object.is_empty() => println!("{}: {{}}", key),
		Value::Object(_) => {
			println!("{}:", key);
			print_human_value(value, indent + 2);
		}
		Value::Array(values) if values.is_empty() => println!("{}: []", key),
		Value::Array(values) => {
			println!("{}:", key);
			for value in values {
				print_human_array_value(value, indent + 2);
			}
		}
		_ => {
			print!("{}: ", key);
			print_human_inline(value);
			println!();
		}
	}
}

fn print_human_array_value(value: &Value, indent: usize) {
	print_indent(indent);
	match value {
		Value::Object(object) if object.is_empty() => println!("- {{}}"),
		Value::Object(object) => {
			println!("-");
			for (key, value) in object {
				print_human_field(key, value, indent + 2);
			}
		}
		Value::Array(values) => {
			println!("-");
			for value in values {
				print_human_array_value(value, indent + 2);
			}
		}
		_ => {
			print!("- ");
			print_human_inline(value);
			println!();
		}
	}
}

fn print_human_inline(value: &Value) {
	match value {
		Value::Null => print!("null"),
		Value::Bool(value) => print!("{}", value),
		Value::Number(value) => print!("{}", value),
		Value::String(value) => print!("{}", value),
		Value::Array(_) | Value::Object(_) => print!("{}", value),
	}
}

fn print_indent(indent: usize) {
	for _ in 0..indent {
		print!(" ");
	}
}

/// Runs filesystem work that Compio does not support without blocking its runtime thread.
pub(super) async fn offload_file_operation<T, F>(operation: F) -> T
where
	T: Send + 'static,
	F: FnOnce() -> T + Send + 'static,
{
	match resource_management::r#async::offload(operation).await {
		Ok(output) => output,
		Err(error) => {
			error.resume_unwind();
			panic!("Asynchronous file operation was cancelled before it completed.");
		}
	}
}

/// Opens a BELD read command without allowing signature synchronization to replace persisted resources.
pub(super) async fn open_read_only_storage(destination_path: String, operation: &str) -> Result<ReDBStorageBackend, i32> {
	let storage = offload_file_operation(move || ReDBStorageBackend::open_read_only(destination_path.into())).await;

	storage.map_err(|error| {
		log::error!(
			"Failed to {} resources. The most likely cause is that they were baked by a different engine revision or the bake is incomplete. BELD did not modify the resources directory. Use a matching BELD build, or run `beld bake` when you are ready to replace the resources. Error: {}",
			operation,
			error
		);
		1
	})
}

/// Reads a resource's whole decoded payload so its streams can be printed.
pub(super) async fn read_payload(resource: &SerializableResource, mut reader: MultiResourceReader) -> Result<Box<[u8]>, i32> {
	if reader.encoding() == ResourcePayloadEncoding::MetalIoLz4 {
		log::error!(
			"Failed to read the payload of resource '{}'. The most likely cause is that it was baked with `--texture-compression metal-io-lz4`, which only the GPU can decode. Bake it again without that option, or print it with `beld bake --dry-run`. See {}.",
			resource.id(),
			utils::online_docs_url("api/beld#payloads")
		);
		return Err(1);
	}
	if resource.size() == 0 {
		return Ok(Box::new([]));
	}

	let buffer = vec![0u8; resource.size()].into_boxed_slice();
	match reader
		.read_into(
			None,
			ReadTargetsMut::Box {
				buffer,
				offset: 0,
				size: None,
			},
		)
		.await
	{
		Ok(ReadTargets::Box(bytes)) if bytes.len() == resource.size() => Ok(bytes),
		_ => {
			log::error!(
				"Failed to read the payload of resource '{}'. The most likely cause is a missing or truncated payload file, so the resource should be baked again.",
				resource.id()
			);
			Err(1)
		}
	}
}

/// Returns how to print payload streams, or exit code 2 for a payload in the human format, which has no form for
/// stream data.
pub(super) fn payload_format(payload: Payload, format: OutputFormat) -> Result<Option<PayloadFormat>, i32> {
	let payload = match payload {
		Payload::None => return Ok(None),
		Payload::Typed => PayloadFormat::Typed,
		Payload::Base64 => PayloadFormat::Base64,
	};
	if format == OutputFormat::Human {
		log::error!(
			"Invalid payload output. `--payload` needs `--format json`, because the human format has no form for stream data."
		);
		return Err(2);
	}
	Ok(Some(payload))
}

/// Adds a resource's development trace to its JSON in debug builds; release builds store no traces.
pub(super) async fn attach_trace(value: &mut Value, storage_backend: &impl ReadStorageBackend, id: &str) -> Result<(), i32> {
	#[cfg(debug_assertions)]
	insert_trace_json(value, &read_resource_trace(storage_backend, id).await?);
	#[cfg(not(debug_assertions))]
	let _ = (value, storage_backend, id);
	Ok(())
}

/// Writes one JSON document, pretty-printed and followed by a newline, to `output`.
pub(super) fn write_json(output: &mut impl std::io::Write, value: &impl serde::Serialize) -> Result<(), i32> {
	let result = serde_json::to_writer_pretty(&mut *output, value)
		.map_err(std::io::Error::from)
		.and_then(|()| writeln!(output));
	result.map_err(|error| {
		log::error!(
			"Failed to write JSON output. The most likely cause is that standard output was closed, for example by a pipe that exited early. Error: {}",
			error
		);
		1
	})
}
