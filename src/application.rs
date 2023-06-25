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

use crate::{orchestrator, window_system, render_system};

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
			std::thread::sleep(target_tick_duration - elapsed);
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
	file_tracker: crate::file_tracker::FileTracker,
	window_system_handle: Option<orchestrator::SystemHandle>,
	render_system_handle: Option<orchestrator::SystemHandle>,
}

impl Application for GraphicsApplication {
	fn new(name: &str) -> Self {
		let application = OrchestratedApplication::new(name);

		GraphicsApplication { application, file_tracker: crate::file_tracker::FileTracker::new(), window_system_handle: None, render_system_handle: None }
	}

	fn initialize(&mut self, arguments: std::env::Args) {
		self.application.initialize(arguments);

		let mut window_system = window_system::WindowSystem::new();
		let mut render_system = render_system::RenderSystem::new();

		let window_handle = window_system.create_window("Main Window", crate::Extent { width: 1920, height: 1080, depth: 1 }, "main_window");

		render_system.bind_to_window(window_system.get_os_handles(window_handle));

		self.window_system_handle = Some(self.application.get_mut_orchestrator().add_system(window_system));
		//self.render_system_handle = Some(self.application.get_mut_orchestrator().add_system(render_system));

		self.file_tracker.watch(std::path::Path::new("configuration.json"));
	}

	fn get_name(&self) -> String { self.application.get_name() }

	fn deinitialize(&mut self) {
		self.application.deinitialize();
	}

	fn tick(&mut self) {
		self.application.tick();
		self.file_tracker.poll();
	}
}

impl GraphicsApplication {
	/// Flags the application for closing.
	pub fn close(&mut self) {
		self.application.close();
	}

	/// Returns a reference to the orchestrator.
	pub fn get_orchestrator(&self) -> &orchestrator::Orchestrator { &self.application.get_orchestrator() }
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

			println!("Tick!");

			if start_time.elapsed().as_secs() > 1 {
				app.close();
			}
		}

		app.deinitialize();
	}
}