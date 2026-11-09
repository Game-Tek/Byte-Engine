use byte_engine::application::{Parameter, parameters::Parameters as _};
use resource_management::asset::{StorageBackend, manager::AssetManager};
use utils::Glob;

/// Creates the asset manager BELD bakes with, with the bake services and asset handlers `configure` installs for the
/// application's `parameters`.
///
/// `configure` is the function the application's debug runtime also configures its manager with, so both bake the same
/// resources with the same services.
pub fn get_asset_manager<AS, RS>(
	storage_backend: AS,
	resource_storage_backend: RS,
	parameters: &[Parameter],
	configure: impl FnOnce(&mut AssetManager, &[Parameter]),
) -> AssetManager
where
	AS: StorageBackend + 'static,
	RS: resource_management::resource::StorageBackend + 'static,
{
	let mut asset_manager = AssetManager::new(storage_backend, resource_storage_backend);
	configure(&mut asset_manager, parameters);
	asset_manager
}

/// Configures `asset_manager` as [`configure_default_baking`] does, but with the CPU texture backends, so tests don't
/// depend on a GPU.
///
/// [`configure_default_baking`]: byte_engine::application::graphics::configure_default_baking
#[cfg(test)]
pub(crate) fn configure_cpu_baking(asset_manager: &mut AssetManager, parameters: &[Parameter]) {
	byte_engine::application::graphics::configure_default_baking(asset_manager, parameters);
	asset_manager.set_image_processor(resource_management::processors::image::ImageProcessor::default());
}

/// Returns the glob patterns of the array parameter `name`, such as `resources.roots`.
pub fn globs(parameters: &[Parameter], name: &str) -> Vec<Glob> {
	parameters
		.get_elements(name)
		.map(|pattern| Glob::new(pattern.value()))
		.collect()
}

#[cfg(test)]
mod tests {

	use std::time::{SystemTime, UNIX_EPOCH};

	use resource_management::{
		ReferenceModel,
		asset::{ResourceId, storage_backend::FileStorageBackend},
		r#async::Executor,
		resource::storage_backend::{ReadStorageBackend, redb::ReDBStorageBackend},
		resources::mesh::MeshModel,
	};

	use super::configure_cpu_baking;
	use super::get_asset_manager;

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

			let asset_manager = get_asset_manager(
				FileStorageBackend::new(assets_path),
				ReDBStorageBackend::new(resources_path.clone()).unwrap(),
				&[],
				configure_cpu_baking,
			);

			let mesh: ReferenceModel<MeshModel> = asset_manager.bake_if_not_exists("triangle_move.fbx").await.expect(
				"FBX mesh baking failed. The most likely cause is broken BELD handler or shader-generator registration.",
			);

			drop(asset_manager);

			let resource_storage = ReDBStorageBackend::new(resources_path).unwrap();

			let (serialized, _) = resource_storage
				.read(ResourceId::new("triangle_move.fbx"))
				.await
				.expect("Baked FBX mesh is missing. The most likely cause is a resource storage failure.");

			let streams = serialized
				.streams()
				.expect("Baked FBX mesh streams are missing. The most likely cause is a mesh serialization regression.");

			assert_eq!(mesh.class(), "Mesh");

			for expected in ["level[0].Vertex.Position", "level[0].Vertex.Normal", "level[0].Vertex.UV"] {
				assert!(streams.iter().any(|stream| stream.name() == expected));
			}

			let resources = resource_storage
				.list()
				.await
				.expect("Resource list is unreadable. The most likely cause is a test storage failure.");

			assert_eq!(resources.len(), 5);
			assert!(resources.iter().any(|resource| resource.id() == "triangle_move.fbx#skeleton"));

			drop(resource_storage);

			std::fs::remove_dir_all(root)
				.expect("FBX test directory could not be removed. The most likely cause is an open resource file.");
		});
	}
}
