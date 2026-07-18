use std::{path::Path, time::Instant};

use resource_management::{
	asset::{asset_manager::AssetManager, FileStorageBackend, ResourceId},
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
	let source_path = std::path::PathBuf::from(source_path);
	let storage_backend = FileStorageBackend::new(source_path.clone());

	let asset_manager = get_asset_manager(storage_backend);
	let ids = if ids.is_empty() {
		discover_asset_ids(&source_path, &asset_manager)?
	} else {
		ids
	};

	if ids.is_empty() {
		log::info!("No supported assets found to bake.");
		return Ok(());
	}

	let storage_backend = RedbStorageBackend::new_writable(destination_path.into());

	let executor = resource_management::r#async::Executor::new().map_err(|_| 1)?;

	let asset_manager = Arc::new(asset_manager);
	let resource_count = ids.len();

	let tasks = ids.into_iter().map(async |id| {
		let asset_manager = asset_manager.clone();
		// Keep decode-time allocations local to each asset so large PNG/WAV/OGG temporaries are released after baking.
		let arena = bumpalo::Bump::with_capacity(DEFAULT_BAKE_ARENA_CAPACITY);
		let allocator = &arena;
		log::info!("Baking resource '{}'", id);
		match asset_manager.bake_in(&id, &storage_backend, &allocator).await {
			Ok(_) => {
				log::info!("Baked resource '{}'", id);
				true
			}
			Err(e) => {
				log::error!("Failed to bake '{}'. Error: {:#?}", id, e);
				false
			}
		}
	});

	let tasks = utils::r#async::stream::iter(tasks);
	let tasks = tasks.buffer_unordered(16).collect::<Vec<_>>();

	let bake_start = Instant::now();
	let results = executor.block_on(tasks);
	let failed_count = results.iter().filter(|result| !**result).count();
	let successful_count = results.len() - failed_count;
	log::info!(
		"Processed {} assets in {:?}: {} succeeded, {} failed",
		resource_count,
		bake_start.elapsed(),
		successful_count,
		failed_count
	);

	if failed_count == 0 {
		Ok(())
	} else {
		Err(1)
	}
}

/// Recursively finds supported source assets beneath the configured assets directory.
fn discover_asset_ids(source_path: &Path, asset_manager: &AssetManager) -> Result<Vec<String>, i32> {
	let mut ids = Vec::new();
	let canonical_root = std::fs::canonicalize(source_path).map_err(|error| {
		log::error!(
			"Failed to resolve assets directory '{}'. The most likely cause is that the directory does not exist or cannot be accessed. Error: {}",
			source_path.display(),
			error
		);
		1
	})?;
	let mut active_directories = std::collections::HashSet::from([canonical_root]);
	discover_asset_ids_in(source_path, source_path, asset_manager, &mut active_directories, &mut ids)?;
	ids.sort();
	Ok(ids)
}

/// Adds supported files from one directory and its children, following symlinks without revisiting an active ancestor.
fn discover_asset_ids_in(
	root_path: &Path,
	directory: &Path,
	asset_manager: &AssetManager,
	active_directories: &mut std::collections::HashSet<std::path::PathBuf>,
	ids: &mut Vec<String>,
) -> Result<(), i32> {
	let entries = std::fs::read_dir(directory).map_err(|error| {
		log::error!(
			"Failed to scan assets directory '{}'. The most likely cause is that the directory cannot be read. Error: {}",
			directory.display(),
			error
		);
		1
	})?;

	for entry in entries {
		let entry = entry.map_err(|error| {
			log::error!(
				"Failed to inspect an asset directory entry in '{}'. The most likely cause is a filesystem read failure. Error: {}",
				directory.display(),
				error
			);
			1
		})?;
		// Preserve the logical path used to enter a symlinked directory. Some platforms expose the resolved target path
		// through `DirEntry::path`, which would otherwise lose the mounted namespace when deriving the asset ID.
		let path = directory.join(entry.file_name());
		let file_type = entry.file_type().map_err(|error| {
			log::error!(
				"Failed to inspect asset path '{}'. The most likely cause is a filesystem metadata failure. Error: {}",
				path.display(),
				error
			);
			1
		})?;

		let followed_type = if file_type.is_symlink() {
			std::fs::metadata(&path)
				.map_err(|error| {
					log::error!(
					"Failed to follow asset symlink '{}'. The most likely cause is a broken link or inaccessible target. Error: {}",
					path.display(),
					error
				);
					1
				})?
				.file_type()
		} else {
			file_type
		};

		if followed_type.is_dir() {
			let canonical_directory = std::fs::canonicalize(&path).map_err(|error| {
				log::error!(
					"Failed to resolve asset directory '{}'. The most likely cause is a broken symlink or inaccessible directory. Error: {}",
					path.display(),
					error
				);
				1
			})?;
			if !active_directories.insert(canonical_directory.clone()) {
				log::warn!("Skipping cyclic asset directory link '{}'.", path.display());
				continue;
			}
			let result = discover_asset_ids_in(root_path, &path, asset_manager, active_directories, ids);
			active_directories.remove(&canonical_directory);
			result?;
			continue;
		}

		if !followed_type.is_file() {
			continue;
		}

		let Some(relative_path) = path.strip_prefix(root_path).ok() else {
			continue;
		};
		let Some(id) = resource_id_path(relative_path) else {
			log::warn!(
				"Skipping asset path '{}'. The most likely cause is a non-UTF-8 path that cannot be represented as a resource ID.",
				path.display()
			);
			continue;
		};

		if path
			.extension()
			.and_then(|extension| extension.to_str())
			.is_some_and(|extension| extension.eq_ignore_ascii_case("bead"))
		{
			continue;
		}

		if asset_manager.supports(&id) {
			ids.push(id);
		}
	}

	Ok(())
}

