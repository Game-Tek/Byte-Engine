//! Headed application runtime and graphics setup entry points.
//!
//! Construct [`GraphicsApplication`], configure it with either [`default_setup`]
//! or selected setup functions, then run its loop. The `triangle` example uses
//! the complete default stack; the `window` example creates only a window.
//!
//! Rendering setup remains in this module because it coordinates the world,
//! renderer, and application factories. General startup defaults and external
//! adapters are kept behind the setup functions re-exported from this module.
//!
//! Follow the [sample project guide](https://byte-engine.0x44491229.dev/docs/use/sample-project)
//! for the complete application setup sequence.

// Bound ready work while the temporary Compio runtime still shares the application thread.
const ASYNC_TASK_POLL_BUDGET_PER_TICK: usize = 8;

/// The [`GraphicsApplication`] struct owns the headed runtime and coordinates
/// windows, input, worlds, resources, audio workers, and rendering.
///
/// Use [`default_setup`] for the conventional engine stack. Use
/// [`setup_default_window`], [`setup_default_input`], and the render-pass setup
/// functions independently when an application needs explicit composition.
/// After setup, call [`Self::do_loop`] to run the application, or
/// [`Self::tick_with`] when application code must run during each tick.
///
/// # Configuration
/// - `kill-after`: Closes the application after this number of ticks. The default is `None`.
/// - `assets-path`: Selects the debug-build asset directory. Relative overrides use the current working directory. In development, the default is `assets` under `CARGO_MANIFEST_DIR` when available, then beside the executable.
/// - `resources.path`: Selects the resource directory. Relative overrides use the current working directory. In development, the default uses `CARGO_MANIFEST_DIR` when available; otherwise, it is beside the executable.
/// - `render.debug`: Enables validation layers. The default is `true` in debug builds.
/// - `render.debug.dump`: Enables graphics API logging. The default is `false`.
/// - `render.debug.extended`: Enables extended validation. The default is `false`.
/// - `messages.max-topics`: Sets the maximum number of typed routes. The default is `64`.
/// - `messages.cells-per-topic`: Sets the fixed payload-cell budget for each typed route. The default is `512`.
/// - `messages.cell-bytes`: Sets the size of each payload cell in bytes. The default is `256`.
/// - `messages.cell-alignment`: Sets the alignment of each payload cell. The default is `64`.
/// - `messages.listeners-per-topic`: Sets the maximum simultaneous listeners on one typed route. The default is `64`.
/// - `render.pass.<name>`: Selects `enabled` or `bypassed` for the named render pass.
/// - `render.gtao.radius`: Sets the GTAO world-space search radius. The default is `1.0`.
/// - `render.gtao.samples-per-ray`: Sets the GTAO samples along each ray. The default is `6`.
/// - `render.gtao.radial-rays`: Sets the even number of GTAO ray directions. The default is `8`.
/// - `render.cone-shadow-map-pool.capacity`: Sets the startup maximum for reusable cone-light shadow maps per sink. Maps allocate on first use; the default capacity is `4`.
/// - `render.point-shadow-map-pool.capacity`: Sets the startup maximum for reusable point-light cube shadow maps per sink. Maps allocate on first use; the default capacity is `4`.
///
/// See the [sample project guide](https://byte-engine.0x44491229.dev/docs/use/sample-project)
/// for a complete `GraphicsApplication` setup.
pub struct GraphicsApplication {
	application: BaseApplication,
	message_bus: MessageBus,
	messages: MessageScope,

	tick_count: u64,
	start_time: std::time::Instant,
	last_tick_time: MediaTime,

	close: bool,

	application_events: (DefaultChannel<Events>, DefaultListener<Events>),
	http_inspector: HttpInspectorServer,
	screenshot_broker: std::sync::Arc<crate::inspector::screenshot::ScreenshotBroker>,
	configuration: Configuration,

	window_factory: (Factory<Window>, DefaultListener<CreateMessage<Window>>),
	action_factory: Factory<Action>,

	generator_factory: Factory<Arc<dyn Generator>>,

