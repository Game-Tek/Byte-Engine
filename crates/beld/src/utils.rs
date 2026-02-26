use resource_management::asset::{StorageBackend, asset_manager::AssetManager, audio_asset_handler::AudioAssetHandler, image_asset_handler::ImageAssetHandler, material_asset_handler::MaterialAssetHandler, mesh_asset_handler::MeshAssetHandler};

pub fn get_asset_manager<SB: StorageBackend + 'static>(storage_backend: SB) -> AssetManager {
	let mut asset_manager = AssetManager::new(storage_backend);

	asset_manager.add_asset_handler(ImageAssetHandler::new());
	asset_manager.add_asset_handler(AudioAssetHandler::new());
	asset_manager.add_asset_handler(MeshAssetHandler::new());

	{
		let mut material_asset_handler = MaterialAssetHandler::new();
		let shader_generator = {
			// let common_shader_generator = byte_engine::rendering::common_shader_generator::CommonShaderGenerator::new();
			let visibility_shader_generation = byte_engine::rendering::pipelines::visibility::shader_generator::VisibilityShaderGenerator::new(false, false, false, false, false, false, true, false);
			visibility_shader_generation
		};
		material_asset_handler.set_shader_generator(shader_generator);
		asset_manager.add_asset_handler(material_asset_handler);
	}

	asset_manager
}
