#[cfg(not(test))]
use resource_management::resources::mips::gpu::MaterialMipGenerator;
use resource_management::{
	asset::{
		asset_manager::AssetManager, bema_asset_handler::BEMAAssetHandler, besl_shader_asset_handler::BESLShaderAssetHandler,
		exr_asset_handler::EXRAssetHandler, fbx_asset_handler::FBXAssetHandler, gltf_asset_handler::GLTFAssetHandler,
		ies_asset_handler::IESAssetHandler, lut_asset_handler::LUTAssetHandler, ogg_asset_handler::OGGAssetHandler,
		pipeline_asset_handler::PipelineAssetHandler, png_asset_handler::PNGAssetHandler, wav_asset_handler::WAVAssetHandler,
		StorageBackend,
	},
	ibl::IBLGenerator,
	resources::mips::{CPUMipGenerationBackend, MipGenerationBackend},
};

pub fn get_asset_manager<AS, RS>(storage_backend: AS, resource_storage_backend: RS) -> AssetManager
where
	AS: StorageBackend + 'static,
	RS: resource_management::resource::StorageBackend + 'static,
{
	let mut asset_manager = AssetManager::new(storage_backend, resource_storage_backend);

	asset_manager.add_asset_handler(PNGAssetHandler::new());
	asset_manager.add_asset_handler(IESAssetHandler::new());
	#[cfg(not(test))]
	let ibl_generator = IBLGenerator::try_with_default_gpu().unwrap_or_else(|error| {
		log::warn!(
			"GPU environment-map setup failed; using CPU generation. The most likely cause is that no compatible compute device is available. Error: {error}"
		);
		IBLGenerator::new()
	});
	#[cfg(test)]
	let ibl_generator = IBLGenerator::new();
	asset_manager.add_asset_handler(EXRAssetHandler::new(ibl_generator));
	asset_manager.add_asset_handler(LUTAssetHandler::new());
	asset_manager.add_asset_handler(WAVAssetHandler::new());
	asset_manager.add_asset_handler(OGGAssetHandler::new());
	asset_manager.add_asset_handler(PipelineAssetHandler);
	let mut besl_shader_asset_handler = BESLShaderAssetHandler::new();
	besl_shader_asset_handler
		.set_shader_generator(byte_engine::rendering::common_shader_generator::CommonShaderGenerator::new());
	asset_manager.add_asset_handler(besl_shader_asset_handler);
	{
		#[cfg(not(test))]
		let material_mip_generator: std::sync::Arc<dyn MipGenerationBackend> = MaterialMipGenerator::try_with_default_gpu()
			.map(|generator| std::sync::Arc::new(generator) as std::sync::Arc<dyn MipGenerationBackend>)
			.unwrap_or_else(|error| {
				log::warn!(
					"GPU material mip setup failed; using CPU generation. The most likely cause is that no compatible compute device is available. Error: {error}"
				);
				std::sync::Arc::new(CPUMipGenerationBackend)
			});
		#[cfg(test)]
		let material_mip_generator: std::sync::Arc<dyn MipGenerationBackend> = std::sync::Arc::new(CPUMipGenerationBackend);

		let mut material_asset_handler = BEMAAssetHandler::new();
		let shader_generator = std::sync::Arc::new({
			// let common_shader_generator = byte_engine::rendering::common_shader_generator::CommonShaderGenerator::new();

			byte_engine::rendering::pipelines::visibility::shader_generator::VisibilityShaderGenerator::new(
				true, true, true, true, true, true, true, true,
			)
		});
		material_asset_handler.set_shader_generator(shader_generator.clone());
		asset_manager.add_asset_handler(material_asset_handler);

		let mut fbx_asset_handler = FBXAssetHandler::new();
		fbx_asset_handler.set_shader_generator(shader_generator.clone());
		fbx_asset_handler.set_material_mip_generator(material_mip_generator.clone());
		asset_manager.add_asset_handler(fbx_asset_handler);

		let mut gltf_asset_handler = GLTFAssetHandler::new();
		gltf_asset_handler.set_shader_generator(shader_generator);
		gltf_asset_handler.set_material_mip_generator(material_mip_generator);
		asset_manager.add_asset_handler(gltf_asset_handler);
	}

	asset_manager
}

#[cfg(test)]
mod tests {
	use std::time::{SystemTime, UNIX_EPOCH};

	use resource_management::{
		asset::{storage_backend::FileStorageBackend, ResourceId, StorageBackend},
		r#async::Executor,
		resource::storage_backend::{redb_storage_backend::RedbStorageBackend, ReadStorageBackend},
		resources::mesh::MeshModel,
		ReferenceModel,
	};

	use super::get_asset_manager;

	const TRIANGLE_MOVE_FBX: &[u8] = include_bytes!("../../resource-management/src/asset/test_data/triangle_move_ascii.fbx");

	struct EmptyAssetStorage;

	impl StorageBackend for EmptyAssetStorage {}

	#[test]
	fn default_asset_manager_registers_shader_and_pipeline_handlers() {
		let resources_path = std::env::temp_dir().join(format!(
			"beld-registration-test-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		));
		let asset_manager = get_asset_manager(EmptyAssetStorage, RedbStorageBackend::new(resources_path.clone()));

		assert!(asset_manager.supports("byte-engine/render-passes/resolve.besl"));
		assert!(asset_manager.supports("byte-engine/rendering/visibility/visibility.pipeline"));
		assert!(asset_manager.supports("lights/office.ies"));
		assert!(!asset_manager.should_discover("byte-engine/render-passes/resolve.besl", false));
		assert!(asset_manager.should_discover("byte-engine/render-passes/resolve.besl", true));
		drop(asset_manager);
		std::fs::remove_dir_all(resources_path).unwrap();
	}

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
				RedbStorageBackend::new(resources_path.clone()),
			);
			let mesh: ReferenceModel<MeshModel> = asset_manager.bake_if_not_exists("triangle_move.fbx").await.expect(
				"FBX mesh baking failed. The most likely cause is broken BELD handler or shader-generator registration.",
			);
			drop(asset_manager);
			let resource_storage = RedbStorageBackend::new(resources_path);
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
}