	world: DefaultWorld,
	cameras_listener: DefaultListener<crate::core::factory::CreateMessage<Camera>>,
	physics_transforms_listener: DefaultListener<TransformationUpdate>,
	renderer_transforms_listener: DefaultListener<TransformationUpdate>,

	input_system: input::InputManager,
	gamepad_system: Option<input::gamepad::GamepadSystem>,
	gamepad_device_class_handle: Option<input::device_class::DeviceClassHandle>,
	resource_manager: EntityHandle<ResourceManager>,
	renderer: Renderer,

	threads: SmallVec<[Thread; 64]>,

	#[cfg(debug_assertions)]
	ttff: MediaTime,
	#[cfg(debug_assertions)]
	min_frame_time: MediaTime,
	#[cfg(debug_assertions)]
	max_frame_time: MediaTime,

	#[cfg(debug_assertions)]
	kill_after: Option<u64>,
}

impl Drop for GraphicsApplication {
	fn drop(&mut self) {
		// Workers may own transferred views into renderer allocations. Join them
		// before field destruction reaches the renderer and its GHI context.
		self.stop_worker_threads();
	}
}

impl Application for GraphicsApplication {
	fn new(name: &str, parameters: &[Parameter]) -> Self {
		let start_time = std::time::Instant::now();

		let application = BaseApplication::new(name, parameters);
		let (message_bus, messages, world_messages) = create_message_bus(&application);
		message_bus.observe().unwrap_or_else(|error| panic!("{error}"));

		let resources_path = resolve_application_directory(application.get_parameter("resources.path"), "resources");

		// Opening an application store first removes resources baked by an incompatible engine revision.
		let resource_storage = ReDBStorageBackend::new_writable_with_settings(
			resources_path,
			ResourceStorageSettings::new(ResourceStorageMode::Files)
				.image_compression(ResourceGpuCompressionPolicy::MetalIoLz4),
		)
		.unwrap(); // TODO: revise this

		let resource_manager = EntityHandle::from(ResourceManager::new(resource_storage));

		let action_factory = messages.factory();

		let input_system = {
			let action_listener = action_factory.listener();
			let event_channel = messages.channel();

			input::InputManager::new(action_listener, event_channel)
		};
		// HID initialization and first enumeration can block startup on Windows, so gamepads are initialized after
		// the first frame has reached the screen.
		let gamepad_system = None;

		let configuration = Configuration::new();
		let mut renderer = rendering::renderer::Renderer::new(&application, &configuration);
		renderer.set_resource_manager(&resource_manager);
		queue_render_pass_startup_parameters(application.parameters(), &configuration);

		#[cfg(debug_assertions)]
		let kill_after = application
			.get_parameter("kill-after")
			.map(|p| p.value.parse::<u64>().unwrap());

		// Register the application listener before control handlers can publish a
		// close request. Worker listeners join this future-only route during setup.
		let application_events: (DefaultChannel<Events>, DefaultListener<Events>) = {
			let channel = messages.channel();
			let listener = channel.listener();
			(channel, listener)
		};

		ctrlc::set_handler({
			let events = application_events.0.clone();
			move || {
				events.send(Events::Close);
			}
		})
		.unwrap();

		let world = DefaultWorld::with_messages(world_messages);
		let cameras_listener = world.factory::<Camera>().listener();
		let physics_transforms_listener = world.transforms_channel().listener();
		let renderer_transforms_listener = world.transforms_channel().listener();

		let inspector = EntityHandle::from(Inspector::new(
			application_events.0.clone(),
			configuration.clone(),
			world.messages().clone(),
		));
		let screenshot_broker = inspector.screenshots();
		let http_inspector = HttpInspectorServer::new(inspector);

		let window_factory = messages.factory();
		let window_factory_listener = window_factory.listener();

		let generator_factory = messages.factory();

		GraphicsApplication {
			application,
			message_bus,
			messages,

			application_events,
			http_inspector,
			screenshot_broker,
			configuration,

			window_factory: (window_factory, window_factory_listener),
			action_factory,

			generator_factory,

			world,
			cameras_listener,
			physics_transforms_listener,
			renderer_transforms_listener,

			input_system,
			gamepad_system,
			gamepad_device_class_handle: None,
			resource_manager,
			renderer,

			threads: SmallVec::new(),

			close: false,

			tick_count: 0,
			start_time,
			last_tick_time: MediaTime::from_std(start_time.elapsed()),

			#[cfg(debug_assertions)]
			ttff: MediaTime::ZERO,
			#[cfg(debug_assertions)]
			min_frame_time: MediaTime::MAX,
			#[cfg(debug_assertions)]
			max_frame_time: MediaTime::ZERO,

			#[cfg(debug_assertions)]
			kill_after,
		}
	}

