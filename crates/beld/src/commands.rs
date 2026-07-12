use resource_management::{
	asset::{FileStorageBackend, ResourceId},
	resource::{
		storage_backend::{Query, QueryCursor, QueryError},
		ReadStorageBackend, RedbStorageBackend, ResourceId as ResourceUid, WriteStorageBackend,
	},
	QueryableValue,
};
use serde_json::{json, Value};
use utils::{r#async::StreamExt, sync::Arc};

use crate::{utils::get_asset_manager, InspectFormat, QueryFormat};

const DEFAULT_BAKE_ARENA_CAPACITY: usize = 64 * 1024 * 1024;

pub fn wipe(destination_path: String) -> Result<(), i32> {
	std::fs::remove_dir_all(&destination_path).map_err(|e| {
		log::error!("Failed to wipe resources. Error: {}", e);
		1
	})?;

	std::fs::create_dir(&destination_path).map_err(|e| {
		log::error!("Failed to create resources directory. Error: {}", e);
		1
	})?;

	Ok(())
}

pub fn list(destination_path: String) -> Result<(), i32> {
	let storage_backend = RedbStorageBackend::new(destination_path.into());

	match storage_backend.list() {
		Ok(resources) => {
			if resources.is_empty() {
				log::info!("No resources found.");
			}

			for resource in resources {
				println!("{}", resource);
			}

			Ok(())
		}
		Err(e) => {
			log::error!("Failed to list resources. Error: {}", e);
			Err(1)
		}
	}
}

/// Queries the resources database by class and queryable property equality predicates.
pub fn query(
	destination_path: String,
	class: String,
	properties: Vec<String>,
	limit: Option<usize>,
	cursor: Option<String>,
	format: QueryFormat,
) -> Result<(), i32> {
	let storage_backend = RedbStorageBackend::new(destination_path.into());
	let mut query = Query::new(&class);

	if let Some(limit) = limit {
		query = query.limit(limit);
	}

	for property in properties {
		let (name, value) = parse_query_property(&property)?;
		query = query.eq(name, value);
	}

	if let Some(cursor) = cursor {
		query = query.cursor(decode_query_cursor(&cursor)?);
	}

	let page = storage_backend.query(query).map_err(|error| {
		log::error!("{}", query_error_message(error));
		1
	})?;

	match format {
		QueryFormat::Human => print_human_query_page(&page.items, page.cursor.as_ref()),
		QueryFormat::Json => print_json_query_page(&page.items, page.cursor.as_ref())?,
	}

	Ok(())
}

/// Parses a `name=value` query predicate from the CLI.
fn parse_query_property(property: &str) -> Result<(&str, &str), i32> {
	let Some((name, value)) = property.split_once('=') else {
		log::error!(
			"Invalid query property '{}'. The most likely cause is that the filter is not in `property=value` form.",
			property
		);
		return Err(1);
	};

	if name.is_empty() || value.is_empty() {
		log::error!(
			"Invalid query property '{}'. The most likely cause is an empty property name or value.",
			property
		);
		return Err(1);
	}

	Ok((name, value))
}

/// Converts storage query failures into user-facing CLI errors.
fn query_error_message(error: QueryError) -> &'static str {
	match error {
		QueryError::InvalidCursor => "Failed to query resources. The most likely cause is that the provided cursor is invalid.",
		QueryError::StorageFailure => {
			"Failed to query resources. The most likely cause is that the resources database could not be read."
		}
	}
}

/// Prints query results in a compact human-readable form.
fn print_human_query_page(
	items: &[(
		resource_management::SerializableResource,
		resource_management::resource::resource_handler::MultiResourceReader,
	)],
	cursor: Option<&QueryCursor>,
) {
	if items.is_empty() {
		log::info!("No resources found.");
	}

	for (resource, _) in items {
		println!("{}", resource.id());
		for property in resource.queryable_properties() {
			print!("  {}: ", property.name);
			print_queryable_value(&property.value);
			println!();
		}
	}

	if let Some(cursor) = cursor {
		println!("cursor: {}", encode_query_cursor(cursor));
	}
}

