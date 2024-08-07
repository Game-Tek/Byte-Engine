//! The application module contains the application trait and some alternative implementations.\
//! An application is the main entry point of the engine and is responsible for initializing and deinitializing the engine.
//! It also contains the main loop of the engine.
//! An application MUST be a singleton and created before any other engine functionality is used.\
//! All state associated with the application/process should be stored in an application.

/// The application trait is the main entry point of the engine.
/// It is responsible for initializing and deinitializing the engine.
/// It also contains the main loop of the engine.
/// An application MUST be a singleton and created before any other engine functionality is used.\
/// All state associated with the application/process should be stored in an application.
pub trait Application {
	/// Creates a new application with the given name.
	fn new(name: &str) -> Self;

	/// Initializes the application with the given arguments.
	fn initialize(&mut self, arguments: std::env::Args);

	/// Returns the name of the application.
	fn get_name(&self) -> String;

	/// Performs a tick of the application.
	fn tick(&mut self);
}

/// The most basic implementation of the application trait.
/// It has no functionality and is only used as a base for other implementations.
/// It just stores the name of the application.
pub struct BaseApplication {
	name: String,
}

impl Application for BaseApplication {
	fn new(name: &str) -> BaseApplication {
		BaseApplication { name: String::from(name) }
	}

	fn initialize(&mut self, arguments: std::env::Args) {
		let _ = simple_logger::SimpleLogger::new().env().init();

		info!("Byte-Engine");
		info!("Initializing \x1b[4m{}\x1b[24m application with parameters: {}.", self.name, arguments.collect::<Vec<String>>().join(", "));

		trace!("Initialized base Byte-Engine application!");
	}

	fn tick(&mut self) {}

	fn get_name(&self) -> String { self.name.clone() }
}

use core::{property::Property, Entity};
use std::{ops::{Deref, DerefMut}, time::Duration};

use log::{info, trace};
use maths_rs::prelude::Base;

use resource_management::{asset::{asset_manager::AssetManager, audio_asset_handler::AudioAssetHandler, image_asset_handler::ImageAssetHandler, material_asset_handler::MaterialAssetHandler, mesh_asset_handler::MeshAssetHandler}, resource::resource_manager::ResourceManager};
use utils::Extent;
use crate::{audio::audio_system::{self, AudioSystem}, core::{self, entity::EntityHandle, orchestrator}, gameplay::space::Space, input, physics, rendering::{self, common_shader_generator}, window_system::{self, Window}, Vector2};

/// An orchestrated application is an application that uses the orchestrator to manage systems.
/// It is the recommended way to create a simple application.
pub struct OrchestratedApplication {
	application: BaseApplication,
	orchestrator: orchestrator::OrchestratorHandle,
	last_tick_time: std::time::Instant,
	close: bool,
}

impl Application for OrchestratedApplication {
	fn new(name: &str) -> Self {
		let application = Application::new(name);
		let orchestrator = orchestrator::Orchestrator::new_handle();

		OrchestratedApplication { application, orchestrator, last_tick_time: std::time::Instant::now(), close: false }
	}

	fn initialize(&mut self, arguments: std::env::Args) {
		self.application.initialize(arguments);
	}

	fn tick(&mut self) {
		let target_tick_duration = std::time::Duration::from_millis(16);
		let elapsed = self.last_tick_time.elapsed();

		if elapsed < target_tick_duration {
			//std::thread::sleep(target_tick_duration - elapsed);
		}

		self.last_tick_time = std::time::Instant::now();
	}

	fn get_name(&self) -> String { self.application.get_name() }
}

impl OrchestratedApplication {
	/// Flags the application for closing.
	pub fn close(&mut self) {
		self.close = true;
	}

	pub fn get_orchestrator_handle(&self) -> orchestrator::OrchestratorHandle {
		self.orchestrator.clone()
	}

	/// Returns a reference to the orchestrator.
	pub fn get_orchestrator(&self) -> std::cell::Ref<'_, orchestrator::Orchestrator> { self.orchestrator.borrow() }