	fn get_name(&self) -> &str {
		self.application.get_name()
	}

	fn tick(&mut self) -> bool {
		self.tick_with(|_, _| {}).is_some()
	}
}

impl GraphicsApplication {
	/// Returns frame-local storage for temporary allocations during the current tick.
	pub fn frame_allocator(&self) -> &bumpalo::Bump {
		&self.application.frame_allocator
	}

	/// Returns the configuration exchange used to inspect startup update results.
	pub fn configuration(&self) -> &Configuration {
		&self.configuration
	}

	/// Runs one graphics tick and lets application code update state before rendering.
	pub fn tick_with<R, F: FnOnce(&mut Self, Time) -> R>(&mut self, f: F) -> Option<R> {
		let span = debug_span!("GraphicsApplication::tick");
		let _enter = span.enter();

		let now = std::time::Instant::now();
		// Sample the monotonic clock once, then keep application time entirely in
		// media ticks so elapsed time is exactly the sum of observed frame deltas.
		let elapsed = MediaTime::from_std(now.duration_since(self.start_time));
		let dt = elapsed - self.last_tick_time;
		self.last_tick_time = elapsed;
		let tick_count = self.tick_count;

		let mut close = false;

		{
			let span = debug_span!("GraphicsApplication::reset_frame_allocator");
			let _enter = span.enter();
			self.application.frame_allocator.reset();
		}

		{
			let span = debug_span!("GraphicsApplication::process_window_events");
			let _enter = span.enter();
			let renderer = &mut self.renderer;
			let input_system = &mut self.input_system;

			for window_events in renderer.update_windows() {
				for event in window_events {
					if let ghi::window::Events::Close = event {
						close = true;
					}

					if let Some((seat_handle, device_handle, input_source_action, value)) =
						process_default_window_input(input_system, event)
					{
						input_system.record_trigger_value_for_device(seat_handle, device_handle, input_source_action, value);
					}
				}
			}
		}

		{
			let span = debug_span!("GraphicsApplication::process_application_events");
			let _enter = span.enter();
			if let Some(e) = self.application_events.1.read() {
				match e {
					Events::Close => {
						close = true;
					}
				}
			}
		}

		{
			let span = debug_span!("GraphicsApplication::process_gamepad_events");
			let _enter = span.enter();
			if self.tick_count > 0 && self.gamepad_system.is_none() {
				self.gamepad_system = input::gamepad::GamepadSystem::new()
					.map_err(|error| log::warn!("{}", error))
					.ok();
			}
			if self.tick_count > 0 {
				if let Some(gamepad_system) = &mut self.gamepad_system {
					let (new_devices, events) = gamepad_system.poll();

					if let Some(gamepad_device_class_handle) = self.gamepad_device_class_handle {
						for (path, kind, device) in new_devices {
							// Each physical HID device gets its own input-system device so actions can
							// preserve player/device identity instead of collapsing into one gamepad.
							let device_handle = self.input_system.create_device(&gamepad_device_class_handle);
							gamepad_system.add_device(path, kind, device, device_handle);
						}
					} else if !new_devices.is_empty() {
						log::warn!(
							"Detected HID gamepad before the Gamepad device class was registered. The most likely cause is that setup_default_input was not called. See {}.",
							crate::online_docs_url("develop/design/input-handling")
						);
					}

					for event in events {
						log::debug!(
							target: "byte_engine::input::events",
							"Forwarding HID gamepad event: device={:?}, trigger={:?}, value={:?}",
							event.device_handle(),
							event.trigger(),
							event.value()
						);
						self.input_system.record_trigger_value_for_device(
							input::SeatHandle::stub(),
							event.device_handle(),
							event.trigger(),
							event.value(),
						);
					}
				}
			}
		}

		let time = Time { elapsed, delta: dt };

		{
			let span = debug_span!("GraphicsApplication::update_input");
			let _enter = span.enter();
			self.input_system.update(&self.application.frame_allocator);
		}

		// Physics publishes its results back to the shared transform route. Discard
		// those prior-frame outputs before collecting commands from this user tick.
		while self.physics_transforms_listener.read().is_some() {}

		let result = {
			let span = debug_span!("GraphicsApplication::user_tick");
			let _enter = span.enter();
			f(self, time)
		};

		{
			let span = debug_span!("GraphicsApplication::update_world");
			let _enter = span.enter();
			self.world.update(
				time,
				&mut self.physics_transforms_listener,
				&mut self.application.frame_allocator,
			);
		}

		{
			let span = debug_span!("GraphicsApplication::prepare_renderer_state");
			let _enter = span.enter();
			let window_listener = &mut self.window_factory.1;

			while let Some(message) = window_listener.read() {
				self.renderer.create_window(message.into_data());
			}

			while let Some(message) = self.cameras_listener.read() {
				self.renderer.create_camera(message.handle(), message.into_data());
			}
		}

		{
			let span = debug_span!("GraphicsApplication::render_frame");
			let _enter = span.enter();
			let requests = self.screenshot_broker.drain();
			let captures = requests
				.iter()
				.map(|request| (request.sink, &request.capture))
				.collect::<Vec<_>>();
			let frame_allocator = &self.application.frame_allocator;
			let results = self
				.renderer
				.prepare(&mut self.renderer_transforms_listener, frame_allocator, &captures);
			for (request, capture) in requests.into_iter().zip(results) {
				let result =
					capture
						.map_err(crate::inspector::screenshot::ScreenshotError::from)
						.and_then(|(frame, readback)| {
							crate::inspector::screenshot::encode_screenshot_png(readback)
								.map(|png| crate::inspector::screenshot::Screenshot { frame, png })
								.map_err(crate::inspector::screenshot::ScreenshotError::Internal)
						});
				request.complete(result);
			}
		}

		{
			let span = debug_span!("GraphicsApplication::flush_world_deletions");
			let _enter = span.enter();
			self.world.flush_deletions();
		}

		self.tick_count += 1;

		#[cfg(debug_assertions)]
		{
			if self.tick_count == 1 {
				self.ttff = MediaTime::from_std(self.start_time.elapsed());
			}

			if let Some(kill_after) = self.kill_after {
				if self.tick_count >= kill_after {
					close = true;
				}
			}

			{
				self.min_frame_time = self.min_frame_time.min(dt);
				self.max_frame_time = self.max_frame_time.max(dt);
			}
		}

		if close {
			self.close();
			None
		} else {
			Some(result)
		}
	}

