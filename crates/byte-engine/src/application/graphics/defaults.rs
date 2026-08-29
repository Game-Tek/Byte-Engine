//! Conventional setup components for [`GraphicsApplication`].
//!
//! [`default_setup`] is the batteries-included path used by the `triangle`
//! example. Applications that replace a subsystem can call the remaining setup
//! functions individually; the `window` example demonstrates that narrower
//! composition.

/// Installs the standard assets, input devices, audio worker, visibility
/// rendering pipeline, and window.
///
/// After setup, create application actions through
/// [`GraphicsApplication::action_factory`], select scene lighting through
/// [`crate::gameplay::world::DefaultWorld::factory`], and run
/// the application with [`GraphicsApplication::do_loop`].
pub fn default_setup(application: &mut GraphicsApplication) {
	#[cfg(debug_assertions)]
	{
		let generator = VisibilityShaderGenerator::new(true, false, false, false, false, false, true, true);

		setup_default_resource_and_asset_management(application, generator);
	}

	setup_default_input(application);

	setup_default_pipeline_compilation(application);

	let mut loading_tasks = build_deferred_tasks_queue();

	setup_default_audio(application, |task| {
		loading_tasks.push(task);
	});

	setup_pbr_visibility_shading_render_pipeline(application, |task| {
		loading_tasks.push(task);
	});

	setup_default_window(application);

	launch_deferred_tasks_thread(application, loading_tasks);
}

/// Starts the renderer's pending pipeline compiler servers on application-owned threads.
///
/// This setup is idempotent. Call it when composing a graphics application
/// without [`default_setup`] before registering pipeline managers or render
/// passes that request asynchronous pipelines.
pub fn setup_default_pipeline_compilation(application: &mut GraphicsApplication) {
	let servers = application.renderer.take_pipeline_compilation_servers();

	for server in servers {
		application
			.threads
			.push(Thread::new(application.application_events.0.listener(), move |mut events| {
				let runtime = build_single_threaded_async_runtime();

				runtime.enter(|| {
					runtime.spawn(server.run()).detach();

					loop {
						if matches!(events.read(), Some(Events::Close)) {
							return;
						}

						let ready = runtime.run();

						runtime.poll_with(Some(if ready {
							std::time::Duration::ZERO
						} else {
							std::time::Duration::from_millis(10)
						}));
					}
				});
			}));
	}
}

/// Runs deferred loading tasks on an application-owned asynchronous worker.
///
/// Build `tasks` with [`build_deferred_tasks_queue`], then launch the worker
/// after every subsystem has registered its loading work.
pub fn launch_deferred_tasks_thread(application: &mut GraphicsApplication, tasks: DeferredTasks) {
	application
		.threads
		.push(Thread::new(application.application_events.0.listener(), move |mut events| {
			let runtime = build_single_threaded_async_runtime();

			// Compio separates task execution from I/O polling. Enter the runtime so
			// resource futures can access it, then drive both halves until shutdown.
			runtime.enter(|| {
				for task in tasks {
					task(&runtime);
				}

				loop {
					if let Some(Events::Close) = events.read() {
						return;
					}

					let has_ready_tasks = runtime.run();

					let timeout = has_ready_tasks
						.then_some(std::time::Duration::ZERO)
						.or(Some(std::time::Duration::from_millis(6)));

					runtime.poll_with(timeout);
				}
			});
		}));
}

/// Creates the single-threaded runtime used by default background workers.
pub fn build_single_threaded_async_runtime() -> compio::runtime::Runtime {
	compio::runtime::Runtime::new().unwrap()
}

/// A loading operation that spawns its work on the provided runtime.
pub type DeferredTask = Box<dyn FnOnce(&compio::runtime::Runtime) + Send>;

/// The loading operations collected while default subsystems are configured.
pub type DeferredTasks = Vec<DeferredTask>;

/// Creates a deferred-task queue sized for the default graphics setup.
///
/// Add subsystem loading tasks, then pass the queue to
/// [`launch_deferred_tasks_thread`].
pub fn build_deferred_tasks_queue() -> DeferredTasks {
	Vec::with_capacity(8)
}

/// Creates the 1920x1080 window used by the default headed setup.
pub fn setup_default_window(application: &mut GraphicsApplication) {
	application
		.window_factory
		.0
		.create(Window::new(application.get_name(), Extent::rectangle(1920, 1080)));
}

