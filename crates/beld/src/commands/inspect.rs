#[cfg(debug_assertions)]
use resource_management::ResourceTraceItem;
use resource_management::{
	asset::ResourceId,
	inspect::{ResourceDocument, inspect_payload},
	resource::{ReDBStorageBackend, ReadStorageBackend, ResourceId as ResourceUid, resource_handler::MultiResourceReader},
};
#[cfg(debug_assertions)]
use serde_json::{Value, json};

#[cfg(debug_assertions)]
use crate::commands::shared::{insert_trace_json, read_resource_trace};
use crate::{
	OutputFormat, Payload,
	commands::shared::{attach_trace, open_read_only_storage, payload_format, print_human_value, read_payload, write_json},
};

/// Inspects one resource by ID or UID and writes it in the selected format.
///
/// Pass `payload` to add the decoded stream bytes, which needs [`OutputFormat::JSON`]. Call [`crate::query`] when you
/// need to find resources by class or indexed properties first, or [`crate::bake_dry_run`] to see what a bake would
/// produce without a store.
pub async fn inspect(destination_path: String, id: String, format: OutputFormat, payload: Payload) -> Result<(), i32> {
	let payload = payload_format(payload, format)?;
	let storage_backend = open_read_only_storage(destination_path, "inspect").await?;
	let resource = read_resource(&storage_backend, &id).await;
	let Some((resource, reader)) = resource else {
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
	attach_trace(&mut output, &storage_backend, resource.id()).await?;

	let payload_bytes = match payload {
		Some(_) => Some(read_payload(&resource, reader).await?),
		None => None,
	};
	let payload = match (payload, &payload_bytes) {
		(Some(payload), Some(bytes)) => Some(inspect_payload(&resource, bytes, payload).map_err(|error| {
			log::error!("{}", error);
			1
		})?),
		_ => None,
	};

	match format {
		OutputFormat::Human => print_human_value(&output, 0),
		OutputFormat::JSON => write_json(
			&mut std::io::stdout().lock(),
			&ResourceDocument {
				json: &output,
				payload: payload.as_ref(),
			},
		)?,
	}

	Ok(())
}

/// Prints diagnostics for an ID whose resource bake failed completely.
#[cfg(debug_assertions)]
pub(super) fn print_trace_only_inspection(id: &str, trace: &[ResourceTraceItem], format: OutputFormat) -> Result<(), i32> {
	let mut output = json!({
		"id": id,
		"resource": Value::Null,
	});
	insert_trace_json(&mut output, trace);

	match format {
		OutputFormat::Human => print_human_value(&output, 0),
		OutputFormat::JSON => write_json(&mut std::io::stdout().lock(), &output)?,
	}

	Ok(())
}

async fn read_resource(
	storage_backend: &ReDBStorageBackend,
	id: &str,
) -> Option<(resource_management::SerializableResource, MultiResourceReader)> {
	if let Some(uid) = ResourceUid::from_uid_hex(id)
		&& let Some(resource) = storage_backend.read_uid(uid).await
	{
		return Some(resource);
	}

	storage_backend.read(ResourceId::new(id)).await
}
