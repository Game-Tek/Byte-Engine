use resource_management::{
	asset::ResourceId,
	resource::{storage_backend::QueryCursor, ReDBStorageBackend, ReadStorageBackend},
	QueryableValue,
};
#[cfg(debug_assertions)]
use resource_management::{ResourceTraceItem, ResourceTraceLevel};
use serde_json::{json, Value};

/// Reads persisted development messages for one resource ID.
#[cfg(debug_assertions)]
pub(super) async fn read_resource_trace(storage_backend: &ReDBStorageBackend, id: &str) -> Result<Vec<ResourceTraceItem>, i32> {
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
	encode_hex(&bytes)
}

/// Decodes a query cursor produced by [`encode_query_cursor`].
pub(super) fn decode_query_cursor(cursor: &str) -> Result<QueryCursor, i32> {
	let bytes = decode_hex(cursor).ok_or_else(|| {
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

/// Encodes bytes as lowercase hexadecimal text for shell-safe cursor transport.
pub(super) fn encode_hex(bytes: &[u8]) -> String {
	const HEX: &[u8; 16] = b"0123456789abcdef";
	let mut output = String::with_capacity(bytes.len() * 2);

	for byte in bytes {
		output.push(HEX[(byte >> 4) as usize] as char);
		output.push(HEX[(byte & 0x0f) as usize] as char);
	}

	output
}

/// Decodes lowercase or uppercase hexadecimal text into the original bytes.
pub(super) fn decode_hex(value: &str) -> Option<Vec<u8>> {
	if !value.len().is_multiple_of(2) {
		return None;
	}

	let mut bytes = Vec::with_capacity(value.len() / 2);
	for chunk in value.as_bytes().chunks_exact(2) {
		let high = decode_hex_digit(chunk[0])?;
		let low = decode_hex_digit(chunk[1])?;
		bytes.push((high << 4) | low);
	}

	Some(bytes)
}

/// Decodes one hexadecimal ASCII digit.
fn decode_hex_digit(value: u8) -> Option<u8> {
	match value {
		b'0'..=b'9' => Some(value - b'0'),
		b'a'..=b'f' => Some(value - b'a' + 10),
		b'A'..=b'F' => Some(value - b'A' + 10),
		_ => None,
	}
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

/// Opens a BELD read command without allowing signature synchronization to replace persisted resources.
pub(super) fn open_read_only_storage(destination_path: String, operation: &str) -> Result<ReDBStorageBackend, i32> {
	ReDBStorageBackend::open_read_only(destination_path.into()).map_err(|error| {
		log::error!(
			"Failed to {} resources. The most likely cause is that they were baked by a different engine revision or the bake is incomplete. BELD did not modify the resources directory. Use a matching BELD build, or run `beld bake` when you are ready to replace the resources. Error: {}",
			operation,
			error
		);
		1
	})
}
