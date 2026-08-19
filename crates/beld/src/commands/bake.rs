use std::{num::NonZeroUsize, path::Path, time::Instant};

use resource_management::{
	asset::{manager::AssetManager, FileStorageBackend},
	resource::{RedbStorageBackend, ResourceStorageMode},
};
use utils::{r#async::StreamExt, sync::Arc};

use crate::utils::get_asset_manager;

pub fn bake(
	source_path: String,
	destination_path: String,
	ids: Vec<String>,
	storage_mode: Option<ResourceStorageMode>,
	memory_budget: NonZeroUsize,
) -> Result<(), i32> {
	let source_path = std::path::PathBuf::from(source_path);

	let asset_storage_backend = FileStorageBackend::new(source_path.clone());

	let destination_path = destination_path.into();

	let resource_storage_backend = match storage_mode {
		Some(mode) => RedbStorageBackend::new_writable_with_mode(destination_path, mode).map_err(|error| {
			log::error!("Failed to bake resources. {error}");

			1
		})?,
		None => RedbStorageBackend::new_writable(destination_path),
	};

	let mut asset_manager = get_asset_manager(asset_storage_backend, resource_storage_backend);

	asset_manager.set_bake_memory_budget(memory_budget);

	log::info!(
		"Using a {} MiB soft memory budget for concurrent asset bakes.",
		memory_budget.get() / (1024 * 1024)
	);

	let ids = if ids.is_empty() {
		discover_asset_ids(&source_path, &asset_manager)?
	} else {
		ids
	};

	if ids.is_empty() {
		log::info!("No supported assets found to bake.");

		return Ok(());
	}

	let executor = resource_management::r#async::Executor::new().map_err(|_| 1)?;

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

	let (successful_count, failed_count) = executor.block_on(tasks);

	log::info!(
		"Processed {} assets in {:?}: {} succeeded, {} failed",
		resource_count,
		bake_start.elapsed(),
		successful_count,
		failed_count
	);

	if failed_count == 0 {
		Ok(())
	} else {
		Err(1)
	}
}

/// Finds supported source assets in the configured directory and its descendants.
pub(super) fn discover_asset_ids(source_path: &Path, asset_manager: &AssetManager) -> Result<Vec<String>, i32> {
	let mut ids = Vec::new();

	let canonical_root = std::fs::canonicalize(source_path).map_err(|error| {
		log::error!(
			"Failed to resolve assets directory '{}'. The most likely cause is that the directory does not exist or cannot be accessed. Error: {}",
			source_path.display(),
			error
		);
		1
	})?;

	let mut active_directories = std::collections::HashSet::from([canonical_root]);

	discover_asset_ids_in(source_path, source_path, asset_manager, &mut active_directories, &mut ids)?;

	ids.sort();

	Ok(ids)
}

/// Adds supported files from a directory tree without revisiting an active symlink ancestor.
fn discover_asset_ids_in(
	root_path: &Path,
	directory: &Path,
	asset_manager: &AssetManager,
	active_directories: &mut std::collections::HashSet<std::path::PathBuf>,
	ids: &mut Vec<String>,
) -> Result<(), i32> {
	let entries = std::fs::read_dir(directory).map_err(|error| {
		log::error!(
			"Failed to scan assets directory '{}'. The most likely cause is that the directory cannot be read. Error: {}",
			directory.display(),
			error
		);

		1
	})?;

	for entry in entries {
		let entry = entry.map_err(|error| {
			log::error!(
				"Failed to inspect an asset directory entry in '{}'. The most likely cause is a filesystem read failure. Error: {}",
				directory.display(),
				error
			);

			1
		})?;

		// Preserve the logical path used to enter a symlinked directory. Some platforms expose the resolved target path
		// through `DirEntry::path`, which would otherwise lose the mounted namespace when deriving the asset ID.
		let path = directory.join(entry.file_name());

		let file_type = entry.file_type().map_err(|error| {
			log::error!(
				"Failed to inspect asset path '{}'. The most likely cause is a filesystem metadata failure. Error: {}",
				path.display(),
				error
			);

			1
		})?;

		let followed_type = if file_type.is_symlink() {
			std::fs::metadata(&path)
				.map_err(|error| {
					log::error!(
						"Failed to follow asset symlink '{}'. The most likely cause is a broken link or inaccessible target. Error: {}",
						path.display(),
						error
					);

					1
				})?
				.file_type()
		} else {
			file_type
		};

		if followed_type.is_dir() {
			let canonical_directory = std::fs::canonicalize(&path).map_err(|error| {
				log::error!(
					"Failed to resolve asset directory '{}'. The most likely cause is a broken symlink or inaccessible directory. Error: {}",
					path.display(),
					error
				);

				1
			})?;

			if !active_directories.insert(canonical_directory.clone()) {
				log::warn!("Skipping cyclic asset directory link '{}'.", path.display());

				continue;
			}

			let result = discover_asset_ids_in(root_path, &path, asset_manager, active_directories, ids);

			active_directories.remove(&canonical_directory);

			result?;

			continue;
		}

		if !followed_type.is_file() {
			continue;
		}

		let Some(relative_path) = path.strip_prefix(root_path).ok() else {
			continue;
		};

		let Some(id) = resource_id_path(relative_path) else {
			log::warn!(
				"Skipping asset path '{}'. The most likely cause is a non-UTF-8 path that cannot be represented as a resource ID.",
				path.display()
			);

			continue;
		};

		if path
			.extension()
			.and_then(|extension| extension.to_str())
			.is_some_and(|extension| extension.eq_ignore_ascii_case("bead"))
		{
			continue;
		}

		let has_sidecar = path.with_added_extension("bead").is_file();

		if asset_manager.should_discover(&id, has_sidecar) {
			ids.push(id);
		}
	}

	Ok(())
}

/// Converts a relative file path to a resource ID that uses `/` separators.
fn resource_id_path(path: &Path) -> Option<String> {
	let mut id = String::new();

	for component in path.components() {
		let component = component.as_os_str().to_str()?;

		if !id.is_empty() {
			id.push('/');
		}

		id.push_str(component);
	}

	Some(id)
}
