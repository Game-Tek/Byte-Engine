use crate::{core::{property::Property, spawn, spawn_as_child, EntityHandle}, gameplay::space::Spawn, input::{input_trigger, utils::{register_gamepad_device_class, register_keyboard_device_class, register_mouse_device_class}}, rendering::{aces_tonemap_render_pass::AcesToneMapPass, visibility_model::render_domain::VisibilityWorldRenderDomain}};
use std::time::Duration;

use maths_rs::num::Base;
use resource_management::{asset::{asset_manager::AssetManager, audio_asset_handler::AudioAssetHandler, image_asset_handler::ImageAssetHandler, material_asset_handler::{MaterialAssetHandler, ProgramGenerator}, mesh_asset_handler::MeshAssetHandler}, material::Material, resource::resource_manager::ResourceManager};
use utils::Extent;

use crate::{audio::audio_system::{AudioSystem, DefaultAudioSystem}, gameplay::{anchor::AnchorSystem, space::Space}, input, physics, rendering::{self, common_shader_generator::CommonShaderGenerator, renderer::Renderer, visibility_shader_generator::VisibilityShaderGenerator}, window_system::{self, Window}, Vector2};

use super::{application::{Application, BaseApplication}, Parameter, Time};

/// A graphics application is the base for all applications that use the graphics functionality of the engine.
/// It uses the orchestrated application as a base and adds rendering and windowing functionality.
pub struct GraphicsApplication {
	application: BaseApplication,

	tick_count: u64,
	start_time: std::time::Instant,
	last_tick_time: std::time::Instant,

	close: bool,

	window_system_handle: EntityHandle<window_system::WindowSystem>,

	input_system_handle: EntityHandle<input::InputManager>,

	resource_manager: EntityHandle<ResourceManager>,
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

		let root_space_handle: EntityHandle<Space> = spawn(Space::new());

		let resources_path: std::path::PathBuf = application.get_parameter("resources-path").map(|p| p.value.clone()).unwrap_or_else(|| "resources".into()).into();

		let resource_manager = spawn(ResourceManager::new(resources_path.clone()));

		let window_system_handle = root_space_handle.spawn(window_system::WindowSystem::new_as_system());
		let input_system_handle = root_space_handle.spawn(input::InputManager::new_as_system());		
		let renderer_handle = root_space_handle.spawn(rendering::renderer::Renderer::new_as_system(window_system_handle.clone(), resource_manager.clone()));
		let audio_system_handle = root_space_handle.spawn(DefaultAudioSystem::new_as_system(resource_manager.clone()));
		let physics_system_handle = root_space_handle.spawn(physics::PhysicsWorld::new_as_system());

		let anchor_system_handle = root_space_handle.spawn(AnchorSystem::new());

		let tick_handle = root_space_handle.spawn(Property::new(Time { elapsed: Duration::new(0, 0), delta: Duration::new(0, 0) }));

		#[cfg(debug_assertions)]
		let kill_after = application.get_parameter("kill-after").map(|p| p.value.parse::<u64>().unwrap());

