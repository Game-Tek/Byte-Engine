use crate::core::{property::Property, EntityHandle, spawn, spawn_as_child};
use std::time::Duration;

use maths_rs::num::Base;
use resource_management::{asset::{asset_manager::AssetManager, audio_asset_handler::AudioAssetHandler, image_asset_handler::ImageAssetHandler, material_asset_handler::MaterialAssetHandler, mesh_asset_handler::MeshAssetHandler}, resource::resource_manager::ResourceManager};
use utils::Extent;

use crate::{audio::audio_system::{AudioSystem, DefaultAudioSystem}, gameplay::{anchor::AnchorSystem, space::Space}, input, physics, rendering::{self, common_shader_generator::CommonShaderGenerator, renderer::Renderer, visibility_shader_generator::VisibilityShaderGenerator}, window_system::{self, Window}, Vector2};

use super::{application::{Application, BaseApplication}, Parameter, Time};

/// A graphics application is the base for all applications that use the graphics functionality of the engine.
/// It uses the orchestrated application as a base and adds rendering and windowing functionality.
pub struct GraphicsApplication {
	application: BaseApplication,
	runtime: utils::r#async::Runtime,

	tick_count: u64,
	start_time: std::time::Instant,
	last_tick_time: std::time::Instant,
	
	close: bool,

	window_system_handle: EntityHandle<window_system::WindowSystem>,

	input_system_handle: EntityHandle<input::InputManager>,
	mouse_device_handle: input::DeviceHandle,
	keyboard_device_handle: input::DeviceHandle,
	gamepad_device_handle: input::DeviceHandle,

	renderer_handle: EntityHandle<Renderer>,
	audio_system_handle: EntityHandle<DefaultAudioSystem>,
	physics_system_handle: EntityHandle<physics::PhysicsWorld>,
	anchor_system_handle: EntityHandle<AnchorSystem>,
	tick_handle: EntityHandle<Property<Time>>,
	root_space_handle: EntityHandle<Space>,

	#[cfg(debug_assertions)]
	ttff: std::time::Duration,
	#[cfg(debug_assertions)]
	min_frame_time: std::time::Duration,
	#[cfg(debug_assertions)]
	max_frame_time: std::time::Duration,

	#[cfg(debug_assertions)]
	kill_after: Option<u64>,
}

