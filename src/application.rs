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

	/// Deinitializes the application.
	fn deinitialize(&mut self);
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

	fn deinitialize(&mut self) {
		trace!("Deinitializing base Byte-Engine application...");
		info!("Deinitialized base Byte-Engine application.");
	}

	fn tick(&mut self) {}

	fn get_name(&self) -> String { self.name.clone() }
}

use std::ops::{DerefMut, Deref};

use log::{info, trace};
use maths_rs::prelude::Base;

use crate::{core::{self, orchestrator::{self,}, entity::EntityHandle}, window_system, input_manager, Vector2, rendering::{self}, resource_management::{self, mesh_resource_handler::MeshResourceHandler, texture_resource_handler::ImageResourceHandler, audio_resource_handler::AudioResourceHandler, material_resource_handler::MaterialResourcerHandler}, file_tracker, audio::audio_system::{self, AudioSystem}, physics, gameplay::space::Space};

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

	fn deinitialize(&mut self) {
		self.application.deinitialize();
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

/// A graphics application is the base for all applications that use the graphics functionality of the engine.
/// It uses the orchestrated application as a base and adds rendering and windowing functionality.
pub struct GraphicsApplication {
	application: OrchestratedApplication,
	tick_count: u64,
	file_tracker_handle: EntityHandle<file_tracker::FileTracker>,
	window_system_handle: EntityHandle<window_system::WindowSystem>,
	mouse_device_handle: input_manager::DeviceHandle,
	input_system_handle: EntityHandle<input_manager::InputManager>,
	renderer_handle: EntityHandle<rendering::renderer::Renderer>,
	audio_system_handle: EntityHandle<audio_system::DefaultAudioSystem>,
	physics_system_handle: EntityHandle<physics::PhysicsWorld>,
	root_space_handle: EntityHandle<Space>,
}

impl Application for GraphicsApplication {
	fn new(name: &str) -> Self {
		let mut application = OrchestratedApplication::new(name);

		let root_space_handle: EntityHandle<Space> = core::spawn(Space::new());

		application.initialize(std::env::args()); // TODO: take arguments

		let resource_manager_handle: EntityHandle<resource_management::resource_manager::ResourceManager> = core::spawn(resource_management::resource_manager::ResourceManager::new_as_system());

		{
			let mut resource_manager = resource_manager_handle.write_sync();
			resource_manager.add_resource_handler(MeshResourceHandler::new());
			resource_manager.add_resource_handler(ImageResourceHandler::new());
			resource_manager.add_resource_handler(AudioResourceHandler::new());
			resource_manager.add_resource_handler(MaterialResourcerHandler::new());
		}

		let window_system_handle = core::spawn_as_child(root_space_handle.clone(), window_system::WindowSystem::new_as_system());
		let input_system_handle: EntityHandle<input_manager::InputManager> = core::spawn_as_child(root_space_handle.clone(), input_manager::InputManager::new_as_system());

		let mouse_device_handle;

		{
			let input_system = input_system_handle.get_lock();
			let mut input_system = input_system.write_arc_blocking();

			let mouse_device_class_handle = input_system.register_device_class("Mouse");
	
			input_system.register_input_source(&mouse_device_class_handle, "Position", input_manager::InputTypes::Vector2(input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));
			input_system.register_input_source(&mouse_device_class_handle, "LeftButton", input_manager::InputTypes::Bool(input_manager::InputSourceDescription::new(false, false, false, true)));
			input_system.register_input_source(&mouse_device_class_handle, "RightButton", input_manager::InputTypes::Bool(input_manager::InputSourceDescription::new(false, false, false, true)));
	
			let gamepad_device_class_handle = input_system.register_device_class("Gamepad");
	
			input_system.register_input_source(&gamepad_device_class_handle, "LeftStick", input_manager::InputTypes::Vector2(input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));
			input_system.register_input_source(&gamepad_device_class_handle, "RightStick", input_manager::InputTypes::Vector2(input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));
	
			mouse_device_handle = input_system.create_device(&mouse_device_class_handle);
		}

		let file_tracker_handle = core::spawn(file_tracker::FileTracker::new());

		let renderer_handle = core::spawn_as_child(root_space_handle.clone(), rendering::renderer::Renderer::new_as_system(window_system_handle.clone(), resource_manager_handle.clone()));

		core::spawn_as_child::<rendering::render_orchestrator::RenderOrchestrator>(root_space_handle.clone(), rendering::render_orchestrator::RenderOrchestrator::new());

		core::spawn_as_child(root_space_handle.clone(), window_system::Window::new("Main Window", crate::Extent { width: 1920, height: 1080, depth: 1 }));

		let audio_system_handle = core::spawn_as_child(root_space_handle.clone(), audio_system::DefaultAudioSystem::new_as_system(resource_manager_handle.clone()));

		let physics_system_handle = core::spawn_as_child(root_space_handle.clone(), physics::PhysicsWorld::new_as_system());

		GraphicsApplication { application, file_tracker_handle, window_system_handle, input_system_handle, mouse_device_handle, renderer_handle, tick_count: 0, audio_system_handle, physics_system_handle, root_space_handle }
	}

	fn initialize(&mut self, _arguments: std::env::Args) {
	}

	fn get_name(&self) -> String { self.application.get_name() }

	fn deinitialize(&mut self) {
		self.application.deinitialize();
	}

	fn tick(&mut self) {
		self.application.tick();
		// let changed_files = self.file_tracker_handle.poll();

		let mut close = false;

		{
			let window_system = self.window_system_handle.get_lock();
			let window_system = window_system.write_arc_blocking();

			{
				let input_system = self.input_system_handle.get_lock();
				let mut input_system = input_system.write_arc_blocking();
				
				window_system.update_windows(|_, event| {
					match event {
						window_system::WindowEvents::Close => { close = true },
						window_system::WindowEvents::Button { pressed, button } => {
							match button {
								window_system::MouseKeys::Left => {
									input_system.record_input_source_action(&self.mouse_device_handle, input_manager::InputSourceAction::Name("Mouse.LeftButton"), input_manager::Value::Bool(pressed));
								},
								window_system::MouseKeys::Right => {
									input_system.record_input_source_action(&self.mouse_device_handle, input_manager::InputSourceAction::Name("Mouse.RightButton"), input_manager::Value::Bool(pressed));
								},
								_ => { }
							}
						},
						window_system::WindowEvents::MouseMove { x, y, time: _ } => {
							let vec = Vector2::new((x as f32 / 1920f32 - 0.5f32) * 2f32, (y as f32 / 1080f32 - 0.5f32) * 2f32);
							input_system.record_input_source_action(&self.mouse_device_handle, input_manager::InputSourceAction::Name("Mouse.Position"), input_manager::Value::Vector2(vec));
						},
						_ => { }
					}
				});
			}
		}

		self.input_system_handle.map(|handle| {
			let mut e = handle.write_sync();
			e.update();
		});
		
		self.physics_system_handle.map(|handle| {
			let mut e = handle.write_sync();
			e.update();
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
	}
}

impl GraphicsApplication {
	/// Flags the application for closing.
	pub fn close(&mut self) {
		self.application.close();
	}

	pub fn get_orchestrator_handle(&self) -> orchestrator::OrchestratorHandle {
		self.application.orchestrator.clone()
	}

	/// Returns a reference to the orchestrator.
	pub fn get_orchestrator(&self) -> std::cell::Ref<'_, orchestrator::Orchestrator> { self.application.get_orchestrator() }
	pub fn get_mut_orchestrator(&mut self) -> std::cell::RefMut<'_, orchestrator::Orchestrator> { self.application.get_mut_orchestrator() }

	pub fn get_input_system_handle_ref(&self) -> &EntityHandle<crate::input_manager::InputManager> {
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

		app.deinitialize();
	}

	#[test]
	fn create_orchestrated_application() {
		let mut app = OrchestratedApplication::new("Test");
		app.initialize(std::env::args());

		assert!(app.get_name() == "Test");

		app.deinitialize();
	}

	#[test]
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

		app.deinitialize();
	}
}