/// In debug builds, connects the asset directory and standard material, model,
/// image, audio, and standalone-shader handlers to the resource manager.
///
/// Release builds intentionally leave the manager without asset processors and
/// must receive their complete resource store from BELD.
pub fn setup_default_resource_and_asset_management(
	application: &mut GraphicsApplication,
	generator: impl ProgramGenerator + 'static,
) {
	#[cfg(not(debug_assertions))]
	{
		let _ = (application, generator);

		return;
	}

	#[cfg(debug_assertions)]
	{
		let generator = std::sync::Arc::new(generator);

		let assets_path = super::resolve_application_directory(application.get_parameter("assets-path"), "assets");

		let storage_backend = FileStorageBackend::new(assets_path);

		let mut asset_manager = AssetManager::new_shared(storage_backend, application.resource_manager.storage_backend());

		let material_mip_generator: std::sync::Arc<dyn MipGenerationBackend> =
			MaterialMipGenerator::try_with_default_gpu()
				.map(|generator| std::sync::Arc::new(generator) as std::sync::Arc<dyn MipGenerationBackend>)
				.unwrap_or_else(|error| {
					log::warn!(
						"GPU material mip setup failed; using CPU generation. The most likely cause is that no compatible compute device is available. Error: {error}"
					);
					std::sync::Arc::new(CPUMipGenerationBackend)
				});

		let mut material_asset_handler = BEMAAssetHandler::new();

		material_asset_handler.set_shader_generator(generator.clone());

		asset_manager.add_asset_handler(material_asset_handler);

		let mut fbx_asset_handler = FBXAssetHandler::new();

		fbx_asset_handler.set_shader_generator(generator.clone());

		fbx_asset_handler.set_material_mip_generator(material_mip_generator.clone());

		asset_manager.add_asset_handler(fbx_asset_handler);

		let mut gltf_asset_handler = GLTFAssetHandler::new();

		gltf_asset_handler.set_shader_generator(generator);

		gltf_asset_handler.set_material_mip_generator(material_mip_generator);

		asset_manager.add_asset_handler(gltf_asset_handler);

		register_default_image_asset_handlers(&mut asset_manager);

		asset_manager.add_asset_handler(resource_management::asset::handler::implementations::pipeline::PipelineAssetHandler);

		let ibl_generator = IBLGenerator::try_with_default_gpu().unwrap_or_else(|error| {
			log::warn!(
				"GPU environment-map setup failed; using CPU generation. The most likely cause is that no compatible compute device is available. Error: {error}"
			);
			IBLGenerator::new()
		});

		asset_manager.add_asset_handler(EXRAssetHandler::new(ibl_generator));

		asset_manager.add_asset_handler(LUTAssetHandler::new());

		asset_manager.add_asset_handler(WAVAssetHandler::new());

		asset_manager.add_asset_handler(OGGAssetHandler::new());

		let mut besl_shader_asset_handler = BESLShaderAssetHandler::new();

		besl_shader_asset_handler.set_shader_generator(CommonShaderGenerator::new());

		asset_manager.add_asset_handler(besl_shader_asset_handler);

		application.resource_manager.set_asset_manager(asset_manager);
	}
}

/// Registers source image formats loaded lazily by the default debug application.
#[cfg(debug_assertions)]
fn register_default_image_asset_handlers(asset_manager: &mut AssetManager) {
	asset_manager.add_asset_handler(PNGAssetHandler::new());

	asset_manager.add_asset_handler(IESAssetHandler::new());
}

/// Installs the device classes expected by [`super::process_default_window_input`].
///
/// Next, create application-level actions through
/// [`GraphicsApplication::action_factory`]. The application tick translates
/// window events and emits their resolved action values.
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

/// Starts the audio worker, its byte-bounded global sample pool, and the
/// standard audio entity listeners.
///
/// Next, submit a [`crate::audio::generator::Generator`] through
/// [`GraphicsApplication::generator_factory`] to make it available to the audio
/// worker, or create an [`crate::audio::graph::AudioGraph`] through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory`].
pub fn setup_default_audio(
	application: &mut GraphicsApplication,
	spawn_loading_task: impl FnOnce(Box<dyn FnOnce(&compio::runtime::Runtime) + Send>),
) {
	let mut audio_graphs_listener = application.world.audio_graph_factory().listener();

	let mut deletions_listener = application.world.deletions_listener();

	let (mut sample_loader_client, sample_loader) =
		AudioSampleLoader::new(application.resource_manager.clone(), AudioSamplePoolConfig::default());

	spawn_loading_task(Box::new(move |runtime| {
		runtime.spawn(sample_loader.run()).detach();
	}));

	application
		.threads
		.push(Thread::new(application.application_events.0.listener(), {
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
					if matches!(receiver.read(), Some(Events::Close)) {
						break;
					}

					while let Some(message) = generators_listener.read() {
						audio_system.create_generator(message.into_data());
					}

					while let Some(message) = audio_graphs_listener.read() {
						let handle = message.handle();

						// A derived creation replaces the old generation before
						// any completion can be adopted for the same handle.
						audio_system.remove_audio_graph(handle);

						sample_loader_client.queue(handle, message.into_data(), audio_system.audio_graph_count());
					}

					while let Some(message) = deletions_listener.read() {
						let handle = message.into_handle();

						sample_loader_client.remove(handle);

						audio_system.remove_audio_graph(handle);
					}

					audio_system.flush_sample_lease_releases(|id| sample_loader_client.return_lease(id));

					sample_loader_client.update(|handle, sample, render_plan| {
						audio_system.create_audio_graph(handle, sample, render_plan);
					});

					if !audio_system.render_available() {
						break;
					}
				}

				log::debug!("Exiting audio thread");
			}
		}));
}

