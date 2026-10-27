use std::num::NonZeroUsize;

use byte_engine::rendering::pipelines::visibility::VisibilityFeatures;
use resource_management::{
	asset::{ResourceId, manager::AssetManager},
	inspect::{ResourceDocument, inspect_payload, inspect_resource},
	resource::{MemoryStorageBackend, ReadStorageBackend, ResourceGpuCompressionPolicy, ResourceStorageMode},
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
	OutputFormat, Payload,
	commands::shared::{attach_trace, payload_format, print_human_value, read_payload, write_json},
	pipeline::{bake_ids, default_asset_manager, open_asset_storage, open_resource_storage},
};

/// Bakes selected source assets, or every discoverable asset when `ids` is empty.
///
/// Assets whose stored resource is current with its source files are skipped unless `force` is set. Force a bake
/// after changing an asset processor, `texture_compression`, or `features`, because source versions don't record those.
/// `features` are the project's visibility features, usually read from its `config.json`; discovery skips the engine
/// assets only left-out features use.
///
/// Call [`crate::list`] next to inspect the resource IDs written to the destination.
#[allow(clippy::too_many_arguments)]
pub async fn bake(
	source_path: String,
	destination_path: String,
	ids: Vec<String>,
	storage_mode: Option<ResourceStorageMode>,
	texture_compression: Option<ResourceGpuCompressionPolicy>,
	memory_budget: NonZeroUsize,
	force: bool,
	features: VisibilityFeatures,
) -> Result<(), i32> {
	let asset_storage_backend = open_asset_storage(source_path.into()).await?;
	let resource_storage_backend = open_resource_storage(destination_path.into(), storage_mode, texture_compression).await?;

	let mut asset_manager = default_asset_manager(asset_storage_backend, resource_storage_backend, features);
	set_memory_budget(&mut asset_manager, memory_budget);

	// A forced run treats everything stored before it started as stale, so shared dependencies rebuild once.
	if force {
		asset_manager.rebuild_resources_baked_before(std::time::SystemTime::now());
	}

	let ids = if ids.is_empty() {
		let mut ids = asset_manager.discover().await.map_err(|error| {
			log::error!("Failed to discover assets. {error}");
			1
		})?;
		// Explicitly requested IDs still bake; only discovery skips what the project left out.
		ids.retain(|id| !features.excludes(id));
		ids
	} else {
		ids
	};

	if ids.is_empty() {
		log::info!("No supported assets found to bake.");

		return Ok(());
	}

	let failures = bake_ids(&asset_manager, ids).await;

	if failures.is_empty() { Ok(()) } else { Err(1) }
}

/// Bakes `ids` and every resource they depend on into memory, and writes everything produced as one document to
/// `output`, leaving the destination directory untouched.
///
/// The document lists each stored resource in [`crate::inspect`]'s shape, ordered by ID, then each requested ID that
/// failed. Pass `payload` to add the decoded stream bytes, which needs [`OutputFormat::JSON`]. Without `ids` nothing
/// bakes, because a dry run doesn't discover assets. Run [`bake`] next to write the same resources to a store.
pub async fn bake_dry_run(
	source_path: String,
	ids: Vec<String>,
	memory_budget: NonZeroUsize,
	features: VisibilityFeatures,
	format: OutputFormat,
	payload: Payload,
	output: &mut impl std::io::Write,
) -> Result<(), i32> {
	let payload = payload_format(payload, format)?;

	let asset_storage_backend = open_asset_storage(source_path.into()).await?;
	// The manager owns one clone; this one reads back what it baked once it is done.
	let resource_storage_backend = MemoryStorageBackend::new();
	let mut asset_manager = default_asset_manager(asset_storage_backend, resource_storage_backend.clone(), features);
	set_memory_budget(&mut asset_manager, memory_budget);
	let failures = bake_ids(&asset_manager, ids).await;
	drop(asset_manager);

	let mut resource_ids = resource_storage_backend.list().await.map_err(|error| {
		log::error!("Failed to list the baked resources. The most likely cause is an internal storage error. Error: {error}");
		1
	})?;
	resource_ids.sort_unstable();

	// Read every resource first: payload inspections borrow the resources and their bytes until the document is written.
	let mut resources = Vec::with_capacity(resource_ids.len());
	for id in &resource_ids {
		let Some((resource, reader)) = resource_storage_backend.read(ResourceId::new(id)).await else {
			continue;
		};
		let mut inspection = inspect_resource(&resource).map_err(|error| {
			log::error!("{}", error);
			1
		})?;
		attach_trace(&mut inspection.json, &resource_storage_backend, id).await?;
		let payload_bytes = match payload {
			Some(_) => Some(read_payload(&resource, reader).await?),
			None => None,
		};
		resources.push((resource, inspection.json, payload_bytes));
	}

	let mut failed = Vec::with_capacity(failures.len());
	for (id, error) in &failures {
		let mut entry = json!({ "id": id, "error": format!("{error:?}") });
		attach_trace(&mut entry, &resource_storage_backend, id).await?;
		failed.push(entry);
	}

	match format {
		OutputFormat::Human => {
			let resources = resources.into_iter().map(|(_, json, _)| json).collect();
			print_human_value(&json!({ "resources": Value::Array(resources), "failed": failed }), 0);
		}
		OutputFormat::JSON => {
			let payloads = resources
				.iter()
				.map(|(resource, _, bytes)| match (payload, bytes) {
					(Some(payload), Some(bytes)) => inspect_payload(resource, bytes, payload).map(Some),
					_ => Ok(None),
				})
				.collect::<Result<Vec<_>, _>>()
				.map_err(|error| {
					log::error!("{}", error);
					1
				})?;
			let resources = resources
				.iter()
				.zip(&payloads)
				.map(|((_, json, _), payload)| ResourceDocument {
					json,
					payload: payload.as_ref(),
				})
				.collect();
			write_json(output, &DryRunDocument { resources, failed })?;
		}
	}

	if failures.is_empty() { Ok(()) } else { Err(1) }
}

/// The JSON document of a dry run. It is written straight to the output, so payload streams never become a JSON tree.
#[derive(Serialize)]
struct DryRunDocument<'a> {
	resources: Vec<ResourceDocument<'a>>,
	failed: Vec<Value>,
}

/// Sets the soft memory budget for concurrent bakes and logs it.
fn set_memory_budget(asset_manager: &mut AssetManager, memory_budget: NonZeroUsize) {
	asset_manager.set_bake_memory_budget(memory_budget);
	log::info!(
		"Using a {} soft memory budget for concurrent asset bakes.",
		utils::ByteSize::from(memory_budget.get())
	);
}
