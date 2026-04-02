use resource_management::asset::{
	asset_manager::AssetManager, bema_asset_handler::BEMAAssetHandler, gltf_asset_handler::GLTFAssetHandler,
	lut_asset_handler::LUTAssetHandler, png_asset_handler::PNGAssetHandler, wav_asset_handler::WAVAssetHandler, StorageBackend,
};

pub fn get_asset_manager<SB: StorageBackend + 'static>(storage_backend: SB) -> AssetManager {
	let mut asset_manager = AssetManager::new(storage_backend);

	asset_manager.add_asset_handler(PNGAssetHandler::new());
	asset_manager.add_asset_handler(LUTAssetHandler::new());
	asset_manager.add_asset_handler(WAVAssetHandler::new());
	asset_manager.add_asset_handler(GLTFAssetHandler::new());

	{
		let mut material_asset_handler = BEMAAssetHandler::new();
		let shader_generator = {
			// let common_shader_generator = byte_engine::rendering::common_shader_generator::CommonShaderGenerator::new();
			let visibility_shader_generation =
				byte_engine::rendering::pipelines::visibility::shader_generator::VisibilityShaderGenerator::new(
					false, false, false, false, false, false, true, false,
				);
			visibility_shader_generation
		};
		material_asset_handler.set_shader_generator(shader_generator);
		asset_manager.add_asset_handler(material_asset_handler);
	}

	asset_manager
}
