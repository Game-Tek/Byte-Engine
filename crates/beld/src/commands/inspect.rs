#[cfg(debug_assertions)]
use resource_management::ResourceTraceItem;
use resource_management::{
	asset::ResourceId,
	resource::{ReDBStorageBackend, ReadStorageBackend, ResourceId as ResourceUid},
};
use serde_json::{json, Value};

use crate::{
	commands::shared::{insert_trace_json, open_read_only_storage, print_human_value, read_resource_trace},
	InspectFormat,
};

pub async fn inspect(destination_path: String, id: String, format: InspectFormat) -> Result<(), i32> {
	let storage_backend = open_read_only_storage(destination_path, "inspect")?;
	let resource = read_resource(&storage_backend, &id).await;
	let Some(resource) = resource else {
		#[cfg(debug_assertions)]
		{
			let trace = read_resource_trace(&storage_backend, &id).await?;
			if !trace.is_empty() {
				return print_trace_only_inspection(&id, &trace, format);
			}
		}
		log::error!(
			"Failed to inspect resource '{}'. The most likely cause is that no baked resource exists for the given ID or UID.",
			id
		);
		return Err(1);
	};

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
	let mut output = inspection.json;
	#[cfg(debug_assertions)]
	let trace = read_resource_trace(&storage_backend, resource.id()).await?;
	#[cfg(debug_assertions)]
	insert_trace_json(&mut output, &trace);

	match format {
		InspectFormat::Human => print_human_value(&output, 0),
		InspectFormat::Json => println!(
			"{}",
			serde_json::to_string_pretty(&output).map_err(|error| {
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

/// Prints diagnostics for an ID whose resource bake failed completely.
#[cfg(debug_assertions)]
pub(super) fn print_trace_only_inspection(id: &str, trace: &[ResourceTraceItem], format: InspectFormat) -> Result<(), i32> {
	let mut output = json!({
		"id": id,
		"resource": Value::Null,
	});
	insert_trace_json(&mut output, trace);

	match format {
		InspectFormat::Human => print_human_value(&output, 0),
		InspectFormat::Json => println!(
			"{}",
			serde_json::to_string_pretty(&output).map_err(|error| {
				log::error!(
					"Failed to print resource trace JSON. The most likely cause is an invalid JSON value. Error: {}",
					error
				);
				1
			})?
		),
	}

	Ok(())
}

async fn read_resource(storage_backend: &ReDBStorageBackend, id: &str) -> Option<resource_management::SerializableResource> {
	if let Some(uid) = ResourceUid::from_uid_hex(id) {
		if let Some((resource, _)) = storage_backend.read_uid(uid).await {
			return Some(resource);
		}
	}

	storage_backend.read(ResourceId::new(id)).await.map(|(resource, _)| resource)
}