	/// Returns a mutable reference to the orchestrator.
	pub fn get_mut_orchestrator(&mut self) -> std::cell::RefMut<'_, orchestrator::Orchestrator> { self.orchestrator.as_ref().borrow_mut() }
}

#[derive(Debug, Clone, Copy)]
pub struct Time {
	elapsed: Duration,
	delta: Duration,
}

impl Time {
	pub fn elapsed(&self) -> Duration {
		self.elapsed
	}

	pub fn delta(&self) -> Duration {
		self.delta
	}
}

/// A graphics application is the base for all applications that use the graphics functionality of the engine.
/// It uses the orchestrated application as a base and adds rendering and windowing functionality.
pub struct GraphicsApplication {
	application: OrchestratedApplication,
	runtime: utils::r#async::Runtime,
	tick_count: u64,
	// file_tracker_handle: EntityHandle<file_tracker::FileTracker>,
	window_system_handle: EntityHandle<window_system::WindowSystem>,
	mouse_device_handle: input::DeviceHandle,
	input_system_handle: EntityHandle<input::InputManager>,
	renderer_handle: EntityHandle<rendering::renderer::Renderer>,
	audio_system_handle: EntityHandle<audio_system::DefaultAudioSystem>,
	physics_system_handle: EntityHandle<physics::PhysicsWorld>,
	tick_handle: EntityHandle<Property<Time>>,
	root_space_handle: EntityHandle<Space>,
	start_time: std::time::Instant,

	#[cfg(debug_assertions)]
	min_frame_time: std::time::Duration,
	#[cfg(debug_assertions)]
	max_frame_time: std::time::Duration,
}

impl Application for GraphicsApplication {
	fn new(name: &str) -> Self {
		let mut application = OrchestratedApplication::new(name);

		let runtime = utils::r#async::create_runtime();

		let root_space_handle: EntityHandle<Space> = runtime.block_on(core::spawn(Space::new()));

		application.initialize(std::env::args()); // TODO: take arguments

		let resource_manager = runtime.block_on(core::spawn(ResourceManager::new()));

		{
			let mut resource_manager = resource_manager.write_sync();

			let mut asset_manager = AssetManager::new("resources".into());

			asset_manager.add_asset_handler(MeshAssetHandler::new());

			{
				let mut material_asset_handler = MaterialAssetHandler::new();
				let root_node = besl::Node::root();
				let shader_generator = {
					let common_shader_generator = rendering::common_shader_generator::CommonShaderGenerator::new();
					let visibility_shader_generation = rendering::visibility_shader_generator::VisibilityShaderGenerator::new(root_node.into());
					visibility_shader_generation
				};
				material_asset_handler.set_shader_generator(shader_generator);
				asset_manager.add_asset_handler(material_asset_handler);
			}

			asset_manager.add_asset_handler(ImageAssetHandler::new());
			asset_manager.add_asset_handler(AudioAssetHandler::new());

			resource_manager.set_asset_manager(asset_manager);
		}

		let window_system_handle = runtime.block_on(core::spawn_as_child(root_space_handle.clone(), window_system::WindowSystem::new_as_system()));
		let input_system_handle: EntityHandle<input::InputManager> = runtime.block_on(core::spawn_as_child(root_space_handle.clone(), input::InputManager::new_as_system()));

		let mouse_device_handle;

		{
			let input_system = input_system_handle.get_lock();
			let mut input_system = input_system.blocking_write();

			let mouse_device_class_handle = input_system.register_device_class("Mouse");

			input_system.register_input_source(&mouse_device_class_handle, "Position", input::input_manager::InputTypes::Vector2(input::input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));
			input_system.register_input_source(&mouse_device_class_handle, "LeftButton", input::input_manager::InputTypes::Bool(input::input_manager::InputSourceDescription::new(false, false, false, true)));
			input_system.register_input_source(&mouse_device_class_handle, "RightButton", input::input_manager::InputTypes::Bool(input::input_manager::InputSourceDescription::new(false, false, false, true)));
			input_system.register_input_source(&mouse_device_class_handle, "Scroll", input::input_manager::InputTypes::Float(input::input_manager::InputSourceDescription::new(0f32, 0f32, -1f32, 1f32)));

			let gamepad_device_class_handle = input_system.register_device_class("Gamepad");

			input_system.register_input_source(&gamepad_device_class_handle, "LeftStick", input::input_manager::InputTypes::Vector2(input::input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));
			input_system.register_input_source(&gamepad_device_class_handle, "RightStick", input::input_manager::InputTypes::Vector2(input::input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));

			mouse_device_handle = input_system.create_device(&mouse_device_class_handle);
		}

		// let file_tracker_handle = core::spawn(file_tracker::FileTracker::new());

		let renderer_handle = runtime.block_on(core::spawn_as_child(root_space_handle.clone(), rendering::renderer::Renderer::new_as_system(window_system_handle.clone(), resource_manager.clone())));

		runtime.block_on(core::spawn_as_child::<rendering::render_orchestrator::RenderOrchestrator>(root_space_handle.clone(), rendering::render_orchestrator::RenderOrchestrator::new()));

		runtime.block_on(core::spawn_as_child::<Window>(root_space_handle.clone(), Window::new("Main Window", Extent::rectangle(1920, 1080,))));

		let audio_system_handle = runtime.block_on(core::spawn_as_child(root_space_handle.clone(), audio_system::DefaultAudioSystem::new_as_system(resource_manager.clone())));

		let physics_system_handle = runtime.block_on(core::spawn_as_child(root_space_handle.clone(), physics::PhysicsWorld::new_as_system()));

		let tick_handle = runtime.block_on(core::spawn_as_child(root_space_handle.clone(), Property::new(Time { elapsed: Duration::new(0, 0), delta: Duration::new(0, 0) })));

		GraphicsApplication {
			application,
			window_system_handle,
			input_system_handle,
			mouse_device_handle,
			renderer_handle,
			audio_system_handle,
			physics_system_handle,
			root_space_handle,
			tick_handle,
			runtime,

			tick_count: 0,
			start_time: std::time::Instant::now(),

			#[cfg(debug_assertions)]
			min_frame_time: std::time::Duration::MAX,
			#[cfg(debug_assertions)]
			max_frame_time: std::time::Duration::ZERO,
		}
	}

