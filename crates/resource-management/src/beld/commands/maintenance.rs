use crate::beld::commands::shared::{log_failure, offload_file_operation, open_read_only_storage};
use crate::{
	asset::ResourceId,
	resource::{ReDBStorageBackend, ReadStorageBackend, WriteStorageBackend},
};

/// Removes every baked resource through the configured resource storage backend.
///
/// Call [`crate::beld::bake`] next to populate the empty store.
pub async fn wipe(destination_path: String) -> Result<(), i32> {
	let storage_backend = open_writable_storage(destination_path, "wipe").await?;

	storage_backend.clear().await.map_err(|error| {
		log::error!(
			"Failed to wipe resources. The most likely cause is that the resource store cannot be updated. Error: {error}"
		);
		1
	})
}

/// Removes every baked resource through the configured resource storage backend.
///
/// This function is the library equivalent of BELD's `clear` alias. Call
/// [`crate::beld::bake`] next to populate the empty store.
pub async fn clear(destination_path: String) -> Result<(), i32> {
	wipe(destination_path).await
}

/// Lists every resource ID in a compatible store.
///
/// Pass an ID to [`crate::beld::inspect`] to read its metadata.
pub async fn list(destination_path: String) -> Result<(), i32> {
	let storage_backend = open_read_only_storage(destination_path, "list").await.map_err(log_failure)?;

	match storage_backend.list().await {
		Ok(resources) => {
			if resources.is_empty() {
				log::info!("No resources found.");
			}

			for resource in resources {
				println!("{}", resource.id());
			}

			Ok(())
		}
		Err(e) => {
			log::error!("Failed to list resources. Error: {}", e);
			Err(1)
		}
	}
}

/// Deletes the selected resource IDs from a writable store.
///
/// Call [`crate::beld::list`] next to verify the remaining resource IDs.
pub async fn delete(destination_path: String, ids: Vec<String>) -> Result<(), i32> {
	if ids.is_empty() {
		log::info!("No resources to delete.");
		return Ok(());
	}

	let storage_backend = open_writable_storage(destination_path, "delete").await?;
	let mut ok = true;

	for id in ids {
		match storage_backend.delete(ResourceId::new(&id)).await {
			Ok(()) => {
				log::info!("Deleted resource '{}'", id);
			}
			Err(error) => {
				log::error!(
					"Failed to delete '{}'. The most likely cause is that the resource store could not be updated. Error: {}",
					id,
					error
				);
				ok = false;
			}
		}
	}

	if ok { Ok(()) } else { Err(1) }
}

/// Opens the store a maintenance command updates, logging the open error and returning exit code 1 on failure.
async fn open_writable_storage(destination_path: String, operation: &str) -> Result<ReDBStorageBackend, i32> {
	offload_file_operation(move || ReDBStorageBackend::new_writable(destination_path.into()))
		.await
		.map_err(|error| {
			log::error!("Failed to {operation} resources. {error}");
			1
		})
}