	/// Drains application-side lifecycle events, then signals and joins every
	/// application worker.
	fn stop_worker_threads(&mut self) {
		if self.threads.is_empty() {
			return;
		}

		while self.application_events.1.read().is_some() {}
		self.application_events.0.send(Events::Close);
		self.threads.drain(..).for_each(|thread| {
			let _ = thread.join();
		});
	}

	/// Flags the application for closing.
	pub fn close(&mut self) {
		self.close = true;

		self.stop_worker_threads();

		#[cfg(debug_assertions)]
		log::debug!(
			"Run stats:\n\tElapsed time: {:#?}\n\tAverage frame time: {:#?}\n\tMin frame time: {:#?}\n\tMax frame time: {:#?}\n\tTime to first frame: {:#?}",
			MediaTime::from_std(self.start_time.elapsed()),
			MediaTime::from_std(self.start_time.elapsed()) / self.tick_count as i64,
			self.min_frame_time,
			self.max_frame_time,
			self.ttff
		);
	}

	/// Returns the input manager that owns devices, triggers, and action state.
	pub fn input_system(&self) -> &input::InputManager {
		&self.input_system
	}

	/// Returns the renderer used by setup functions and advanced render integrations.
	pub fn renderer(&self) -> &Renderer {
		&self.renderer
	}

