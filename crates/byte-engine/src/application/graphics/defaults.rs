//! Conventional setup components for [`GraphicsApplication`].
//!
//! [`default_setup`] is the batteries-included path used by the `triangle`
//! example. Applications that replace a subsystem can call the remaining setup
//! functions individually; the `window` example demonstrates that narrower
//! composition.

/// Installs the standard assets, input devices, audio worker, visibility
/// rendering pipeline, and window.
///
/// After setup, create application actions and select scene lighting through
/// [`crate::gameplay::world::DefaultWorld`], then run the application with
/// [`GraphicsApplication::do_loop`].
pub fn default_setup(application: &mut GraphicsApplication) {
	#[cfg(debug_assertions)]
	{
		let generator = VisibilityShaderGenerator::with_access(ScopeAccess {
			material_count: ghi::AccessPolicies::READ,
			material_offset: ghi::AccessPolicies::NONE,
			material_offset_scratch: ghi::AccessPolicies::NONE,
			pixel_mapping: ghi::AccessPolicies::READ_WRITE,
		});

		setup_default_resource_and_asset_management(application, generator);
	}

	setup_default_input(application);

	setup_default_audio(application);

	setup_pbr_visibility_shading_render_pipeline(application);

	setup_particles(application);

	setup_default_window(application);

	launch_deferred_tasks_thread(application);
}

/// Runs the application's deferred loading tasks and its [`Loader`](crate::rendering::loading::Loader) on one
/// application-owned loading thread.
///
/// Launch the thread after every subsystem has registered its loading work with
/// [`GraphicsApplication::add_deferred_task`]. Tasks run in registration order,
/// after the loader starts. Pipelines set up afterwards cannot add loader lanes.
pub fn launch_deferred_tasks_thread(application: &mut GraphicsApplication) {
	let (loader, tasks) = application.take_loading_work();
	application
		.threads
		.push(Thread::new(application.application_events.0.listener(), move |mut events| {
			let runtime = build_single_threaded_async_runtime();

			// Compio separates task execution from I/O polling. Enter the runtime so
			// resource futures can access it, then drive both halves until shutdown.
			runtime.enter(|| {
				if let Some(loader) = loader {
					loader.run(&runtime);
				}
				for task in tasks {
					task(&runtime);
				}
				drive_runtime(&runtime, || matches!(events.read(), Some(Events::Close)));
			});
		}));
}

/// Runs the tasks of `runtime` until `stop` returns `true`, on the thread that entered the runtime.
///
/// The loop wakes for ready tasks, finished I/O, and the earliest timer, so timed tasks such as the loader's
/// completion checks run on time. While nothing is ready, it still checks `stop` at least every 6 ms.
pub fn drive_runtime(runtime: &compio::runtime::Runtime, mut stop: impl FnMut() -> bool) {
	const MAX_IDLE: std::time::Duration = std::time::Duration::from_millis(6);

	while !stop() {
		let timeout = if runtime.run() {
			std::time::Duration::ZERO
		} else {
			runtime.current_timeout().map_or(MAX_IDLE, |timer| timer.min(MAX_IDLE))
		};
		runtime.poll_with(Some(timeout));
	}
}

/// Creates the single-threaded runtime used by default background workers.
pub fn build_single_threaded_async_runtime() -> compio::runtime::Runtime {
	compio::runtime::Runtime::new().unwrap()
}

/// A loading operation that spawns its work on the provided runtime.
pub(crate) type DeferredTask = Box<dyn FnOnce(&compio::runtime::Runtime) + Send>;

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
	generator: impl ProgramGenerator + Clone + 'static,
) {
	#[cfg(not(debug_assertions))]
	{
		let _ = (application, generator);

		return;
	}

	#[cfg(debug_assertions)]
	{
		let assets_path = super::resolve_application_directory(application.get_parameter("assets-path"), "assets");

		let storage_backend = FileStorageBackend::new(assets_path);

		let mut asset_manager = AssetManager::new_shared(storage_backend, application.resource_manager.storage_backend());

		let (material_mips, ibl) = default_offline_generators();

		register_default_asset_handlers(&mut asset_manager, generator, material_mips, ibl);

		application.resource_manager.set_asset_manager(asset_manager);
	}
}

