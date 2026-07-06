use resource_management::asset::{
	asset_manager::AssetManager, bema_asset_handler::BEMAAssetHandler, gltf_asset_handler::GLTFAssetHandler,
	lut_asset_handler::LUTAssetHandler, ogg_asset_handler::OGGAssetHandler, png_asset_handler::PNGAssetHandler,
	wav_asset_handler::WAVAssetHandler, StorageBackend,
};

pub fn get_asset_manager<SB: StorageBackend + 'static>(storage_backend: SB) -> AssetManager {
	let mut asset_manager = AssetManager::new(storage_backend);

	asset_manager.add_asset_handler(PNGAssetHandler::new());
	asset_manager.add_asset_handler(LUTAssetHandler::new());
	asset_manager.add_asset_handler(WAVAssetHandler::new());
	asset_manager.add_asset_handler(OGGAssetHandler::new());
	{
		let mut material_asset_handler = BEMAAssetHandler::new();
		let shader_generator = std::sync::Arc::new({
			// let common_shader_generator = byte_engine::rendering::common_shader_generator::CommonShaderGenerator::new();

			byte_engine::rendering::pipelines::visibility::shader_generator::VisibilityShaderGenerator::new(
				false, false, false, false, false, false, true, true,
			)
		});
		material_asset_handler.set_shader_generator(shader_generator.clone());
		asset_manager.add_asset_handler(material_asset_handler);

		let mut gltf_asset_handler = GLTFAssetHandler::new();
		gltf_asset_handler.set_shader_generator(shader_generator);
		asset_manager.add_asset_handler(gltf_asset_handler);
	}

	asset_manager
}