	fn initialize(&mut self, _arguments: std::env::Args) {
	}

	fn get_name(&self) -> String { self.application.get_name() }

	fn tick(&mut self) {
		let now = std::time::Instant::now();
		let dt = now - self.application.last_tick_time;
		self.application.last_tick_time = std::time::Instant::now();
		// let changed_files = self.file_tracker_handle.poll();

		let mut close = false;

		{
			let window_system = self.window_system_handle.get_lock();
			let mut window_system = window_system.blocking_write();

			{
				let input_system = self.input_system_handle.get_lock();
				let mut input_system = input_system.blocking_write();

				window_system.update_windows(|_, event| {
					match event {
						ghi::WindowEvents::Close => { close = true },
						ghi::WindowEvents::Button { pressed, button } => {
							match button {
								ghi::MouseKeys::Left => {
									input_system.record_input_source_action(&self.mouse_device_handle, input::input_manager::InputSourceAction::Name("Mouse.LeftButton"), input::Value::Bool(pressed));
								},
								ghi::MouseKeys::Right => {
									input_system.record_input_source_action(&self.mouse_device_handle, input::input_manager::InputSourceAction::Name("Mouse.RightButton"), input::Value::Bool(pressed));
								},
								ghi::MouseKeys::ScrollUp => {
									input_system.record_input_source_action(&self.mouse_device_handle, input::input_manager::InputSourceAction::Name("Mouse.Scroll"), input::Value::Float(1f32));
								},
								ghi::MouseKeys::ScrollDown => {
									input_system.record_input_source_action(&self.mouse_device_handle, input::input_manager::InputSourceAction::Name("Mouse.Scroll"), input::Value::Float(-1f32));
								},
								_ => { }
							}
						},
						ghi::WindowEvents::MouseMove { x, y, time: _ } => {
							let vec = Vector2::new((x as f32 / 1920f32 - 0.5f32) * 2f32, (y as f32 / 1080f32 - 0.5f32) * 2f32);
							input_system.record_input_source_action(&self.mouse_device_handle, input::input_manager::InputSourceAction::Name("Mouse.Position"), input::Value::Vector2(vec));
						},
						ghi::WindowEvents::Resize { width, height } => {
							log::debug!("Resizing window to {}x{}", width, height);
						}
						_ => { }
					}
				});
			}
		}

		let time = Time { elapsed: self.start_time.elapsed(), delta: dt };

		self.tick_handle.sync_get_mut(move |tick| {
			tick.set(|_| time);
		});

		self.input_system_handle.map(|handle| {
			let mut e = handle.write_sync();
			e.update();
		});

		self.physics_system_handle.map(move |handle| {
			let mut e = handle.write_sync();
			e.update(time);
		});

		self.renderer_handle.map(|handle| {
			let mut e = handle.write_sync();
			e.render();
		});

		self.audio_system_handle.map(|handle| {
			let mut e = handle.write_sync();
			e.render();
		});

		if close {
			self.application.close();
		}

		self.tick_count += 1;

		#[cfg(debug_assertions)]
		{
			self.min_frame_time = self.min_frame_time.min(dt);
			self.max_frame_time = self.max_frame_time.max(dt);
		}
	}
}