	/// Returns mutable renderer access for application-defined pipeline setup.
	///
	/// Use this during startup to create renderer-owned GHI resources. For an
	/// asynchronous resource integration, create the mapped transfer buffer and
	/// [`crate::rendering::resource_loading::UploadStagingArena`] here, keep the
	/// buffer handle in the pipeline manager's upload store, and run the staging
	/// worker and resource servers through application-owned tasks. Then call
	/// [`Renderer::add_pipeline_manager`] before the application starts rendering.
	///
	/// The opposite lifetime is equally important: stop and join those tasks
	/// before this application drops the renderer. The built-in Simple and
	/// Visibility setup functions demonstrate that shutdown ordering.
	pub fn renderer_mut(&mut self) -> &mut Renderer {
		&mut self.renderer
	}

	/// Returns the factory used to request new windows.
	pub fn window_factory(&self) -> &Factory<Window> {
		&self.window_factory.0
	}

	/// Returns the factory used to register input actions.
	pub fn action_factory(&self) -> &Factory<Action> {
		&self.action_factory
	}

	/// Returns the application-owned namespace used by headed-runtime channels and factories.
	///
	/// Next, call [`MessageScope::channel`] or [`MessageScope::factory`] to add an
	/// application-defined message route without declaring its type at startup.
	pub fn messages(&self) -> &MessageScope {
		&self.messages
	}

	/// Returns the shared fixed-storage message bus for diagnostics and new scopes.
	///
	/// Next, call [`MessageBus::topics`] to inspect registered routes or
	/// [`MessageBus::new_scope`] to isolate routes owned by another subsystem.
	pub fn message_bus(&self) -> &MessageBus {
		&self.message_bus
	}

	/// Returns the default world updated by the graphics application loop.
	pub fn world(&self) -> &DefaultWorld {
		&self.world
	}

	/// Returns mutable access to the default world updated by the graphics application loop.
	pub fn world_mut(&mut self) -> &mut DefaultWorld {
		&mut self.world
	}

	/// Returns the audio generator factory used by default audio setup.
	pub fn generator_factory(&self) -> &Factory<Arc<dyn Generator>> {
		&self.generator_factory
	}

	/// Runs ticks until the application is closed.
	pub fn do_loop(&mut self) {
		while !self.close {
			self.tick();
		}
	}

	/// Runs ticks with an application callback until the application is closed.
	pub fn do_loop_with<F: FnOnce(&mut Self, Time) + Copy>(&mut self, f: F) {
		while !self.close {
			self.tick_with(f);
		}
	}

	/// Returns the resource manager shared by rendering and asset setup.
	pub fn resource_manager(&self) -> &ResourceManager {
		&self.resource_manager
	}

	/// Returns shared ownership of the resource manager for application-owned async systems.
	///
	/// Use this handle when constructing loaders such as
	/// [`crate::animation::graph::AnimationPool`]. Next, spawn the returned
	/// worker on the application's chosen async runtime.
	pub fn resource_manager_handle(&self) -> EntityHandle<ResourceManager> {
		self.resource_manager.clone()
	}
}

impl Parameters for GraphicsApplication {
	fn get_parameter(&self, name: &str) -> Option<&Parameter> {
		self.application.get_parameter(name)
	}
}

/// Converts resolved render-pass startup parameters into asynchronous configuration events.
fn queue_render_pass_startup_parameters(parameters: &[Parameter], configuration: &Configuration) {
	for parameter in parameters {
		if parameter.name().starts_with(RENDER_PASS_PARAMETER_PREFIX) {
			configuration.update(parameter.name(), parameter.value());
		}
	}
}

const RENDER_PASS_PARAMETER_PREFIX: &str = "render.pass.";

/// Allocates the application bus and its initial isolated namespaces.
fn create_message_bus(application: &BaseApplication) -> (MessageBus, MessageScope, MessageScope) {
	let message_bus = MessageBus::new(message_bus_config(application)).unwrap_or_else(|error| panic!("{error}"));
	let application_messages = message_bus.new_scope("application");
	let world_messages = message_bus.new_scope("world");
	(message_bus, application_messages, world_messages)
}

