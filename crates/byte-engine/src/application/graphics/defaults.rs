//! Conventional setup components for [`GraphicsApplication`].
//!
//! [`default_setup`] is the batteries-included path used by the `triangle`
//! example. Applications that replace a subsystem can call the remaining setup
//! functions individually; the `window` example demonstrates that narrower
//! composition.

use resource_management::asset::{
	asset_manager::AssetManager,
	bema_asset_handler::{BEMAAssetHandler, ProgramGenerator},
	gltf_asset_handler::GLTFAssetHandler,
	lut_asset_handler::LUTAssetHandler,
	ogg_asset_handler::OGGAssetHandler,
	png_asset_handler::PNGAssetHandler,
	wav_asset_handler::WAVAssetHandler,
	FileStorageBackend,
};
use tracing::debug_span;
use utils::Extent;

use super::{setup_pbr_visibility_shading_render_pipeline, GraphicsApplication};
use crate::{
	application::{application::Application, parameters::Parameters as _, thread::Thread, Events},
	audio::audio_system::{AudioSystem, DefaultAudioSystem},
	core::listener::Listener as _,
	input::utils::{register_gamepad_device_class, register_keyboard_device_class, register_mouse_device_class},
	rendering::{pipelines::visibility::shader_generator::VisibilityShaderGenerator, window::Window},
};

/// Installs the standard assets, input devices, audio worker, visibility
/// rendering pipeline, and window.
pub fn default_setup(application: &mut GraphicsApplication) {
	let generator = VisibilityShaderGenerator::new(false, false, false, false, false, false, true, true);
	setup_default_resource_and_asset_management(application, generator);
	setup_default_input(application);
	setup_default_audio(application);
	setup_pbr_visibility_shading_render_pipeline(application);
	setup_default_window(application);
}

/// Creates the 1920x1080 window used by the default headed setup.
pub fn setup_default_window(application: &mut GraphicsApplication) {
	application
		.window_factory
		.0
		.create(Window::new(application.get_name(), Extent::rectangle(1920, 1080)));
}

/// Connects the asset directory and standard material, glTF, image, LUT, and
/// audio handlers to the application's resource manager.
pub fn setup_default_resource_and_asset_management(
	application: &mut GraphicsApplication,
	generator: impl ProgramGenerator + 'static,
) {
	let generator = std::sync::Arc::new(generator);
	let assets_path: std::path::PathBuf = application
		.get_parameter("assets-path")
		.map(|parameter| parameter.value.clone())
		.unwrap_or_else(|| "assets".into())
		.into();

	let storage_backend = FileStorageBackend::new(assets_path);
	let mut asset_manager = AssetManager::new(storage_backend);

	let mut material_asset_handler = BEMAAssetHandler::new();
	material_asset_handler.set_shader_generator(generator.clone());
	asset_manager.add_asset_handler(material_asset_handler);

	let mut gltf_asset_handler = GLTFAssetHandler::new();
	gltf_asset_handler.set_shader_generator(generator);
	asset_manager.add_asset_handler(gltf_asset_handler);
	asset_manager.add_asset_handler(PNGAssetHandler::new());
	asset_manager.add_asset_handler(LUTAssetHandler::new());
	asset_manager.add_asset_handler(WAVAssetHandler::new());
	asset_manager.add_asset_handler(OGGAssetHandler::new());

	application
		.resource_manager
		.try_map_mut(|resource_manager| resource_manager.set_asset_manager(asset_manager))
		.expect("Failed to set up resource manager. Application cannot run without a resource manager.");
}

/// Installs the device classes expected by [`super::process_default_window_input`].
pub fn setup_default_input(application: &mut GraphicsApplication) {
	let input_system = &mut application.input_system;
	let mouse = register_mouse_device_class(input_system);
	let keyboard = register_keyboard_device_class(input_system);
	let gamepad = register_gamepad_device_class(input_system);
	application.gamepad_device_class_handle = Some(gamepad);

	input_system.create_device(&mouse);
	input_system.create_device(&keyboard);
	input_system.create_device(&gamepad);
}

/// Starts the audio worker and connects generators created through the
/// application's generator factory.
pub fn setup_default_audio(application: &mut GraphicsApplication) {
	application
		.threads
		.push(Thread::new(application.application_events.0.spawn_rx(), {
			let mut generators_listener = application.generator_factory.listener();

			move |mut receiver| {
				let Ok(mut audio_system) = DefaultAudioSystem::try_new()
					.map_err(|error| format!("Failed to spawn audio system. No audio will play. Reason: {error}"))
					.warn()
				else {
					return;
				};

				let span = debug_span!("Render audio");
				let _entered = span.enter();

				loop {
					if let Ok(Events::Close) = receiver.try_recv() {
						break;
					}

					while let Some(message) = generators_listener.read() {
						audio_system.create_generator(message.into_data());
					}

					if !audio_system.render_available() {
						break;
					}
				}

				log::debug!("Exiting audio thread");
			}
		}));
}

trait LogResult {
	fn warn(self) -> Self;
}

impl<T, E: std::fmt::Display> LogResult for Result<T, E> {
	fn warn(self) -> Self {
		if let Err(error) = &self {
			log::warn!("{error}");
		}
		self
	}
}