		GraphicsApplication {
			application,
			window_system_handle,

			input_system_handle,

			renderer_handle,
			resource_manager,
			audio_system_handle,
			physics_system_handle,
			anchor_system_handle,
			root_space_handle,

			tick_handle,

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

	fn get_name(&self) -> &str { self.application.get_name() }

	fn tick(&mut self) {
		let now = std::time::Instant::now();
		let dt = now - self.last_tick_time;
		self.last_tick_time = std::time::Instant::now();

		let mut close = false;

		{
			let window_system = self.window_system_handle.get_lock();
			let mut window_system = window_system.write();

			{
				let mut input_system = self.input_system_handle.write();

				let mouse_device_handle = input_system.get_devices_by_class_name("Mouse").unwrap().get(0).unwrap().clone();
				let keyboard_device_handle = input_system.get_devices_by_class_name("Keyboard").unwrap().get(0).unwrap().clone();

				window_system.update_windows(|_, event| {
					match event {
						ghi::WindowEvents::Close => { close = true },
						ghi::WindowEvents::Button { pressed, button } => {
							match button {
								ghi::MouseKeys::Left => {
									input_system.record_trigger_value_for_device(mouse_device_handle, input::input_manager::TriggerReference::Name("Mouse.LeftButton"), input::Value::Bool(pressed));
								},
								ghi::MouseKeys::Right => {
									input_system.record_trigger_value_for_device(mouse_device_handle, input::input_manager::TriggerReference::Name("Mouse.RightButton"), input::Value::Bool(pressed));
								},
								ghi::MouseKeys::ScrollUp => {
									input_system.record_trigger_value_for_device(mouse_device_handle, input::input_manager::TriggerReference::Name("Mouse.Scroll"), input::Value::Float(1f32));
								},
								ghi::MouseKeys::ScrollDown => {
									input_system.record_trigger_value_for_device(mouse_device_handle, input::input_manager::TriggerReference::Name("Mouse.Scroll"), input::Value::Float(-1f32));
								},
								_ => { }
							}
						},
						ghi::WindowEvents::MouseMove { x, y, time: _ } => {
							let vec = Vector2::new(x, y);
							input_system.record_trigger_value_for_device(mouse_device_handle, input::input_manager::TriggerReference::Name("Mouse.Position"), input::Value::Vector2(vec));
						},
						ghi::WindowEvents::Resize { width, height } => {
						}
						ghi::WindowEvents::Key { pressed, key } => {
							let (device_handle, input_source_action, value) = match key {
								ghi::Keys::W => {
									(keyboard_device_handle, input::input_manager::TriggerReference::Name("Keyboard.W"), input::Value::Bool(pressed))
								},
								ghi::Keys::S => {
									(keyboard_device_handle, input::input_manager::TriggerReference::Name("Keyboard.S"), input::Value::Bool(pressed))
								},
								ghi::Keys::A => {
									(keyboard_device_handle, input::input_manager::TriggerReference::Name("Keyboard.A"), input::Value::Bool(pressed))
								},
								ghi::Keys::D => {
									(keyboard_device_handle, input::input_manager::TriggerReference::Name("Keyboard.D"), input::Value::Bool(pressed))
								},
								ghi::Keys::Space => {
									(keyboard_device_handle, input::input_manager::TriggerReference::Name("Keyboard.Space"), input::Value::Bool(pressed))
								},
								ghi::Keys::Escape => {
									close = true;
									(keyboard_device_handle, input::input_manager::TriggerReference::Name("Keyboard.Escape"), input::Value::Bool(pressed))
								}
								_ => { return; }
							};

							input_system.record_trigger_value_for_device(device_handle, input_source_action, value);
						},
						_ => { }
					}
				});
			}
		}

		let time = Time { elapsed: self.start_time.elapsed(), delta: dt };

		self.tick_handle.get_mut(move |tick| {
			tick.set(|_| time);
		});

		self.input_system_handle.map(|handle| {
			let mut e = handle.write();
			e.update();
		});

		self.anchor_system_handle.map(|handle| {
			let e = handle.write();
			e.update();
		});

		self.physics_system_handle.map(move |handle| {
			let mut e = handle.write();
			e.update(time);
		});

		self.renderer_handle.map(|handle| {
			let mut e = handle.write();
			e.render();
		});

		self.audio_system_handle.map(|handle| {
			let mut e = handle.write();
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

	pub fn get_renderer_handle(&self) -> &EntityHandle<Renderer> {
		&self.renderer_handle
	}
	
	pub fn do_loop(&mut self) {
		while !self.close {
			self.tick();
		}
	}
}

/// Performs a default setup of the application.
/// This includes setting up mouse, keyboard and gamepad input devices,
/// as well as setting up the resource manager with default asset handlers.
/// It also sets up the renderer with a default render pipeline.
/// The default render pipeline includes a visibility shader generator and a PBR visibility shading render pipeline.
/// The default render pipeline also includes a tone mapping pass.
/// A window is created with the application name.
pub fn default_setup(application: &mut GraphicsApplication) {
	{
		let generator = {
			let common_shader_generator = CommonShaderGenerator::new();
			let visibility_shader_generation = VisibilityShaderGenerator::new();
			visibility_shader_generation
		};

		setup_default_resource_and_asset_management(application, generator);
	}

	setup_default_input(application);

	setup_pbr_visibility_shading_render_pipeline(application);

	setup_default_window(application);
}

/// Creates a new window under the root space with the application name and an extent of 1920x1080.
pub fn setup_default_window(application: &mut GraphicsApplication) {
	let root_space_handle = application.get_root_space_handle();
	root_space_handle.spawn(Window::new(application.get_name(), Extent::rectangle(1920, 1080,)));
}

/// Sets up the default resource and asset management for the application.
/// This includes setting up the resource manager with default asset handlers.
/// The default asset handlers include:
/// - MaterialAssetHandler
/// - MeshAssetHandler
/// - ImageAssetHandler
/// - AudioAssetHandler
/// 
/// The default material asset handler is set up with a shader generator.
/// The shader generator is passed as a parameter to this function.
/// The resources folder path is taken from the `resources-path` parameter and defaults to `resources`.
/// The assets folder path is taken from the `assets-path` parameter and defaults to `assets`.
pub fn setup_default_resource_and_asset_management(application: &mut GraphicsApplication, generator: impl ProgramGenerator + 'static) {
	let mut resource_manager = application.resource_manager.write();

	let resources_path: std::path::PathBuf = application.get_parameter("resources-path").map(|p| p.value.clone()).unwrap_or_else(|| "resources".into()).into();
	let assets_path: std::path::PathBuf = application.get_parameter("assets-path").map(|p| p.value.clone()).unwrap_or_else(|| "assets".into()).into();

	let mut asset_manager = AssetManager::new(assets_path, resources_path);

	let mut material_asset_handler = MaterialAssetHandler::new();
	material_asset_handler.set_shader_generator(generator);
	asset_manager.add_asset_handler(material_asset_handler);

	asset_manager.add_asset_handler(MeshAssetHandler::new());
	asset_manager.add_asset_handler(ImageAssetHandler::new());
	asset_manager.add_asset_handler(AudioAssetHandler::new());

	resource_manager.set_asset_manager(asset_manager);
}

/// Sets up a default input system for the application.
/// This includes setting up mouse, keyboard and gamepad input devices.
pub fn setup_default_input(application: &mut GraphicsApplication) {
	let mut input_system = application.input_system_handle.write();

	let mouse_device_class_handle = register_mouse_device_class(&mut input_system);
	let keyboard_device_class_handle = register_keyboard_device_class(&mut input_system);
	let gamepad_device_class_handle = register_gamepad_device_class(&mut input_system);

	input_system.create_device(&mouse_device_class_handle);
	input_system.create_device(&keyboard_device_class_handle);
	input_system.create_device(&gamepad_device_class_handle);
}

pub fn setup_pbr_visibility_shading_render_pipeline(application: &mut GraphicsApplication) {
	let mut renderer = application.renderer_handle.write();

	renderer.add_render_pass::<VisibilityWorldRenderDomain>(application.root_space_handle.clone());
	renderer.add_render_pass::<AcesToneMapPass>(application.root_space_handle.clone());
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
