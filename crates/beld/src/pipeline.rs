//! Compose the steps of a bake to build your own version of BELD.
//!
//! [`crate::bake`] runs these steps in order. A custom tool runs the same steps and adds its own between them, such as
//! a [`ResourceValidator`](resource_management::asset::validation::ResourceValidator) that enforces a size limit:
//!
//! 1. [`open_asset_storage`] and [`open_resource_storage`] open the source and destination directories.
//! 2. [`default_asset_manager`] builds the manager with the engine's asset handlers. Register extra handlers or
//!    validators on it, and set its memory budget.
//! 3. [`select_ids`] chooses what to bake.
//! 4. [`bake_ids`] bakes them and returns a [`BakeReport`].

use std::{num::NonZeroUsize, path::Path, time::Instant};

use byte_engine::rendering::pipelines::visibility::VisibilityFeatures;
use resource_management::{
	asset::{FileStorageBackend, StorageBackend, manager::AssetManager, manager::LoadMessages},
	resource::{ReDBStorageBackend, ResourceGpuCompressionPolicy, ResourceStorageMode, ResourceStorageSettings},
};
use utils::r#async::StreamExt;

use crate::{BeldError, commands::shared::offload_file_operation};

/// The number of root bakes [`bake_ids`] keeps in flight at once.
const BAKE_CONCURRENCY: usize = 16;

/// Opens the source asset directory a bake reads from, creating it when it is missing.
///
/// Next, open the destination with [`open_resource_storage`] and pass both to [`default_asset_manager`].
pub async fn open_asset_storage(source_path: &Path) -> Result<FileStorageBackend, BeldError> {
	FileStorageBackend::open(source_path.to_path_buf()).await.map_err(|error| {
		BeldError::new(format!(
			"Failed to open assets directory '{}'. The most likely cause is that the directory cannot be created or accessed. Check the path and its permissions. Error: {error}",
			source_path.display()
		))
	})
}

