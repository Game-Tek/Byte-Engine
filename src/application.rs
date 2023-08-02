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
		println!("\x1b[32m\x1b[1mByte-Engine\x1b[0m");
		println!("Initializing \x1b[4m{}\x1b[24m application with parameters: {}.", self.name, arguments.collect::<Vec<String>>().join(", "));

		println!("Initialized base Byte-Engine application!");
	}

	fn deinitialize(&mut self) {
		println!("Deinitializing base Byte-Engine application...");

		println!("Deinitialized base Byte-Engine application.");
	}

	fn tick(&mut self) { return; }

	fn get_name(&self) -> String { self.name.clone() }
}

use maths_rs::prelude::Base;

use crate::{orchestrator, window_system, render_system, input_manager, Vector2, rendering, render_domain, resource_manager, file_tracker};

/// An orchestrated application is an application that uses the orchestrator to manage systems.
/// It is the recommended way to create a simple application.
pub struct OrchestratedApplication {
	application: BaseApplication,
	orchestrator: orchestrator::Orchestrator,
	last_tick_time: std::time::Instant,
	close: bool,
}

impl Application for OrchestratedApplication {
	fn new(name: &str) -> Self {
		let application = Application::new(name);
		let orchestrator = orchestrator::Orchestrator::new();

		OrchestratedApplication { application, orchestrator, last_tick_time: std::time::Instant::now(), close: false }
	}

	fn initialize(&mut self, arguments: std::env::Args) {
		self.application.initialize(arguments);
		self.orchestrator.initialize();
	}

	fn deinitialize(&mut self) {
		self.orchestrator.deinitialize();
		self.application.deinitialize();
	}

	fn tick(&mut self) {
		let target_tick_duration = std::time::Duration::from_millis(16);
		let elapsed = self.last_tick_time.elapsed();

		if elapsed < target_tick_duration {
			//std::thread::sleep(target_tick_duration - elapsed);
		}

		self.last_tick_time = std::time::Instant::now();

		self.orchestrator.update();
	}

	fn get_name(&self) -> String { self.application.get_name() }
}

impl OrchestratedApplication {
	/// Flags the application for closing.
	pub fn close(&mut self) {
		self.close = true;
	}

	/// Returns a reference to the orchestrator.
	pub fn get_orchestrator(&self) -> &orchestrator::Orchestrator { &self.orchestrator }

	/// Returns a mutable reference to the orchestrator.
	pub fn get_mut_orchestrator(&mut self) -> &mut orchestrator::Orchestrator { &mut self.orchestrator }
}

/// A graphics application is the base for all applications that use the graphics functionality of the engine.
/// It uses the orchestrated application as a base and adds rendering and windowing functionality.
pub struct GraphicsApplication {
	application: OrchestratedApplication,
	tick_count: u64,
	file_tracker_handle: orchestrator::EntityHandle<file_tracker::FileTracker>,
	window_system_handle: orchestrator::EntityHandle<window_system::WindowSystem>,
	mouse_device_handle: input_manager::DeviceHandle,
	input_system_handle: orchestrator::EntityHandle<input_manager::InputManager>,
	visibility_render_domain_handle: orchestrator::EntityHandle<render_domain::VisibilityWorldRenderDomain>,
	render_system_handle: orchestrator::EntityHandle<render_system::RenderSystem>,
}

impl Application for GraphicsApplication {
	fn new(name: &str) -> Self {
		let mut application = OrchestratedApplication::new(name);

		application.initialize(std::env::args()); // TODO: take arguments

		let orchestrator = application.get_mut_orchestrator();

		let window_system_handle = orchestrator.spawn_entity(window_system::WindowSystem::new_as_system);

		let input_system_handle = orchestrator.spawn_entity(input_manager::InputManager::new_as_system);

		let (mouse_device_class_handle, gamepad_device_class_handle, mouse_device_handle) = orchestrator.get_mut_and(&input_system_handle, |input_system: &mut input_manager::InputManager| {
			let mouse_device_class_handle = input_system.register_device_class("Mouse");

			input_system.register_input_source(&mouse_device_class_handle, "Position", input_manager::InputTypes::Vector2(input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));
			input_system.register_input_source(&mouse_device_class_handle, "LeftButton", input_manager::InputTypes::Bool(input_manager::InputSourceDescription::new(false, false, false, true)));
			input_system.register_input_source(&mouse_device_class_handle, "RightButton", input_manager::InputTypes::Bool(input_manager::InputSourceDescription::new(false, false, false, true)));

			let gamepad_device_class_handle = input_system.register_device_class("Gamepad");

			input_system.register_input_source(&gamepad_device_class_handle, "LeftStick", input_manager::InputTypes::Vector2(input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));
			input_system.register_input_source(&gamepad_device_class_handle, "RightStick", input_manager::InputTypes::Vector2(input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));

			let mouse_device_handle = input_system.create_device(&mouse_device_class_handle);

			(mouse_device_class_handle, gamepad_device_class_handle, mouse_device_handle)
		});

		let file_tracker_handle = orchestrator.spawn_entity(file_tracker::FileTracker::new);

		let render_system_handle = orchestrator.spawn_entity(render_system::RenderSystem::new_as_system);

		orchestrator.get_2_mut_and(&window_system_handle, &render_system_handle, |window_system, render_system: &mut render_system::RenderSystem| {
			let window_handle = window_system.create_window("Main Window", crate::Extent { width: 1920, height: 1080, depth: 1 }, "main_window");
			render_system.bind_to_window(window_system.get_os_handles(&window_handle));
		});

		let visibility_render_domain_handle = orchestrator.spawn_entity(render_domain::VisibilityWorldRenderDomain::new);

		orchestrator.spawn_entity(rendering::render_orchestrator::RenderOrchestrator::new);
		orchestrator.spawn_system(resource_manager::ResourceManager::new_as_system);

		GraphicsApplication { application, file_tracker_handle: file_tracker_handle, window_system_handle, input_system_handle, mouse_device_handle, visibility_render_domain_handle, tick_count: 0, render_system_handle }
	}