impl GraphicsApplication {
	/// Flags the application for closing.
	pub fn close(&mut self) {
		self.application.close();

		#[cfg(debug_assertions)]
		log::debug!("Run stats:\n\tAverage frame time: {:#?}\n\tMin frame time: {:#?}\n\tMax frame time: {:#?}", self.start_time.elapsed().div_f32(self.tick_count as f32), self.min_frame_time, self.max_frame_time);
	}

	pub fn get_orchestrator_handle(&self) -> orchestrator::OrchestratorHandle {
		self.application.orchestrator.clone()
	}

	/// Returns a reference to the orchestrator.
	pub fn get_orchestrator(&self) -> std::cell::Ref<'_, orchestrator::Orchestrator> { self.application.get_orchestrator() }
	pub fn get_mut_orchestrator(&mut self) -> std::cell::RefMut<'_, orchestrator::Orchestrator> { self.application.get_mut_orchestrator() }

	pub fn get_runtime(&self) -> &utils::r#async::Runtime {
		&self.runtime
	}

	pub fn get_input_system_handle_ref(&self) -> &EntityHandle<input::InputManager> {
		&self.input_system_handle
	}

	pub fn get_audio_system_handle(&self) -> &EntityHandle<crate::audio::audio_system::DefaultAudioSystem> {
		&self.audio_system_handle
	}

	pub fn get_physics_world_handle(&self) -> &EntityHandle<crate::physics::PhysicsWorld> {
		&self.physics_system_handle
	}

	pub fn get_root_space_handle(&self) -> &EntityHandle<crate::gameplay::space::Space> {
		&self.root_space_handle
	}

	pub fn get_tick_handle(&self) -> &EntityHandle<Property<Time>> {
		&self.tick_handle
	}

	pub fn do_loop(&mut self) {
		while !self.application.close {
			self.tick();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn create_base_application() {
		let mut app = BaseApplication::new("Test");
		app.initialize(std::env::args());

		assert!(app.get_name() == "Test");
	}

	#[test]
	fn create_orchestrated_application() {
		let mut app = OrchestratedApplication::new("Test");
		app.initialize(std::env::args());

		assert!(app.get_name() == "Test");
	}

	#[test]
	#[ignore] // Renderer broken.
	fn create_graphics_application() {
		let mut app = GraphicsApplication::new("Test");
		app.initialize(std::env::args());

		assert!(app.get_name() == "Test");

		let start_time = std::time::Instant::now();

		while !app.application.close {
			app.tick();

			if start_time.elapsed().as_secs() > 1 {
				app.close();
			}
		}
	}
}
