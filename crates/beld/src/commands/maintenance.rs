use resource_management::{
	asset::ResourceId,
	resource::{ReDBStorageBackend, ReadStorageBackend, WriteStorageBackend},
};

use crate::commands::shared::{offload_file_operation, open_read_only_storage};

/// Removes a resource store and recreates its destination directory.
///
/// Call [`crate::bake`] next to populate the empty store.
pub async fn wipe(destination_path: String) -> Result<(), i32> {
	let path = std::path::PathBuf::from(destination_path);
	let removal_path = path.clone();
	offload_file_operation(move || std::fs::remove_dir_all(removal_path))
		.await
		.map_err(|error| {
			log::error!(
				"Failed to wipe resources. The most likely cause is that the destination does not exist or cannot be removed. Error: {error}"
			);
			1
		})?;

	resource_management::r#async::create_dir(path).await.map_err(|error| {
		log::error!(
			"Failed to create resources directory. The most likely cause is that its parent directory is missing or not writable. Error: {error}"
		);
		1
	})?;

	Ok(())
}

/// Removes a resource store and recreates its destination directory.
///
/// This function is the library equivalent of BELD's `clear` alias. Call
/// [`crate::bake`] next to populate the empty store.
pub async fn clear(destination_path: String) -> Result<(), i32> {
	wipe(destination_path).await
}

/// Lists every resource ID in a compatible store.
///
/// Pass an ID to [`crate::inspect`] to read its metadata.
pub async fn list(destination_path: String) -> Result<(), i32> {
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
		Err(e) => {
			log::error!("Failed to list resources. Error: {}", e);
			Err(1)
		}
	}
}

/// Deletes the selected resource IDs from a writable store.
///
/// Call [`crate::list`] next to verify the remaining resource IDs.
pub async fn delete(destination_path: String, ids: Vec<String>) -> Result<(), i32> {
	if ids.is_empty() {
		log::info!("No resources to delete.");
		return Ok(());
	}

	offload_file_operation(move || {
		let storage_backend = ReDBStorageBackend::new_writable(destination_path.into());
		let mut ok = true;

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
					ok = false;
				}
			}
		}

		if ok { Ok(()) } else { Err(1) }
	})
	.await
}