/// Registers the standard material, model, image, audio, and standalone-shader handlers on `asset_manager`.
///
/// The debug runtime and BELD both call this, so a baked store and a debug run produce the same resources from the same
/// assets. `generator` adapts generated material shaders to the renderer, and `material_mips` and `ibl` select the
/// offline texture backends; [`default_offline_generators`] returns the usual ones.
pub fn register_default_asset_handlers(
	asset_manager: &mut AssetManager,
	generator: impl ProgramGenerator + Clone + 'static,
	material_mips: Arc<MipGenerator>,
	ibl: IBLGenerator,
) {
	let mut material_asset_handler = BEMAAssetHandler::new();
	material_asset_handler.set_shader_generator(generator.clone());
	asset_manager.add_asset_handler(material_asset_handler);

	let mut fbx_asset_handler = FBXAssetHandler::new();
	fbx_asset_handler.set_shader_generator(generator.clone());
	fbx_asset_handler.set_material_mip_generator(material_mips.clone());
	asset_manager.add_asset_handler(fbx_asset_handler);

	let mut gltf_asset_handler = GLTFAssetHandler::new();
	gltf_asset_handler.set_shader_generator(generator);
	gltf_asset_handler.set_material_mip_generator(material_mips);
	asset_manager.add_asset_handler(gltf_asset_handler);

	// PNG and EXR both handle `Image` sources and the asset manager picks the first match, so PNG must come first.
	asset_manager.add_asset_handler(PNGAssetHandler::new());
	asset_manager.add_asset_handler(IESAssetHandler::new());
	asset_manager.add_asset_handler(PipelineAssetHandler);
	asset_manager.add_asset_handler(FlipbookAssetHandler);
	asset_manager.add_asset_handler(ParticleSystemAssetHandler::new());
	asset_manager.add_asset_handler(EXRAssetHandler::new());
	asset_manager.add_asset_handler(EnvironmentMapAssetHandler::new(ibl));
	asset_manager.add_asset_handler(LUTAssetHandler::new());
	asset_manager.add_asset_handler(WAVAssetHandler::new());
	asset_manager.add_asset_handler(OGGAssetHandler::new());

	let mut besl_shader_asset_handler = BESLShaderAssetHandler::new();
	besl_shader_asset_handler.set_shader_generator(CommonShaderGenerator::new());
	asset_manager.add_asset_handler(besl_shader_asset_handler);
}

/// Returns the GPU material mip and environment-map generators, falling back to CPU generation when GPU setup fails.
///
/// Pass the result to [`register_default_asset_handlers`].
pub fn default_offline_generators() -> (Arc<MipGenerator>, IBLGenerator) {
	let material_mips = MaterialMipGenerator::try_with_default_gpu()
		.map(|generator| Arc::new(MipGenerator::Gpu(generator)))
		.unwrap_or_else(|error| {
			log::warn!(
				"GPU material mip setup failed; using CPU generation. The most likely cause is that no compatible compute device is available. Error: {error}"
			);
			Arc::new(MipGenerator::Cpu)
		});

	let ibl = IBLGenerator::try_with_default_gpu().unwrap_or_else(|error| {
		log::warn!(
			"GPU environment-map setup failed; using CPU generation. The most likely cause is that no compatible compute device is available. Error: {error}"
		);
		IBLGenerator::new()
	});

	(material_mips, ibl)
}

/// Installs the device classes expected by [`super::process_default_window_input`].
///
/// Next, create application-level actions through [`GraphicsApplication::world`].
/// The application tick translates window events and emits their resolved action
/// values.
pub fn setup_default_input(application: &mut GraphicsApplication) {
	let input = &mut application.input;
	let mouse = register_mouse_device_class(input);
	let keyboard = register_keyboard_device_class(input);
	let gamepad = register_gamepad_device_class(input);
	application.gamepad_device_class_handle = Some(gamepad);
	input.create_device(&mouse);
	input.create_device(&keyboard);
	input.create_device(&gamepad);
}