impl Application for GraphicsApplication {
	fn new(name: &str, parameters: &[Parameter],) -> Self {
		let start_time = std::time::Instant::now();

		let application = BaseApplication::new(name, parameters);

		let runtime = utils::r#async::create_runtime();

		let root_space_handle: EntityHandle<Space> = runtime.block_on(spawn(Space::new()));

		let resources_path: std::path::PathBuf = application.get_parameter("resources-path").map(|p| p.value.clone()).unwrap_or_else(|| "resources".into()).into();
		let assets_path: std::path::PathBuf = application.get_parameter("assets-path").map(|p| p.value.clone()).unwrap_or_else(|| "assets".into()).into();

		let resource_manager = runtime.block_on(spawn(ResourceManager::new(resources_path.clone())));

		{
			let mut resource_manager = resource_manager.write_sync();

			let mut asset_manager = AssetManager::new(assets_path, resources_path);

			asset_manager.add_asset_handler(MeshAssetHandler::new());

			{
				let mut material_asset_handler = MaterialAssetHandler::new();
				let root_node = besl::Node::root();
				let shader_generator = {
					let common_shader_generator = CommonShaderGenerator::new();
					let visibility_shader_generation = VisibilityShaderGenerator::new(root_node.into());
					visibility_shader_generation
				};
				material_asset_handler.set_shader_generator(shader_generator);
				asset_manager.add_asset_handler(material_asset_handler);
			}

			asset_manager.add_asset_handler(ImageAssetHandler::new());
			asset_manager.add_asset_handler(AudioAssetHandler::new());

			resource_manager.set_asset_manager(asset_manager);
		}

		let window_system_handle = runtime.block_on(spawn_as_child(root_space_handle.clone(), window_system::WindowSystem::new_as_system()));
		let input_system_handle: EntityHandle<input::InputManager> = runtime.block_on(spawn_as_child(root_space_handle.clone(), input::InputManager::new_as_system()));

		let mouse_device_handle;
		let keyboard_device_handle;
		let gamepad_device_handle;

		{
			let input_system = input_system_handle.get_lock();
			let mut input_system = input_system.blocking_write();

			let mouse_device_class_handle = input_system.register_device_class("Mouse");

			input_system.register_input_source(&mouse_device_class_handle, "Position", input::input_manager::InputTypes::Vector2(input::input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));
			input_system.register_input_source(&mouse_device_class_handle, "LeftButton", input::input_manager::InputTypes::Bool(input::input_manager::InputSourceDescription::new(false, false, false, true)));
			input_system.register_input_source(&mouse_device_class_handle, "RightButton", input::input_manager::InputTypes::Bool(input::input_manager::InputSourceDescription::new(false, false, false, true)));
			input_system.register_input_source(&mouse_device_class_handle, "Scroll", input::input_manager::InputTypes::Float(input::input_manager::InputSourceDescription::new(0f32, 0f32, -1f32, 1f32)));

			let keyboard_device_class_handle = input_system.register_device_class("Keyboard");

			input_system.register_input_source(&keyboard_device_class_handle, "W", input::input_manager::InputTypes::Bool(input::input_manager::InputSourceDescription::new(false, false, false, true)));
			input_system.register_input_source(&keyboard_device_class_handle, "S", input::input_manager::InputTypes::Bool(input::input_manager::InputSourceDescription::new(false, false, false, true)));
			input_system.register_input_source(&keyboard_device_class_handle, "A", input::input_manager::InputTypes::Bool(input::input_manager::InputSourceDescription::new(false, false, false, true)));
			input_system.register_input_source(&keyboard_device_class_handle, "D", input::input_manager::InputTypes::Bool(input::input_manager::InputSourceDescription::new(false, false, false, true)));
			input_system.register_input_source(&keyboard_device_class_handle, "Space", input::input_manager::InputTypes::Bool(input::input_manager::InputSourceDescription::new(false, false, false, true)));

			let gamepad_device_class_handle = input_system.register_device_class("Gamepad");

			input_system.register_input_source(&gamepad_device_class_handle, "LeftStick", input::input_manager::InputTypes::Vector2(input::input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));
			input_system.register_input_source(&gamepad_device_class_handle, "RightStick", input::input_manager::InputTypes::Vector2(input::input_manager::InputSourceDescription::new(Vector2::zero(), Vector2::zero(), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))));

			mouse_device_handle = input_system.create_device(&mouse_device_class_handle);
			keyboard_device_handle = input_system.create_device(&keyboard_device_class_handle);
			gamepad_device_handle = input_system.create_device(&gamepad_device_class_handle);
		}

		// let file_tracker_handle = crate::core::spawn(file_tracker::FileTracker::new());

		let renderer_handle = runtime.block_on(spawn_as_child(root_space_handle.clone(), rendering::renderer::Renderer::new_as_system(window_system_handle.clone(), resource_manager.clone())));

		runtime.block_on(spawn_as_child::<Window>(root_space_handle.clone(), Window::new("Main Window", Extent::rectangle(1920, 1080,))));

		let audio_system_handle = runtime.block_on(spawn_as_child(root_space_handle.clone(), DefaultAudioSystem::new_as_system(resource_manager.clone())));

		let physics_system_handle = runtime.block_on(spawn_as_child(root_space_handle.clone(), physics::PhysicsWorld::new_as_system()));

		let anchor_system_handle: EntityHandle<AnchorSystem> = runtime.block_on(spawn_as_child(root_space_handle.clone(), AnchorSystem::new()));

		let tick_handle = runtime.block_on(spawn_as_child(root_space_handle.clone(), Property::new(Time { elapsed: Duration::new(0, 0), delta: Duration::new(0, 0) })));