/// Resolves the fixed message-storage limits from application startup parameters.
fn message_bus_config(application: &BaseApplication) -> MessageBusConfig {
	let defaults = MessageBusConfig::default();

	MessageBusConfig::new(
		message_bus_limit(application, "messages.max-topics", defaults.max_topics),
		message_bus_limit(application, "messages.cells-per-topic", defaults.cells_per_topic),
		message_bus_limit(application, "messages.cell-bytes", defaults.cell_bytes),
	)
	.with_cell_alignment(message_bus_limit(
		application,
		"messages.cell-alignment",
		defaults.cell_alignment,
	))
	.with_max_listeners_per_topic(message_bus_limit(
		application,
		"messages.listeners-per-topic",
		defaults.max_listeners_per_topic,
	))
}

/// Parses one unsigned message-storage limit while preserving the configured default.
fn message_bus_limit(application: &BaseApplication, name: &str, default: usize) -> usize {
	application
		.get_parameter(name)
		.map(|parameter| {
			parameter.value().parse::<usize>().unwrap_or_else(|error| {
				panic!(
					"Message bus parameter '{name}' is invalid. The most likely cause is that '{}' is not an unsigned integer: {error}",
					parameter.value()
				)
			})
		})
		.unwrap_or(default)
}

/// Resolves an explicit path as supplied while anchoring the development default to its Cargo application.
fn resolve_application_directory(parameter: Option<&Parameter>, default_directory: &str) -> std::path::PathBuf {
	parameter.map(|parameter| parameter.value().into()).unwrap_or_else(|| {
		// Cargo provides the application manifest directory while running development binaries.
		#[cfg(debug_assertions)]
		if let Some(manifest_directory) = std::env::var_os("CARGO_MANIFEST_DIR") {
			return default_application_directory(Some(std::path::Path::new(&manifest_directory)), None, default_directory);
		}

		let executable = std::env::current_exe().unwrap_or_else(|error| {
			panic!(
				"Application directory could not be resolved. The most likely cause is that the current executable path is unavailable: {error}"
			)
		});
		default_application_directory(None, Some(&executable), default_directory)
	})
}

/// Builds a default directory from a Cargo manifest when available, then from the executable.
fn default_application_directory(
	manifest_directory: Option<&std::path::Path>,
	executable: Option<&std::path::Path>,
	directory: &str,
) -> std::path::PathBuf {
	manifest_directory
		.or_else(|| executable.and_then(std::path::Path::parent))
		.unwrap_or_else(|| {
			panic!(
				"Application directory could not be resolved. The most likely cause is that neither a Cargo manifest directory nor an executable parent is available."
			)
		})
		.join(directory)
}

mod pipeline;
use pipeline::drain_render_pass_messages;
pub use pipeline::{
	setup_aces_color_grading_render_pass, setup_aces_tonemap_render_pass, setup_agx_tonemap_render_pass,
	setup_atmosphere_sky_render_pass, setup_bloom_render_pass, setup_dwg_color_grading_render_pass, setup_lut_render_pass,
	setup_pbr_visibility_shading_render_pipeline, setup_simple_render_pipeline, setup_smaa_render_pass, setup_ui_render_pass,
};

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bypass_message_drain_adopts_every_pending_value() {
		let channel = DefaultChannel::new();
		let mut listener = channel.listener();
		channel.send(1);
		channel.send(2);
		let mut adopted = Vec::new();

		drain_render_pass_messages(&mut listener, |value| adopted.push(value));

		assert_eq!(adopted, vec![1, 2]);
		assert!(listener.read().is_none());
	}

	#[test]
	fn application_directories_prefer_the_cargo_manifest() {
		let manifest = std::path::Path::new("app");
		let executable = std::path::Path::new("target/debug/game");

		assert_eq!(
			default_application_directory(Some(manifest), Some(executable), "resources"),
			std::path::Path::new("app/resources")
		);
	}

	#[test]
	fn application_directories_fall_back_beside_the_executable() {
		let executable = std::path::Path::new("app/target/debug/game");

		assert_eq!(
			default_application_directory(None, Some(executable), "resources"),
			std::path::Path::new("app/target/debug/resources")
		);
	}

	#[test]
	fn explicit_application_directories_remain_working_directory_relative() {
		let parameter = Parameter::new("resources.path", "custom/resources");

		assert_eq!(
			resolve_application_directory(Some(&parameter), "resources"),
			std::path::Path::new("custom/resources")
		);
	}

	#[test]
	fn startup_parameters_queue_only_render_pass_configuration() {
		let configuration = Configuration::new();
		let port = configuration.register(RENDER_PASS_PARAMETER_PREFIX);
		let parameters = [
			Parameter::new("render.pass.bloom", "bypassed"),
			Parameter::new("audio.master.gain", "0.5"),
		];

		queue_render_pass_startup_parameters(&parameters, &configuration);

		let update = port.read().expect("render-pass startup configuration");

		assert_eq!(update.parameter(), "render.pass.bloom");
		assert_eq!(update.value(), &crate::configuration::ConfigurationValue::from("bypassed"));
		assert!(port.read().is_none());
		assert_eq!(configuration.events().len(), 1);
	}
}

