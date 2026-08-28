use std::{num::NonZeroUsize, time::Instant};

use resource_management::{
	asset::FileStorageBackend,
	resource::{ReDBStorageBackend, ResourceGpuCompressionPolicy, ResourceStorageMode, ResourceStorageSettings},
};
use utils::{r#async::StreamExt, sync::Arc};

use crate::{commands::shared::offload_file_operation, utils::get_asset_manager};

/// Bakes selected source assets, or every discoverable asset when `ids` is empty.
///
/// Call [`crate::list`] next to inspect the resource IDs written to the destination.
pub async fn bake(
	source_path: String,
	destination_path: String,
	ids: Vec<String>,
	storage_mode: Option<ResourceStorageMode>,
	texture_compression: Option<ResourceGpuCompressionPolicy>,
	memory_budget: NonZeroUsize,
) -> Result<(), i32> {
	let source_path = std::path::PathBuf::from(source_path);
	let asset_storage_backend = FileStorageBackend::open(source_path.clone()).await.map_err(|error| {
		log::error!(
			"Failed to open assets directory '{}'. The most likely cause is that the directory cannot be created or accessed. Error: {error}",
			source_path.display()
		);
		1
	})?;
	let destination_path = std::path::PathBuf::from(destination_path);
	let resource_storage_backend = offload_file_operation(move || match texture_compression {
		Some(compression) => ReDBStorageBackend::new_writable_with_settings(
			destination_path,
			ResourceStorageSettings::new(storage_mode.unwrap_or_default()).image_compression(compression),
		),
		None => match storage_mode {
			Some(mode) => ReDBStorageBackend::new_writable_with_mode(destination_path, mode),
			None => Ok(ReDBStorageBackend::new_writable(destination_path)),
		},
	})
	.await
	.map_err(|error| {
		log::error!("Failed to bake resources. {error}");
		1
	})?;

	let mut asset_manager = get_asset_manager(asset_storage_backend, resource_storage_backend);

	asset_manager.set_bake_memory_budget(memory_budget);

	log::info!(
		"Using a {} MiB soft memory budget for concurrent asset bakes.",
		memory_budget.get() / (1024 * 1024)
	);

	let ids = if ids.is_empty() {
		asset_manager.discover().await.map_err(|error| {
			log::error!("Failed to discover assets. {error}");
			1
		})?
	} else {
		ids
	};

	if ids.is_empty() {
		log::info!("No supported assets found to bake.");

		return Ok(());
	}

	let asset_manager = Arc::new(asset_manager);

	let resource_count = ids.len();

	let tasks = ids.into_iter().map(async |id| {
		let asset_manager = asset_manager.clone();

		log::info!("Baking resource '{}'", id);

		match asset_manager.bake(&id).await {
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

	let tasks = utils::r#async::stream::iter(tasks).buffer_unordered(16).fold(
		(0usize, 0usize),
		|(successful, failed), result| async move {
			if result {
				(successful + 1, failed)
			} else {
				(successful, failed + 1)
			}
		},
	);

	let bake_start = Instant::now();

	let (successful_count, failed_count) = tasks.await;

	log::info!(
		"Processed {} assets in {:?}: {} succeeded, {} failed",
		resource_count,
		bake_start.elapsed(),
		successful_count,
		failed_count
	);

	if failed_count == 0 { Ok(()) } else { Err(1) }
}
