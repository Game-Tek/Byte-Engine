use resource_management::{
	asset::{FileStorageBackend, ResourceId},
	resource::{ReadStorageBackend, RedbStorageBackend, ResourceId as ResourceUid, WriteStorageBackend},
};
use serde_json::Value;
use utils::{r#async::StreamExt, sync::Arc};

use crate::{utils::get_asset_manager, InspectFormat};

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