/// Opens or creates the resource store a bake writes to.
///
/// `storage_mode` only applies to a new store; an existing store keeps its persisted mode. `texture_compression`
/// selects native transport compression for baked textures, and `None` keeps the store's setting.
///
/// Next, pass the store to [`default_asset_manager`].
pub async fn open_resource_storage(
	destination_path: &Path,
	storage_mode: Option<ResourceStorageMode>,
	texture_compression: Option<ResourceGpuCompressionPolicy>,
) -> Result<ReDBStorageBackend, BeldError> {
	let destination_path = destination_path.to_path_buf();
	offload_file_operation(move || match texture_compression {
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
	.map_err(|error| BeldError::new(format!("Failed to bake resources. {error}")))
}

/// Creates the asset manager BELD bakes with, registering the same handlers as the engine's debug runtime.
///
/// Material shaders are generated for the visibility renderer with the project's `features`. Tests bake with the CPU
/// texture backends so they do not depend on a GPU.
///
/// Next, register any extra handlers with [`AssetManager::add_asset_handler`] and project rules with
/// [`AssetManager::add_resource_validator`], set [`AssetManager::set_bake_memory_budget`], then call [`select_ids`].
pub fn default_asset_manager<AS, RS>(
	storage_backend: AS,
	resource_storage_backend: RS,
	features: VisibilityFeatures,
) -> AssetManager
where
	AS: StorageBackend + 'static,
	RS: resource_management::resource::StorageBackend + 'static,
{
	let mut asset_manager = AssetManager::new(storage_backend, resource_storage_backend);

	// A new asset manager already processes images on the CPU, which tests keep.
	#[cfg(not(test))]
	asset_manager.set_image_processor(byte_engine::application::graphics::default_image_processor());

	byte_engine::application::graphics::register_default_asset_handlers(
		&mut asset_manager,
		byte_engine::rendering::pipelines::visibility::VisibilityShaderGenerator::with_features(features),
		features,
	);

	asset_manager
}

/// Returns `ids`, or every discoverable asset the project's `features` use when `ids` is empty.
///
/// Explicitly requested IDs are returned unchanged; only discovery skips the engine assets that left-out features use.
///
/// Next, pass the IDs to [`bake_ids`].
pub async fn select_ids(
	asset_manager: &AssetManager,
	ids: Vec<String>,
	features: VisibilityFeatures,
) -> Result<Vec<String>, BeldError> {
	if !ids.is_empty() {
		return Ok(ids);
	}

	let mut ids = asset_manager.discover().await.map_err(|error| {
		BeldError::new(format!(
			"Failed to discover assets. The most likely cause is an unreadable file below the source directory. Check its permissions. Error: {error}"
		))
	})?;
	ids.retain(|id| !features.excludes(id));
	Ok(ids)
}

/// The `BakeReport` struct tells the caller of [`bake_ids`] which assets baked and which failed, so it can decide what
/// to do next, such as exit with an error or publish the resources that baked.
#[derive(Clone, Debug, Default)]
pub struct BakeReport {
	/// The IDs whose resource is now current, whether it was baked or reused.
	pub succeeded: Vec<String>,
	/// The IDs that failed, with the reason each one failed.
	pub failed: Vec<(String, LoadMessages)>,
}

/// Bakes every stale ID in `ids`, up to 16 at once, and reports which succeeded and which failed.
///
/// Each outcome is logged as it completes. Set [`AssetManager::set_bake_memory_budget`] before the call to bound how
/// much memory concurrent bakes retain.
///
/// Next, check [`BakeReport::failed`], then read the destination with [`crate::list`] or [`crate::inspect`].
pub async fn bake_ids(asset_manager: &AssetManager, ids: Vec<String>) -> BakeReport {
	let resource_count = ids.len();

	// Every bake borrows the same manager; the futures only run inside the stream below, while it is still alive.
	let tasks = ids.into_iter().map(async |id| {
		log::info!("Baking resource '{}'", id);

		match asset_manager.bake_if_stale(&id).await {
			Ok(()) => {
				// A current resource is reused rather than baked, so report the outcome both paths share.
				log::info!("Resource '{}' is up to date", id);

				Ok(id)
			}
			Err(error) => {
				log::error!("Failed to bake '{}'. Error: {:#?}", id, error);

				Err((id, error))
			}
		}
	});

	let bake_start = Instant::now();

	let mut report = BakeReport {
		succeeded: Vec::with_capacity(resource_count),
		failed: Vec::new(),
	};
	let mut outcomes = utils::r#async::stream::iter(tasks).buffer_unordered(BAKE_CONCURRENCY);
	while let Some(outcome) = outcomes.next().await {
		match outcome {
			Ok(id) => report.succeeded.push(id),
			Err(failure) => report.failed.push(failure),
		}
	}

	log::info!(
		"Processed {} assets in {:?}: {} succeeded, {} failed",
		resource_count,
		bake_start.elapsed(),
		report.succeeded.len(),
		report.failed.len()
	);

	report
}

/// Returns the configured bake budget, or half of the available system memory, and never less than a quarter of the
/// installed memory.
///
/// The floor matters on macOS, where the available figure subtracts compressed pages and reaches zero under memory
/// pressure. A zero budget pauses every independent bake while any other bake retains memory, which serializes the
/// whole run. The budget is soft, so the floor only bounds how much work starts at once.
///
/// Pass the result to [`AssetManager::set_bake_memory_budget`].
pub fn default_bake_memory_budget() -> NonZeroUsize {
	let mut system = sysinfo::System::new();
	system.refresh_memory();
	let available_bytes = system.available_memory().max(system.total_memory() / 2);
	let budget_bytes = usize::try_from(available_bytes / 2).unwrap_or(usize::MAX);
	NonZeroUsize::new(budget_bytes).unwrap_or(NonZeroUsize::MIN)
}

#[cfg(test)]
mod tests {

	use std::time::{SystemTime, UNIX_EPOCH};

	use resource_management::{
		ReferenceModel,
		asset::validation::{ResourceCandidate, ResourceValidator, ValidationError},
		asset::{ResourceId, storage_backend::FileStorageBackend},
		r#async::Executor,
		resource::storage_backend::{ReadStorageBackend, redb::ReDBStorageBackend},
		resources::mesh::MeshModel,
	};

	use super::{VisibilityFeatures, bake_ids, default_asset_manager, open_asset_storage};

	const TRIANGLE_MOVE_FBX: &[u8] = include_bytes!("../../resource-management/src/asset/test_data/triangle_move_ascii.fbx");

	/// Confirms that the production handlers bake an FBX mesh and its visibility material dependencies.
	#[test]
	fn default_asset_manager_bakes_fbx_mesh_and_generated_materials() {
		let executor =
			Executor::new().expect("Async runtime could not start. The most likely cause is unavailable platform I/O support.");

		executor.block_on(async {
			let root = std::env::temp_dir().join(format!(
				"beld-fbx-test-{}-{}",
				std::process::id(),
				SystemTime::now()
					.duration_since(UNIX_EPOCH)
					.expect("System clock is invalid. The most likely cause is a clock value before the Unix epoch.")
					.as_nanos()
			));

			let assets_path = root.join("assets");

			let resources_path = root.join("resources");

			std::fs::create_dir_all(&assets_path)
				.expect("FBX test assets could not be created. The most likely cause is an unwritable temporary directory.");

			std::fs::write(assets_path.join("triangle_move.fbx"), TRIANGLE_MOVE_FBX)
				.expect("FBX test asset could not be written. The most likely cause is an unwritable temporary directory.");

			let asset_manager = default_asset_manager(
				FileStorageBackend::new(assets_path),
				ReDBStorageBackend::new(resources_path.clone()),
				VisibilityFeatures::default(),
			);

			let mesh: ReferenceModel<MeshModel> = asset_manager.bake_if_not_exists("triangle_move.fbx").await.expect(
				"FBX mesh baking failed. The most likely cause is broken BELD handler or shader-generator registration.",
			);

			drop(asset_manager);

			let resource_storage = ReDBStorageBackend::new(resources_path);

			let (serialized, _) = resource_storage
				.read(ResourceId::new("triangle_move.fbx"))
				.await
				.expect("Baked FBX mesh is missing. The most likely cause is a resource storage failure.");

			let streams = serialized
				.streams()
				.expect("Baked FBX mesh streams are missing. The most likely cause is a mesh serialization regression.");

			assert_eq!(mesh.class(), "Mesh");

			for expected in ["Vertex.Position", "Vertex.Normal", "Vertex.UV"] {
				assert!(streams.iter().any(|stream| stream.name() == expected));
			}

			let resources = resource_storage
				.list()
				.await
				.expect("Resource list is unreadable. The most likely cause is a test storage failure.");

			assert_eq!(resources.len(), 5);
			assert!(resources.iter().any(|resource| resource == "triangle_move.fbx#skeleton"));

			drop(resource_storage);

			std::fs::remove_dir_all(root)
				.expect("FBX test directory could not be removed. The most likely cause is an open resource file.");
		});
	}

	/// The `MaxPayloadSize` struct rejects resources whose payload is larger than a byte limit, like a project rule.
	struct MaxPayloadSize(usize);

	impl ResourceValidator for MaxPayloadSize {
		fn validate(&self, candidate: &ResourceCandidate<'_>) -> Result<(), ValidationError> {
			if candidate.payload_size() <= self.0 {
				return Ok(());
			}

			Err(ValidationError::new(
				format!(
					"The payload is {} bytes, over the {}-byte limit.",
					candidate.payload_size(),
					self.0
				),
				"The most likely cause is an oversized source asset.",
				"Reduce the source asset's size.",
			))
		}
	}

	/// Builds a 16-bit stereo PCM WAV file holding `frame_count` silent frames.
	fn silent_wav(frame_count: u32) -> Vec<u8> {
		let data_size = frame_count * 4;
		let mut wav = Vec::with_capacity(44 + data_size as usize);
		wav.extend_from_slice(b"RIFF");
		wav.extend_from_slice(&(36 + data_size).to_le_bytes());
		wav.extend_from_slice(b"WAVEfmt ");
		wav.extend_from_slice(&16u32.to_le_bytes());
		wav.extend_from_slice(&1u16.to_le_bytes());
		wav.extend_from_slice(&2u16.to_le_bytes());
		wav.extend_from_slice(&48_000u32.to_le_bytes());
		wav.extend_from_slice(&192_000u32.to_le_bytes());
		wav.extend_from_slice(&4u16.to_le_bytes());
		wav.extend_from_slice(&16u16.to_le_bytes());
		wav.extend_from_slice(b"data");
		wav.extend_from_slice(&data_size.to_le_bytes());
		wav.resize(44 + data_size as usize, 0);
		wav
	}

	/// Confirms that a pipeline composed with a validator bakes small assets and rejects large ones without storing them.
	#[test]
	fn a_composed_pipeline_rejects_resources_over_its_validator_limit() {
		let executor =
			Executor::new().expect("Async runtime could not start. The most likely cause is unavailable platform I/O support.");
		let root = std::env::temp_dir().join(format!(
			"beld-validation-test-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		));
		let assets_path = root.join("assets");
		let resources_path = root.join("resources");
		std::fs::create_dir_all(&assets_path).unwrap();
		std::fs::write(assets_path.join("small.wav"), silent_wav(128)).unwrap();
		std::fs::write(assets_path.join("large.wav"), silent_wav(1024)).unwrap();

		executor.block_on(async {
			let assets = open_asset_storage(&assets_path).await.unwrap();
			let mut asset_manager = default_asset_manager(
				assets,
				ReDBStorageBackend::new(resources_path.clone()),
				VisibilityFeatures::default(),
			);
			asset_manager.add_resource_validator(MaxPayloadSize(1024));

			let report = bake_ids(&asset_manager, vec!["small.wav".to_string(), "large.wav".to_string()]).await;
			drop(asset_manager);

			assert_eq!(report.succeeded, ["small.wav"]);
			assert_eq!(report.failed.len(), 1);
			assert_eq!(report.failed[0].0, "large.wav");

			let resource_storage = ReDBStorageBackend::new(resources_path);
			assert!(resource_storage.read(ResourceId::new("small.wav")).await.is_some());
			assert!(resource_storage.read(ResourceId::new("large.wav")).await.is_none());
		});

		std::fs::remove_dir_all(root).unwrap();
	}
}