	fn initialize(&mut self, arguments: std::env::Args) {
	}

	fn get_name(&self) -> String { self.application.get_name() }

	fn deinitialize(&mut self) {
		self.application.deinitialize();
	}

	fn tick(&mut self) {
		self.application.tick();
		// let changed_files = self.file_tracker_handle.poll();

		let window_res = self.application.get_orchestrator().get_2_mut_and(&self.window_system_handle, &self.input_system_handle, |window_system, input_system| {
			while let Some(event) = window_system.update_window(0) {
				match event {
					window_system::WindowEvents::Close => return false,
					window_system::WindowEvents::Button { pressed, button } => {
						input_system.record_input_source_action(&self.mouse_device_handle, input_manager::InputSourceAction::Name("Mouse.LeftButton"), input_manager::Value::Bool(pressed));
					},
					window_system::WindowEvents::MouseMove { x, y, time } => {
						let vec = Vector2::new((x as f32 / 1920f32 - 0.5f32) * 2f32, (y as f32 / 1080f32 - 0.5f32) * 2f32);
						input_system.record_input_source_action(&self.mouse_device_handle, input_manager::InputSourceAction::Name("Mouse.Position"), input_manager::Value::Vector2(vec));
					},
					_ => {}
				}
			}
			
			true
		});

		self.application.get_orchestrator().invoke_mut(self.input_system_handle.copy(), input_manager::InputManager::update);

		// self.application.get_orchestrator().get_2_mut_and(&self.render_system_handle, &self.visibility_render_domain_handle, |render_system, visibility_render_domain| {
		// 	// let files = changed_files.iter().filter(|event| {
		// 	// 	println!("File changed: {:?}", event.event.paths);
		// 	// 	event.kind.is_modify()
		// 	// }).for_each(|event| {
		// 	// 	println!("File changed: {:?}", event.event.paths);

		// 	// 	let shader_source = std::fs::read_to_string(event.event.paths[0].to_str().unwrap()).unwrap();

		// 	// 	println!("Shader source: {}", shader_source);

		// 	// 	visibility_render_domain.update_shader(render_system, event.paths[0].to_str().unwrap(), shader_source.as_str());
		// 	// });

		// 	visibility_render_domain.render(self.get_orchestrator(), render_system, self.tick_count as u32);
		// });
		
		self.application.get_orchestrator().invoke_mut(self.visibility_render_domain_handle.copy(), render_domain::VisibilityWorldRenderDomain::render);

		if !window_res {
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

	/// Returns a reference to the orchestrator.
	pub fn get_orchestrator(&self) -> &orchestrator::Orchestrator { self.application.get_orchestrator() }
	pub fn get_mut_orchestrator(&mut self) -> &mut orchestrator::Orchestrator { self.application.get_mut_orchestrator() }

	pub fn get_input_system_handle(&self) -> orchestrator::EntityHandle<crate::input_manager::InputManager> {
		self.input_system_handle.copy()
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

	#[ignore = "Ignore until we have a way to disable this test in CI where windows are not supported"]
	#[test]
	fn create_graphics_application() {
		let mut app = GraphicsApplication::new("Test");
		app.initialize(std::env::args());

		assert!(app.get_name() == "Test");

		let start_time = std::time::Instant::now();

		while !app.application.close {
			app.tick();

			println!("Tick!");

			if start_time.elapsed().as_secs() > 1 {
				app.close();
			}
		}

		app.deinitialize();
	}
}