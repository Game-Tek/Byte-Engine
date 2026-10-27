use std::path::Path;

use resource_management::{
	asset::ResourceId,
	resource::{ReDBStorageBackend, ReadStorageBackend, WriteStorageBackend},
};

use crate::{
	BeldError,
	commands::shared::{offload_file_operation, open_read_only_storage},
};

/// Removes every baked resource through the configured resource storage backend.
///
/// Call [`crate::bake`] next to populate the empty store.
pub async fn wipe(destination_path: &Path) -> Result<(), BeldError> {
	let destination_path = destination_path.to_path_buf();
	let storage_backend = offload_file_operation(move || ReDBStorageBackend::new_writable(destination_path)).await;

	storage_backend.clear().await.map_err(|error| {
		BeldError::new(format!(
			"Failed to wipe resources. The most likely cause is that the resource store cannot be updated. Close other processes that use the resources directory, then retry. Error: {error}"
		))
	})
}

/// Removes every baked resource through the configured resource storage backend.
///
/// This function is the library equivalent of BELD's `clear` alias. Call
/// [`crate::bake`] next to populate the empty store.
pub async fn clear(destination_path: &Path) -> Result<(), BeldError> {
	wipe(destination_path).await
}

/// Lists every resource ID in a compatible store.
///
/// Pass an ID to [`crate::inspect`] to read its metadata.
pub async fn list(destination_path: &Path) -> Result<(), BeldError> {
	let storage_backend = open_read_only_storage(destination_path, "list").await?;

	match storage_backend.list().await {
		Ok(resources) => {
			if resources.is_empty() {
				log::info!("No resources found.");
			}

			for resource in resources {
				println!("{}", resource);
			}

			Ok(())
		}
		Err(error) => Err(BeldError::new(format!(
			"Failed to list resources. The most likely cause is an unreadable resources database. Run `beld bake` to rebuild it. Error: {error}"
		))),
	}
}

/// Deletes the selected resource IDs from a writable store.
///
/// Call [`crate::list`] next to verify the remaining resource IDs.
pub async fn delete(destination_path: &Path, ids: Vec<String>) -> Result<(), BeldError> {
	if ids.is_empty() {
		log::info!("No resources to delete.");
		return Ok(());
	}

	let destination_path = destination_path.to_path_buf();
	offload_file_operation(move || {
		let storage_backend = ReDBStorageBackend::new_writable(destination_path);
		let mut failures = 0usize;

		for id in ids {
			match storage_backend.delete(ResourceId::new(&id)) {
				Ok(()) => {
					log::info!("Deleted resource '{}'", id);
				}
				Err(error) => {
					log::error!(
						"Failed to delete '{}'. The most likely cause is that the resource store could not be updated. Error: {}",
						id,
						error
					);
					failures += 1;
				}
			}
		}

		if failures == 0 {
			return Ok(());
		}

		Err(BeldError::new(format!(
			"Failed to delete {failures} resources. The most likely cause is that the resource store could not be updated; each failure is logged above. Close other processes that use the resources directory, then retry."
		)))
	})
	.await
}
