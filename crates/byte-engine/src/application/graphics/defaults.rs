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
/// [`GraphicsApplication::world`], then run the application with
/// [`GraphicsApplication::do_loop`].
pub fn default_setup(application: &mut GraphicsApplication) {
	#[cfg(debug_assertions)]
	{
		let generator = VisibilityShaderGenerator::new(application.parameters().scope(RENDER_PARAMETERS));

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
		.push(Thread::new(application.application.events().listener(), move |mut events| {
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
#[cfg_attr(not(debug_assertions), allow(unused_variables))]
pub fn setup_default_resource_and_asset_management(
	application: &mut GraphicsApplication,
	generator: impl ProgramGenerator + Clone + 'static,
) {
	#[cfg(debug_assertions)]
	{
		use crate::application::application::resolve_application_path;

		let assets_path = resolve_application_path(application.get_parameter("assets-path"), "assets");

		let storage_backend = FileStorageBackend::new(assets_path);

		let mut asset_manager = AssetManager::new_shared(storage_backend, application.resource_manager.storage_backend());

		asset_manager.set_image_processor(default_image_processor());

		let features = VisibilityFeatures::from_parameters(application.parameters().scope(RENDER_PARAMETERS));
		register_default_asset_handlers(&mut asset_manager, generator, features);

		application.resource_manager.set_asset_manager(asset_manager);
	}
}

/// Registers the standard material, model, image, audio, and standalone-shader handlers on `asset_manager`.
///
/// The debug runtime and BELD both call this, so a baked store and a debug run produce the same resources from the same
/// assets. `generator` adapts generated material shaders to the renderer, and `features` removes left-out features from
/// standalone shaders. Every handler that produces images bakes them through the manager's image processor; install
/// [`default_image_processor`] with [`AssetManager::set_image_processor`] to use the GPU backends.
pub fn register_default_asset_handlers(
	asset_manager: &mut AssetManager,
	generator: impl ProgramGenerator + Clone + 'static,
	features: VisibilityFeatures,
) {
	let mut material_asset_handler = BEMAAssetHandler::new();
	material_asset_handler.set_shader_generator(generator.clone());
	asset_manager.add_asset_handler(material_asset_handler);

	let mut fbx_asset_handler = FBXAssetHandler::new();
	fbx_asset_handler.set_shader_generator(generator.clone());
	asset_manager.add_asset_handler(fbx_asset_handler);

	let mut gltf_asset_handler = GLTFAssetHandler::new();
	gltf_asset_handler.set_shader_generator(generator);
	asset_manager.add_asset_handler(gltf_asset_handler);

	// PNG and EXR both handle `Image` sources and the asset manager picks the first match, so PNG must come first.
	asset_manager.add_asset_handler(PNGAssetHandler::new());
	asset_manager.add_asset_handler(IESAssetHandler::new());
	asset_manager.add_asset_handler(PipelineAssetHandler);
	asset_manager.add_asset_handler(FlipbookAssetHandler);
	asset_manager.add_asset_handler(ParticleSystemAssetHandler::new());
	asset_manager.add_asset_handler(EXRAssetHandler::new());
	asset_manager.add_asset_handler(EnvironmentMapAssetHandler::new());
	asset_manager.add_asset_handler(LUTAssetHandler::new());
	asset_manager.add_asset_handler(WAVAssetHandler::new());
	asset_manager.add_asset_handler(OGGAssetHandler::new());

	let mut besl_shader_asset_handler = BESLShaderAssetHandler::new();
	besl_shader_asset_handler.set_program_transform(CommonShaderGenerator::with_features(features));
	asset_manager.add_asset_handler(besl_shader_asset_handler);
}

/// Returns an image processor with GPU mip and environment-map generators, falling back to CPU generation for each one
/// whose GPU setup fails.
///
/// Install it with [`AssetManager::set_image_processor`].
pub fn default_image_processor() -> ImageProcessor {
	let mips = MaterialMipGenerator::try_with_default_gpu()
		.map(MipGenerator::Gpu)
		.unwrap_or_else(|error| {
			log::warn!(
				"GPU texture mip setup failed; using CPU generation. The most likely cause is that no compatible compute device is available. Error: {error}"
			);
			MipGenerator::Cpu
		});

	let ibl = IBLGenerator::try_with_default_gpu().unwrap_or_else(|error| {
		log::warn!(
			"GPU environment-map setup failed; using CPU generation. The most likely cause is that no compatible compute device is available. Error: {error}"
		);
		IBLGenerator::new()
	});

	ImageProcessor::new(mips, ibl)
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
/// [`crate::audio::graph::AudioGraphFactory`].
pub fn setup_default_audio(application: &mut GraphicsApplication) {
	let mut audio_graphs_listener = AudioGraphFactory::in_scope(&application.world).listener();

	let mut deletions_listener = application.world.channel::<DeleteMessage>().listener();

	let (mut sample_loader_client, sample_loader) =
		AudioSampleLoader::new(application.resource_manager.clone(), AudioSamplePoolConfig::default());

	application.add_deferred_task(move |runtime| {
		runtime.spawn(sample_loader.run()).detach();
	});

	application
		.threads
		.push(Thread::new(application.application.events().listener(), {
			let mut generators_listener = application.generator_factory.listener();

			move |mut receiver| {
				let mut audio_system = match DefaultAudioSystem::try_new() {
					Ok(audio_system) => audio_system,
					Err(error) => {
						log::warn!("Failed to spawn audio system. No audio will play. Reason: {error}");
						return;
					}
				};

				let _span = debug_span!("Render audio").entered();

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

/// Creates the world's animation system from the application's `animation` parameters.
///
/// Call before starting the loading thread and creating animators. Next, register graphs with
/// [`AnimationSystem::register`](crate::animation::system::AnimationSystem::register) and step the system each frame.
pub fn setup_animation_system(application: &mut GraphicsApplication) -> crate::animation::system::AnimationSystem {
	use crate::application::parameters::Parameters as _;

	let (system, worker) = crate::animation::system::AnimationSystem::new(
		application.world(),
		application.resource_manager_handle(),
		application.parameters().scope("animation"),
	);

	application.add_deferred_task(move |runtime| {
		runtime.spawn(worker.run()).detach();
	});

	system
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

use std::num::NonZeroUsize;

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
	processors::image::ImageProcessor,
	resources::mips::{MipGenerator, gpu::MaterialMipGenerator},
};
use tracing::debug_span;
use utils::Extent;

use super::{GraphicsApplication, setup_particles, setup_pbr_visibility_shading_render_pipeline};
use crate::rendering::common_shader_generator::CommonShaderGenerator;
#[cfg(debug_assertions)]
use crate::rendering::pipelines::visibility::VisibilityShaderGenerator;
use crate::rendering::pipelines::visibility::{RENDER_PARAMETERS, VisibilityFeatures};
use crate::{
	animation::graph::AnimationPool,
	application::{Events, parameters::Parameters as _, thread::Thread},
	audio::graph::AudioGraphFactory,
	audio::{
		audio_system::DefaultAudioSystem,
		sample_loader::{AudioSampleLoader, AudioSamplePoolConfig},
	},
	core::{listener::Listener as _, message::DeleteMessage},
	input::utils::{register_gamepad_device_class, register_keyboard_device_class, register_mouse_device_class},
	rendering::window::Window,
};
