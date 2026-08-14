use resource_management::{
	asset::ResourceId,
	resource::{ReadStorageBackend, RedbStorageBackend, WriteStorageBackend},
};

use crate::commands::shared::open_read_only_storage;

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

pub async fn list(destination_path: String) -> Result<(), i32> {
	let storage_backend = open_read_only_storage(destination_path, "list")?;

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