/// Starts the audio worker, its byte-bounded global sample pool, and the
/// standard audio entity listeners.
///
/// Next, submit a [`crate::audio::generator::Generator`] through
/// [`GraphicsApplication::generator_factory`] to make it available to the audio
/// worker, or create an [`crate::audio::graph::AudioGraph`] through
/// [`crate::gameplay::world::DefaultWorld::audio_graph_factory`].
pub fn setup_default_audio(application: &mut GraphicsApplication) {
	let mut audio_graphs_listener = application.world.audio_graph_factory().listener();

	let mut deletions_listener = application.world.deletions_listener();

	let (mut sample_loader_client, sample_loader) =
		AudioSampleLoader::new(application.resource_manager.clone(), AudioSamplePoolConfig::default());

	application.add_deferred_task(move |runtime| {
		runtime.spawn(sample_loader.run()).detach();
	});

	application
		.threads
		.push(Thread::new(application.application_events.0.listener(), {
			let mut generators_listener = application.generator_factory.listener();

			move |mut receiver| {
				let mut audio_system = match DefaultAudioSystem::try_new() {
					Ok(audio_system) => audio_system,
					Err(error) => {
						log::warn!("Failed to spawn audio system. No audio will play. Reason: {error}");
						return;
					}
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
/// queues its load worker on the application's loading thread.
///
/// Next, call [`AnimationPool::update`] once per tick before advancing graph
/// players that share the pool, then create animation graphs through
/// [`crate::animation::graph::AnimationGraphPlayer`].
pub fn setup_animation_pool(application: &mut GraphicsApplication, byte_budget: NonZeroUsize) -> AnimationPool {
	// The pool owns pose evaluation state on the application thread while its
	// worker resolves animation resources asynchronously.
	let (pool, worker) = AnimationPool::new(application.resource_manager_handle(), byte_budget);

	application.add_deferred_task(move |runtime| {
		runtime.spawn(worker.run()).detach();
	});

	pool
}

use std::{num::NonZeroUsize, sync::Arc};

#[cfg(debug_assertions)]
use resource_management::asset::FileStorageBackend;
use resource_management::{
	asset::{
		handler::implementations::{
			bema::{BEMAAssetHandler, ProgramGenerator},
			besl::BESLShaderAssetHandler,
			environment::EnvironmentMapAssetHandler,
			exr::EXRAssetHandler,
			fbx::FBXAssetHandler,
			flipbook::FlipbookAssetHandler,
			gltf::GLTFAssetHandler,
			ies::IESAssetHandler,
			lut::LUTAssetHandler,
			ogg::OGGAssetHandler,
			particles::ParticleSystemAssetHandler,
			pipeline::PipelineAssetHandler,
			png::PNGAssetHandler,
			wav::WAVAssetHandler,
		},
		manager::AssetManager,
	},
	ibl::IBLGenerator,
	resources::mips::{MipGenerator, gpu::MaterialMipGenerator},
};
use tracing::debug_span;
use utils::Extent;

use super::{GraphicsApplication, setup_particles, setup_pbr_visibility_shading_render_pipeline};
use crate::rendering::common_shader_generator::CommonShaderGenerator;
#[cfg(debug_assertions)]
use crate::rendering::pipelines::visibility::{ScopeAccess, VisibilityShaderGenerator};
use crate::{
	animation::graph::AnimationPool,
	application::{Events, parameters::Parameters as _, thread::Thread},
	audio::{
		audio_system::DefaultAudioSystem,
		sample_loader::{AudioSampleLoader, AudioSamplePoolConfig},
	},
	core::listener::Listener as _,
	input::utils::{register_gamepad_device_class, register_keyboard_device_class, register_mouse_device_class},
	rendering::window::Window,
};