/// Converts a filesystem-relative path into a resource-style ID with `/` separators.
fn resource_id_path(path: &Path) -> Option<String> {
	let mut id = String::new();
	for component in path.components() {
		let component = component.as_os_str().to_str()?;
		if !id.is_empty() {
			id.push('/');
		}
		id.push_str(component);
	}
	Some(id)
}

pub fn delete(destination_path: String, ids: Vec<String>) -> Result<(), i32> {
	let storage_backend = RedbStorageBackend::new_writable(destination_path.into());

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
		asset::{FileStorageBackend, ResourceId},
		resource::{
			storage_backend::{QueryCursor, QueryError},
			ReadStorageBackend, RedbStorageBackend,
		},
		QueryableProperty, QueryableValue,
	};
	use serde_json::json;

	use super::{
		bake, decode_hex, decode_query_cursor, discover_asset_ids, encode_hex, encode_query_cursor, parse_query_property,
		query_error_message, queryable_properties_json, wipe,
	};
	use crate::utils::get_asset_manager;

	const TRIANGLE_MOVE_FBX: &[u8] = include_bytes!("../../resource-management/src/asset/test_data/triangle_move_ascii.fbx");

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
	fn discovers_supported_assets_recursively_and_ignores_sidecars_and_unknown_files() {
		let root = std::env::temp_dir().join(format!(
			"beld-discovery-test-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		));
		std::fs::create_dir_all(root.join("nested/deeper")).unwrap();
		std::fs::write(root.join("z-last.png"), []).unwrap();
		std::fs::write(root.join("nested/deeper/a-first.fbx"), []).unwrap();
		std::fs::write(root.join("nested/material.bema"), []).unwrap();
		std::fs::write(root.join("nested/material.bema.bead"), []).unwrap();
		std::fs::write(root.join("ignored.txt"), []).unwrap();

		let asset_manager = get_asset_manager(FileStorageBackend::new(root.clone()));
		let ids = discover_asset_ids(&root, &asset_manager).unwrap();

		assert_eq!(ids, ["nested/deeper/a-first.fbx", "nested/material.bema", "z-last.png"]);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[cfg(unix)]
	#[test]
	fn discovers_assets_through_symlinks_without_following_directory_cycles() {
		use std::os::unix::fs::symlink;

		let nonce = format!(
			"{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		);
		let root = std::env::temp_dir().join(format!("beld-symlink-discovery-test-{nonce}"));
		let engine_assets = std::env::temp_dir().join(format!("beld-engine-assets-test-{nonce}"));
		std::fs::create_dir_all(engine_assets.join("shaders")).unwrap();
		std::fs::create_dir_all(&root).unwrap();
		std::fs::write(engine_assets.join("shaders/render-pass.bema"), []).unwrap();
		std::fs::write(engine_assets.join("engine-icon.png"), []).unwrap();
		symlink(&engine_assets, root.join("byte-engine")).unwrap();
		symlink(engine_assets.join("engine-icon.png"), root.join("linked-engine-icon.png")).unwrap();
		symlink(&root, engine_assets.join("cycle-to-application-assets")).unwrap();

		let asset_manager = get_asset_manager(FileStorageBackend::new(root.clone()));
		let ids = discover_asset_ids(&root, &asset_manager).unwrap();

		assert_eq!(
			ids,
			[
				"byte-engine/engine-icon.png",
				"byte-engine/shaders/render-pass.bema",
				"linked-engine-icon.png",
			]
		);
		std::fs::remove_dir_all(root).unwrap();
		std::fs::remove_dir_all(engine_assets).unwrap();
	}

	#[test]
	fn default_bake_continues_after_a_failed_asset_and_returns_failure() {
		let root = std::env::temp_dir().join(format!(
			"beld-bake-all-test-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		));
		let assets_path = root.join("assets");
		let resources_path = root.join("resources");
		std::fs::create_dir_all(&assets_path).unwrap();
		std::fs::write(assets_path.join("triangle_move.fbx"), TRIANGLE_MOVE_FBX).unwrap();
		std::fs::write(assets_path.join("broken.png"), b"not a PNG").unwrap();

		assert_eq!(
			bake(
				assets_path.to_string_lossy().into_owned(),
				resources_path.to_string_lossy().into_owned(),
				Vec::new(),
			),
			Err(1)
		);

		let resource_storage = RedbStorageBackend::new(resources_path.clone());
		assert!(resource_storage.read(ResourceId::new("triangle_move.fbx")).is_some());
		std::fs::remove_dir_all(root).unwrap();
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