/// Prints query results as JSON for scripts and editor integrations.
fn print_json_query_page(
	items: &[(
		resource_management::SerializableResource,
		resource_management::resource::resource_handler::MultiResourceReader,
	)],
	cursor: Option<&QueryCursor>,
) -> Result<(), i32> {
	let resources = items
		.iter()
		.map(|(resource, _)| {
			json!({
				"id": resource.id(),
				"uid": resource.uid(),
				"class": resource.class(),
				"properties": queryable_properties_json(resource.queryable_properties()),
			})
		})
		.collect::<Vec<_>>();

	let output = json!({
		"resources": resources,
		"cursor": cursor.map(encode_query_cursor),
	});

	println!(
		"{}",
		serde_json::to_string_pretty(&output).map_err(|error| {
			log::error!(
				"Failed to print query JSON. The most likely cause is an invalid JSON value. Error: {}",
				error
			);
			1
		})?
	);

	Ok(())
}

/// Converts queryable properties to JSON without deserializing the resource body.
fn queryable_properties_json(properties: &[resource_management::QueryableProperty]) -> Value {
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
fn print_queryable_value(value: &QueryableValue) {
	match value {
		QueryableValue::String(value) => print!("{}", value),
	}
}

/// Encodes an opaque query cursor as JSON and hex so it is safe to pass through the shell.
fn encode_query_cursor(cursor: &QueryCursor) -> String {
	let bytes = serde_json::to_vec(cursor).expect("query cursors should serialize");
	encode_hex(&bytes)
}

/// Decodes a shell-safe query cursor produced by `encode_query_cursor`.
fn decode_query_cursor(cursor: &str) -> Result<QueryCursor, i32> {
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
fn encode_hex(bytes: &[u8]) -> String {
	const HEX: &[u8; 16] = b"0123456789abcdef";
	let mut output = String::with_capacity(bytes.len() * 2);

	for byte in bytes {
		output.push(HEX[(byte >> 4) as usize] as char);
		output.push(HEX[(byte & 0x0f) as usize] as char);
	}

	output
}

/// Decodes lowercase or uppercase hexadecimal text into the original bytes.
fn decode_hex(value: &str) -> Option<Vec<u8>> {
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

pub fn inspect(destination_path: String, id: String, format: InspectFormat) -> Result<(), i32> {
	let storage_backend = RedbStorageBackend::new(destination_path.into());
	let resource = read_resource(&storage_backend, &id).ok_or_else(|| {
		log::error!(
			"Failed to inspect resource '{}'. The most likely cause is that no baked resource exists for the given ID or UID.",
			id
		);
		1
	})?;

	let inspection = resource_management::inspect::inspect_resource(&resource).map_err(|error| {
		log::error!("{}", error);
		1
	})?;

	if inspection.unsupported_resource_section {
		log::warn!(
			"Unsupported resource class '{}'. Printing metadata without a deserialized resource section.",
			resource.class()
		);
	}

	match format {
		InspectFormat::Human => print_human_value(&inspection.json, 0),
		InspectFormat::Json => println!(
			"{}",
			serde_json::to_string_pretty(&inspection.json).map_err(|error| {
				log::error!(
					"Failed to print resource JSON. The most likely cause is an invalid JSON value. Error: {}",
					error
				);
				1
			})?
		),
	}

	Ok(())
}

fn read_resource(storage_backend: &RedbStorageBackend, id: &str) -> Option<resource_management::SerializableResource> {
	if let Some(uid) = ResourceUid::from_uid_hex(id) {
		if let Some((resource, _)) = storage_backend.read_uid(uid) {
			return Some(resource);
		}
	}

	storage_backend.read(ResourceId::new(id)).map(|(resource, _)| resource)
}

fn print_human_value(value: &Value, indent: usize) {
	match value {
		Value::Object(object) => {
			for (key, value) in object {
				print_human_field(key, value, indent);
			}
		}
		_ => print_human_scalar(value, indent),
	}
}

fn print_human_field(key: &str, value: &Value, indent: usize) {
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

fn print_human_scalar(value: &Value, indent: usize) {
	print_indent(indent);
	print_human_inline(value);
	println!();
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

pub fn bake(source_path: String, destination_path: String, ids: Vec<String>) -> Result<(), i32> {
	if ids.is_empty() {
		log::info!("No resources to bake.");
		return Ok(());
	}

	let storage_backend = FileStorageBackend::new(source_path.into());

	let asset_manager = get_asset_manager(storage_backend);

	let storage_backend = RedbStorageBackend::new(destination_path.into());

	let executor = resource_management::r#async::Executor::new().map_err(|_| 1)?;

	let asset_manager = Arc::new(asset_manager);

	let tasks = ids.into_iter().map(async |id| {
		let asset_manager = asset_manager.clone();
		// Keep decode-time allocations local to each asset so large PNG/WAV/OGG temporaries are released after baking.
		let arena = bumpalo::Bump::with_capacity(DEFAULT_BAKE_ARENA_CAPACITY);
		let allocator = &arena;
		log::info!("Baking resource '{}'", id);
		match asset_manager.bake_in(&id, &storage_backend, &allocator).await {
			Ok(_) => {
				log::info!("Baked resource '{}'", id);
			}
			Err(e) => {
				log::error!("Failed to bake '{}'. Error: {:#?}", id, e);
			}
		}
	});

	let tasks = utils::r#async::stream::iter(tasks);
	let tasks = tasks.buffer_unordered(16).collect::<Vec<_>>();

	executor.block_on(tasks);

	Ok(())
}

pub fn delete(destination_path: String, ids: Vec<String>) -> Result<(), i32> {
	let storage_backend = RedbStorageBackend::new(destination_path.into());

	let mut ok = true;

	if ids.is_empty() {
		log::info!("No resources to delete.");
		return Ok(());
	}

	for id in ids {
		match storage_backend.delete(ResourceId::new(&id)) {
			Ok(()) => {
				log::info!("Deleted resource '{}'", id);
			}
			Err(e) => {
				log::error!("Failed to delete '{}'. Error: {}", id, e);
				ok = false;
			}
		}
	}

	if ok {
		Ok(())
	} else {
		Err(1)
	}
}

#[cfg(test)]
mod tests {
	use std::time::{SystemTime, UNIX_EPOCH};

	use resource_management::{
		resource::storage_backend::{QueryCursor, QueryError},
		QueryableProperty, QueryableValue,
	};
	use serde_json::json;

	use super::{
		decode_hex, decode_query_cursor, encode_hex, encode_query_cursor, parse_query_property, query_error_message,
		queryable_properties_json, wipe,
	};

	#[test]
	fn query_property_parser_splits_once_and_rejects_missing_halves() {
		assert_eq!(parse_query_property("name=hero"), Ok(("name", "hero")));
		assert_eq!(parse_query_property("expression=a=b"), Ok(("expression", "a=b")));
		assert_eq!(parse_query_property("name"), Err(1));
		assert_eq!(parse_query_property("=value"), Err(1));
		assert_eq!(parse_query_property("name="), Err(1));
	}

	#[test]
	fn hex_codec_round_trips_all_byte_values_and_accepts_uppercase() {
		let bytes: Vec<u8> = (u8::MIN..=u8::MAX).collect();
		let encoded = encode_hex(&bytes);
		assert_eq!(encoded.len(), bytes.len() * 2);
		assert_eq!(decode_hex(&encoded), Some(bytes.clone()));
		assert_eq!(decode_hex(&encoded.to_uppercase()), Some(bytes));
		assert_eq!(decode_hex("0"), None);
		assert_eq!(decode_hex("gg"), None);
	}

	#[test]
	fn query_cursor_codec_is_lossless_and_rejects_non_cursor_json() {
		let cursor = QueryCursor::new(vec![0, 1, 2, 0xfe, 0xff]);
		let encoded = encode_query_cursor(&cursor);
		assert_eq!(decode_query_cursor(&encoded), Ok(cursor));
		assert_eq!(decode_query_cursor("not-hex"), Err(1));
		assert_eq!(decode_query_cursor(&encode_hex(br#"{"wrong":true}"#)), Err(1));
	}

	#[test]
	fn query_properties_convert_to_json_without_losing_names_or_values() {
		let properties = [
			QueryableProperty {
				name: "name".into(),
				value: QueryableValue::String("hero".into()),
			},
			QueryableProperty {
				name: "group".into(),
				value: QueryableValue::String("opaque".into()),
			},
		];
		assert_eq!(
			queryable_properties_json(&properties),
			json!({"name": "hero", "group": "opaque"})
		);
	}

	#[test]
	fn query_errors_keep_distinct_actionable_causes() {
		assert!(query_error_message(QueryError::InvalidCursor).contains("cursor is invalid"));
		assert!(query_error_message(QueryError::StorageFailure).contains("database could not be read"));
	}

	#[test]
	fn wipe_removes_old_contents_and_recreates_empty_destination() {
		let path = std::env::temp_dir().join(format!(
			"beld-wipe-test-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		));
		std::fs::create_dir_all(path.join("nested")).unwrap();
		std::fs::write(path.join("nested/old.resource"), b"old").unwrap();

		wipe(path.to_string_lossy().into_owned()).unwrap();
		assert!(path.is_dir());
		assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
		std::fs::remove_dir(path).unwrap();
	}
}
