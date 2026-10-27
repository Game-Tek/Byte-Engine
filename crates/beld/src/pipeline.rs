//! Compose the steps of a bake to build your own version of BELD.
//!
//! [`crate::bake`] runs these steps in order. A custom tool runs the same steps and adds its own between them, such as
//! a [`ResourceValidator`](resource_management::asset::validation::ResourceValidator) that enforces a size limit:
//! open storage with [`open_asset_storage`] and [`open_resource_storage`], build the manager with
//! [`default_asset_manager`], register extra rules on it, then call [`bake_ids`].
//!
//! Each step logs its failure and returns BELD's exit code, like the commands.

use std::{path::PathBuf, time::Instant};

use byte_engine::rendering::pipelines::visibility::VisibilityFeatures;
use resource_management::{
	asset::{FileStorageBackend, StorageBackend, manager::AssetManager, manager::LoadMessages},
	resource::{ReDBStorageBackend, ResourceGpuCompressionPolicy, ResourceStorageMode, ResourceStorageSettings},
};
use utils::r#async::StreamExt;

use crate::commands::shared::offload_file_operation;

/// Opens the source asset directory a bake reads from.
///
/// Next, open the destination with [`open_resource_storage`].
pub async fn open_asset_storage(source_path: PathBuf) -> Result<FileStorageBackend, i32> {
	FileStorageBackend::open(source_path.clone()).await.map_err(|error| {
		log::error!(
			"Failed to open assets directory '{}'. The most likely cause is that the directory cannot be created or accessed. Error: {error}",
			source_path.display()
		);
		1
	})
}

/// Opens or creates the resource store a bake writes to.
///
/// `storage_mode` only applies to a new store; an existing store keeps its persisted mode.
///
/// Next, pass both stores to [`default_asset_manager`].
pub async fn open_resource_storage(
	destination_path: PathBuf,
	storage_mode: Option<ResourceStorageMode>,
	texture_compression: Option<ResourceGpuCompressionPolicy>,
) -> Result<ReDBStorageBackend, i32> {
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
	.map_err(|error| {
		log::error!("Failed to bake resources. {error}");
		1
	})
}

/// Creates the asset manager BELD bakes with, registering the same handlers as the engine's debug runtime.
///
/// Material shaders are generated for the visibility renderer with the project's `features`. Tests bake with the CPU
/// texture backends so they do not depend on a GPU.
///
/// Next, register project rules with [`AssetManager::add_resource_validator`], set
/// [`AssetManager::set_bake_memory_budget`], and call [`bake_ids`].
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

/// Bakes every ID that is stale, up to 16 at once, logs each outcome, and returns the IDs that failed with their
/// errors.
pub async fn bake_ids(asset_manager: &AssetManager, ids: Vec<String>) -> Vec<(String, LoadMessages)> {
	let resource_count = ids.len();

	// Every bake borrows the same manager; the futures only run inside `tasks.await` below, while it is still alive.
	let tasks = ids.into_iter().map(async |id| {
		log::info!("Baking resource '{}'", id);

		match asset_manager.bake_if_stale(&id).await {
			Ok(_) => {
				// A current resource is reused rather than baked, so report the outcome both paths share.
				log::info!("Resource '{}' is up to date", id);

				None
			}
			Err(e) => {
				log::error!("Failed to bake '{}'. Error: {:#?}", id, e);

				Some((id, e))
			}
		}
	});

	let bake_start = Instant::now();

	let failures = utils::r#async::stream::iter(tasks)
		.buffer_unordered(16)
		.filter_map(async |failure| failure)
		.collect::<Vec<_>>()
		.await;

	log::info!(
		"Processed {} assets in {:?}: {} succeeded, {} failed",
		resource_count,
		bake_start.elapsed(),
		resource_count - failures.len(),
		failures.len()
	);

	failures
}

#[cfg(test)]
mod tests {

	use std::time::{SystemTime, UNIX_EPOCH};

	use resource_management::{
		ProcessedAsset, ReferenceModel,
		asset::validation::ResourceValidator,
		asset::{ResourceId, storage_backend::FileStorageBackend},
		r#async::Executor,
		resource::storage_backend::{ReadStorageBackend, redb::ReDBStorageBackend},
		resources::mesh::MeshModel,
	};

	use super::{VisibilityFeatures, bake_ids, default_asset_manager};

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

	/// Rejects resources whose payload is larger than a byte limit, like a project rule.
	struct MaxPayloadSize(usize);

	impl ResourceValidator for MaxPayloadSize {
		fn validate(&self, _: &ProcessedAsset, payload_size: usize) -> Result<(), String> {
			if payload_size <= self.0 {
				return Ok(());
			}
			Err(format!(
				"The payload is {payload_size} bytes, over the {}-byte limit.",
				self.0
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
		for field in [
			16u32.to_le_bytes(),
			[1, 0, 2, 0],
			48_000u32.to_le_bytes(),
			192_000u32.to_le_bytes(),
			[4, 0, 16, 0],
		] {
			wav.extend_from_slice(&field);
		}
		wav.extend_from_slice(b"data");
		wav.extend_from_slice(&data_size.to_le_bytes());
		wav.resize(44 + data_size as usize, 0);
		wav
	}

	/// Confirms that a pipeline composed with a validator bakes small assets and rejects large ones without storing them.
	#[test]
	fn a_composed_pipeline_rejects_resources_over_its_validator_limit() {
		let executor = Executor::new().unwrap();
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
			let mut asset_manager = default_asset_manager(
				FileStorageBackend::new(assets_path),
				ReDBStorageBackend::new(resources_path.clone()),
				VisibilityFeatures::default(),
			);
			asset_manager.add_resource_validator(MaxPayloadSize(1024));

			let failures = bake_ids(&asset_manager, vec!["small.wav".to_string(), "large.wav".to_string()]).await;
			drop(asset_manager);

			assert_eq!(failures.len(), 1);
			assert_eq!(failures[0].0, "large.wav");

			let resource_storage = ReDBStorageBackend::new(resources_path);
			assert!(resource_storage.read(ResourceId::new("small.wav")).await.is_some());
			assert!(resource_storage.read(ResourceId::new("large.wav")).await.is_none());
		});

		std::fs::remove_dir_all(root).unwrap();
	}
}
