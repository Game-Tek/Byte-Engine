use resource_management::{
	asset::{
		FileStorageBackend, ResourceId
	},
	resource::{ReadStorageBackend, RedbStorageBackend, WriteStorageBackend},
};
use utils::{r#async::StreamExt, sync::Arc};

use crate::utils::get_asset_manager;

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
		log::info!("Baking resource '{}'", id);
		match asset_manager.bake(&id, &storage_backend).await {
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