use core::time;
use std::{sync::Arc, thread};

use ghi::{Context as _, ContextCreate as _, Frame as _, Queue as _};
use resource_management::{
	resource::{
		ReDBStorageBackend, ResourceGpuCompressionPolicy, ResourceStorageMode, ResourceStorageSettings,
		resource_manager::ResourceManager,
	},
	resources::material::Material,
};
use smallvec::SmallVec;
use tracing::{Level, debug_span, instrument, span};
use utils::{Box, sync::RwLock};

use super::{
	Events, Parameter, Time,
	application::{Application, BaseApplication},
};
use crate::{
	application::{parameters::Parameters, thread::Thread},
	audio::generator::Generator,
	configuration::Configuration,
	core::{
		Entity, EntityHandle,
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Creator, Factory},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
		message_bus::{MessageBus, MessageBusConfig, MessageScope},
		task,
	},
	gameplay::{transform::TransformationUpdate, world::DefaultWorld},
	ghi::command_buffer::CommandBufferRecording as _,
	input::{Action, input_trigger},
	inspector::{Inspector, http::HttpInspectorServer},
	physics::dynabit::{self, body::PhysicsBody},
	rendering::{
		Environment, RenderableMesh, UpdatePose,
		pipeline_manager::PipelineManager,
		pipelines::{
			simple::{SimplePipelineManager, SimpleRenderPass},
			visibility::{
				CONE_SHADOW_MAP_POOL_CAPACITY_PARAMETER, POINT_SHADOW_MAP_POOL_CAPACITY_PARAMETER, VisibilityPipelineManager,
				VisibilityPipelineSettings, resource_manager::VisibilityResourcePreparer,
			},
		},
		render_pass::RenderPass,
		render_passes::{
			aces::AcesToneMapPass,
			agx::AgxToneMapPass,
			bloom::{BloomPass, BloomPassSettings},
			color_grading::{ColorGradingPass, ColorGradingWorkflow},
			sky::AtmosphereSkyRenderPass,
			smaa::SmaaPass,
		},
		renderable, renderer,
	},
	time::MediaTime,
	ui::{layout::engine::Render, render_pass::UiRenderPass},
};
impl Creator<Window> for GraphicsApplication {
	fn publish(&self, handle: Option<crate::core::factory::Handle>, window: Window) -> crate::core::factory::Handle {
		if let Some(handle) = handle {
			self.window_factory.0.derive(handle, window);
			handle
		} else {
			self.window_factory.0.create(window)
		}
	}
}

use crate::{
	gameplay::anchor::AnchorSystem,
	input, physics,
	rendering::{self, Camera, renderer::Renderer, window::Window},
};
pub mod defaults;
mod integrations;

pub use defaults::{
	default_setup, setup_animation_pool, setup_default_audio, setup_default_input, setup_default_pipeline_compilation,
	setup_default_resource_and_asset_management, setup_default_window,
};
pub use integrations::process_default_window_input;
#[cfg(feature = "dmx")]
pub use integrations::setup_default_dmx;