		#[cfg(debug_assertions)]
		let kill_after = application.get_parameter("kill-after").map(|p| p.value.parse::<u64>().unwrap());

		GraphicsApplication {
			application,
			window_system_handle,

			input_system_handle,
			mouse_device_handle,
			keyboard_device_handle,
			gamepad_device_handle,

			renderer_handle,
			audio_system_handle,
			physics_system_handle,
			anchor_system_handle,
			root_space_handle,

			tick_handle,
			runtime,

			close: false,

			tick_count: 0,
			start_time,
			last_tick_time: std::time::Instant::now(),

			#[cfg(debug_assertions)]
			ttff: std::time::Duration::ZERO,
			#[cfg(debug_assertions)]
			min_frame_time: std::time::Duration::MAX,
			#[cfg(debug_assertions)]
			max_frame_time: std::time::Duration::ZERO,

			#[cfg(debug_assertions)]
			kill_after,
		}
	}

	fn get_parameter(&self, name: &str) -> Option<&Parameter> {
		self.application.get_parameter(name)
	}

	fn get_name(&self) -> String { self.application.get_name() }

	fn tick(&mut self) {
		let now = std::time::Instant::now();
		let dt = now - self.last_tick_time;
		self.last_tick_time = std::time::Instant::now();

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
						ghi::WindowEvents::Key { pressed, key } => {
							let (device_handle, input_source_action, value) = match key {
								ghi::Keys::W => {
									(self.keyboard_device_handle.clone(), input::input_manager::InputSourceAction::Name("Keyboard.W"), input::Value::Bool(pressed))
								},
								ghi::Keys::S => {
									(self.keyboard_device_handle.clone(), input::input_manager::InputSourceAction::Name("Keyboard.S"), input::Value::Bool(pressed))
								},
								ghi::Keys::A => {
									(self.keyboard_device_handle.clone(), input::input_manager::InputSourceAction::Name("Keyboard.A"), input::Value::Bool(pressed))
								},
								ghi::Keys::D => {
									(self.keyboard_device_handle.clone(), input::input_manager::InputSourceAction::Name("Keyboard.D"), input::Value::Bool(pressed))
								},
								ghi::Keys::Space => {
									(self.keyboard_device_handle.clone(), input::input_manager::InputSourceAction::Name("Keyboard.Space"), input::Value::Bool(pressed))
								},
								_ => { return; }
							};

							input_system.record_input_source_action(&device_handle, input_source_action, value);
						},
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

		self.anchor_system_handle.map(|handle| {
			let e = handle.write_sync();
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

		self.tick_count += 1;

		if self.tick_count == 1 {
			self.ttff = self.start_time.elapsed();
		}

		#[cfg(debug_assertions)]
		if let Some(kill_after) = self.kill_after {
			if self.tick_count >= kill_after {
				close = true;
			}
		}

		#[cfg(debug_assertions)]
		{
			self.min_frame_time = self.min_frame_time.min(dt);
			self.max_frame_time = self.max_frame_time.max(dt);
		}

		if close {
			self.close();
		}
	}
}

impl GraphicsApplication {
	/// Flags the application for closing.
	pub fn close(&mut self) {
		self.close = true;

		#[cfg(debug_assertions)]
		log::debug!("Run stats:\n\tAverage frame time: {:#?}\n\tMin frame time: {:#?}\n\tMax frame time: {:#?}\n\tTime to first frame: {:#?}", self.start_time.elapsed().div_f32(self.tick_count as f32), self.min_frame_time, self.max_frame_time, self.ttff);
	}

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
		while !self.close {
			self.tick();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	#[ignore] // Renderer broken.
	fn create_graphics_application() {
		let mut app = GraphicsApplication::new("Test", &[]);

		assert!(app.get_name() == "Test");

		let start_time = std::time::Instant::now();

		while !app.close {
			app.tick();

			if start_time.elapsed().as_secs() > 1 {
				app.close();
			}
		}
	}
}