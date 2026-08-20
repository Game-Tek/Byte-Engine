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

	tick_count: u64,
	start_time: std::time::Instant,
	last_tick_time: MediaTime,

	close: bool,

	application_events: (Sender<Events>, Receiver<Events>),
	http_inspector: HttpInspectorServer,
	configuration: Configuration,

	window_factory: (Factory<Window>, DefaultListener<CreateMessage<Window>>),
	action_factory: Factory<Action>,

	generator_factory: Factory<Arc<dyn Generator>>,

	world_factory: Factory<DefaultWorld>,
	world: DefaultWorld,
	cameras_listener: DefaultListener<crate::core::factory::CreateMessage<Camera>>,
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

impl Application for GraphicsApplication {
	fn new(name: &str, parameters: &[Parameter]) -> Self {
		let start_time = std::time::Instant::now();

		let application = BaseApplication::new(name, parameters);

		let resources_path = resolve_application_directory(application.get_parameter("resources.path"), "resources");

		// Opening an application store first removes resources baked by an incompatible engine revision.
		let resource_storage = ReDBStorageBackend::new(resources_path);
		let resource_manager = EntityHandle::from(ResourceManager::new(resource_storage));

		let action_factory = Factory::new();

		let input_system = {
			let action_listener = action_factory.listener();
			let event_channel = DefaultChannel::new();

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

		let tx = Sender::new(16);

		ctrlc::set_handler({
			let tx = tx.clone();
			move || {
				tx.send(Events::Close).unwrap();
			}
		})
		.unwrap();

		let inspector = EntityHandle::from(Inspector::new(tx.clone(), configuration.clone()));
		let http_inspector = HttpInspectorServer::new(inspector);

		let rx = tx.spawn_rx();
		let application_events = (tx, rx);

		let window_factory = Factory::new();
		let window_factory_listener = window_factory.listener();

		let world = DefaultWorld::new();
		let cameras_listener = world.camera_factory().listener();
		let renderer_transforms_listener = world.transforms_channel().listener();

		GraphicsApplication {
			application,

			application_events,
			http_inspector,
			configuration,

			window_factory: (window_factory, window_factory_listener),
			action_factory,

			generator_factory: Factory::new(),

			world_factory: Factory::new(),
			world,
			cameras_listener,
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
			if let Ok(e) = self.application_events.1.try_recv() {
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

		let mut physics_transforms_listener = self.world.transforms_channel().listener();

		let result = {
			let span = debug_span!("GraphicsApplication::user_tick");
			let _enter = span.enter();
			f(self, time)
		};

		{
			let span = debug_span!("GraphicsApplication::update_world");
			let _enter = span.enter();
			self.world
				.update(time, &mut physics_transforms_listener, &mut self.application.frame_allocator);
		}

		{
			let span = debug_span!("GraphicsApplication::prepare_renderer_state");
			let _enter = span.enter();
			let camera_messages = self.world.camera_factory_mut().drain_created_before_listener();

			let window_listener = &mut self.window_factory.1;

			while let Some(message) = window_listener.read() {
				self.renderer.create_window(message.into_data());
			}

			for message in camera_messages {
				self.renderer.create_camera(*message.handle(), message.into_data());
			}

			while let Some(message) = self.cameras_listener.read() {
				self.renderer.create_camera(*message.handle(), message.into_data());
			}
		}

		{
			let span = debug_span!("GraphicsApplication::render_frame");
			let _enter = span.enter();
			let frame_allocator = &self.application.frame_allocator;
			self.renderer.prepare(&mut self.renderer_transforms_listener, frame_allocator);
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

		while self.application_events.1.try_recv().is_ok() {}
		let _ = self.application_events.0.blocking_send(Events::Close);
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

	/// Returns the factory used to request new windows.
	pub fn window_factory(&self) -> &Factory<Window> {
		&self.window_factory.0
	}

	/// Returns mutable access to the factory used to request new windows.
	pub fn window_factory_mut(&mut self) -> &mut Factory<Window> {
		&mut self.window_factory.0
	}

	/// Returns the factory used to register input actions.
	pub fn action_factory(&self) -> &Factory<Action> {
		&self.action_factory
	}

	/// Returns mutable access to the factory used to register input actions.
	pub fn action_factory_mut(&mut self) -> &mut Factory<Action> {
		&mut self.action_factory
	}

	/// Returns the factory used to create additional worlds.
	pub fn world_factory(&self) -> &Factory<DefaultWorld> {
		&self.world_factory
	}

	/// Returns mutable access to the factory used to create additional worlds.
	pub fn world_factory_mut(&mut self) -> &mut Factory<DefaultWorld> {
		&mut self.world_factory
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

	/// Returns mutable access to the audio generator factory used by default audio setup.
	pub fn generator_factory_mut(&mut self) -> &mut Factory<Arc<dyn Generator>> {
		&mut self.generator_factory
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
	setup_aces_tonemap_render_pass, setup_agx_tonemap_render_pass, setup_atmosphere_sky_render_pass, setup_bloom_render_pass,
	setup_lut_render_pass, setup_pbr_visibility_shading_render_pipeline, setup_simple_render_pipeline, setup_smaa_render_pass,
	setup_swapchain_blit_render_pass, setup_ui_render_pass,
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

	#[test]
	#[ignore] // Renderer broken.
	fn create_graphics_application() {
		let mut app = GraphicsApplication::new("Test", &[]);

		assert_eq!(app.get_name(), "Test");

		let start_time = std::time::Instant::now();

		while !app.close {
			app.tick();

			if start_time.elapsed().as_secs() > 1 {
				app.close();
			}
		}
	}
}

use core::time;
use std::{collections::VecDeque, sync::Arc, thread};

use ghi::{Context as _, ContextCreate as _, Frame as _, Queue as _};
use resource_management::{
	resource::{resource_manager::ResourceManager, ReDBStorageBackend},
	resources::material::Material,
};
use smallvec::SmallVec;
use tracing::{debug_span, instrument, span, Level};
use utils::{sync::RwLock, Box};

use super::{
	application::{Application, BaseApplication},
	Events, Parameter, Receiver, Sender, Time,
};
use crate::{
	application::{parameters::Parameters, thread::Thread},
	audio::generator::Generator,
	configuration::Configuration,
	core::{
		channel::{Channel, DefaultChannel},
		factory::{CreateMessage, Creator, Factory},
		listener::{DefaultListener, Listener},
		message::DeleteMessage,
		task, Entity, EntityHandle,
	},
	gameplay::{transform::TransformationUpdate, world::DefaultWorld},
	ghi::command_buffer::CommandBufferRecording as _,
	input::{input_trigger, Action},
	inspector::{http::HttpInspectorServer, Inspector},
	physics::dynabit::{self, body::PhysicsBody},
	rendering::{
		lights::{Light, Lights},
		pipeline_manager::PipelineManager,
		pipelines::{
			simple::{SimplePipelineManager, SimpleRenderPass},
			visibility::{
				resource_manager::VisibilityPipelineResourceManager, VisibilityPipelineManager, VisibilityPipelineSettings,
				CONE_SHADOW_MAP_POOL_CAPACITY_PARAMETER, POINT_SHADOW_MAP_POOL_CAPACITY_PARAMETER,
			},
		},
		render_pass::RenderPass,
		render_passes::{
			aces::AcesToneMapPass,
			agx::AgxToneMapPass,
			blit::SwapchainBlitPass,
			bloom::{BloomPass, BloomPassSettings},
			sky::AtmosphereSkyRenderPass,
			smaa::SmaaPass,
		},
		renderable, renderer, Environment, RenderableMesh, UpdatePose,
	},
	time::MediaTime,
	ui::{layout::engine::Render, render_pass::UiRenderPass},
};
impl Creator<Window> for GraphicsApplication {
	fn publish(&mut self, handle: Option<crate::core::factory::Handle>, window: Window) -> crate::core::factory::Handle {
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
	rendering::{self, renderer::Renderer, window::Window, Camera},
};
pub mod defaults;
mod integrations;

pub use defaults::{
	default_setup, setup_default_audio, setup_default_input, setup_default_pipeline_compilation,
	setup_default_resource_and_asset_management, setup_default_window,
};
pub use integrations::process_default_window_input;
#[cfg(feature = "dmx")]
pub use integrations::setup_default_dmx;