/// Creates an [`AnimationPool`] with the given decoded-clip byte budget and
/// spawns its load worker on the application's async runtime.
///
/// Next, call [`AnimationPool::update`] once per tick before advancing graph
/// players that share the pool, then create animation graphs through
/// [`crate::animation::graph::AnimationGraphPlayer`].
pub fn setup_animation_pool(
	application: &mut GraphicsApplication,
	byte_budget: NonZeroUsize,
	spawn_loading_task: impl FnOnce(Box<dyn FnOnce(&compio::runtime::Runtime) + Send>),
) -> AnimationPool {
	// The pool owns pose evaluation state on the application thread while its
	// worker resolves animation resources asynchronously.
	let (pool, worker) = AnimationPool::new(application.resource_manager_handle(), AnimationPoolConfig::new(byte_budget));

	spawn_loading_task(Box::new(move |runtime| {
		runtime.spawn(worker.run()).detach();
	}));

	pool
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

use std::num::{NonZero, NonZeroUsize};

use resource_management::asset::handler::implementations::bema::ProgramGenerator;
#[cfg(debug_assertions)]
use resource_management::asset::{
	FileStorageBackend, handler::implementations::bema::BEMAAssetHandler,
	handler::implementations::besl::BESLShaderAssetHandler, handler::implementations::exr::EXRAssetHandler,
	handler::implementations::fbx::FBXAssetHandler, handler::implementations::gltf::GLTFAssetHandler,
	handler::implementations::ies::IESAssetHandler, handler::implementations::lut::LUTAssetHandler,
	handler::implementations::ogg::OGGAssetHandler, handler::implementations::png::PNGAssetHandler,
	handler::implementations::wav::WAVAssetHandler, manager::AssetManager,
};
#[cfg(debug_assertions)]
use resource_management::{
	ibl::IBLGenerator,
	resources::mips::{CPUMipGenerationBackend, MipGenerationBackend, gpu::MaterialMipGenerator},
};
use tracing::debug_span;
use utils::Extent;

use super::{GraphicsApplication, setup_pbr_visibility_shading_render_pipeline};
#[cfg(debug_assertions)]
use crate::rendering::common_shader_generator::CommonShaderGenerator;
#[cfg(debug_assertions)]
use crate::rendering::pipelines::visibility::shader_generator::VisibilityShaderGenerator;
use crate::{
	animation::graph::{AnimationPool, AnimationPoolConfig},
	application::{Events, application::Application, parameters::Parameters as _, thread::Thread},
	audio::{
		audio_system::{AudioSystem, DefaultAudioSystem},
		sample_loader::{AudioSampleLoader, AudioSamplePoolConfig},
	},
	core::listener::Listener as _,
	input::utils::{register_gamepad_device_class, register_keyboard_device_class, register_mouse_device_class},
	rendering::window::Window,
};

#[cfg(all(test, debug_assertions))]

mod tests {

	use std::sync::atomic::{AtomicUsize, Ordering};

	use resource_management::{
		asset::{FileStorageBackend, manager::AssetManager},
		resource::storage_backend::redb::ReDBStorageBackend,
	};

	use super::register_default_image_asset_handlers;

	#[test]
	fn default_image_handlers_support_ies_profiles() {
		static NEXT_TEST_ID: AtomicUsize = AtomicUsize::new(0);

		let root = std::env::temp_dir().join(format!(
			"byte-engine-default-image-handlers-{}-{}",
			std::process::id(),
			NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
		));

		let assets = root.join("assets");

		let resources = root.join("resources");

		let mut asset_manager = AssetManager::new(FileStorageBackend::new(assets), ReDBStorageBackend::new(resources));

		register_default_image_asset_handlers(&mut asset_manager);

		assert!(asset_manager.supports("lights/profile.ies"));

		drop(asset_manager);

		std::fs::remove_dir_all(root).expect("the default image-handler test directory must be removable");
	}
}
