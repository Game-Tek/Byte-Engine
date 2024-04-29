#![feature(const_mut_refs)]
#![feature(async_closure)]
#![feature(closure_lifetime_binder)]

use core::EntityHandle;
use byte_engine::{application::Application, rendering::{mesh, point_light::PointLight}, Vector3};
use maths_rs::prelude::{MatTranslate, MatScale};

#[ignore]
#[test]
fn load() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter");

	let _ = app.get_audio_system_handle().clone();

	app.initialize(std::env::args());

	let space_handle = app.get_root_space_handle();

	// let lookaround_action_handle = core::spawn_as_child(space_handle.clone(), input::Action::new("Lookaround", &[
	// 	input::ActionBindingDescription::new("Mouse.Position").mapped(input::Value::Vector3(Vector3::new(1f32, 1f32, 1f32)), input::Function::Sphere),
	// 	input::ActionBindingDescription::new("Gamepad.RightStick"),
	// ],));

	// let trigger_action = core::spawn_as_child(space_handle.clone(), input::Action::new("Trigger", &[
	// 	input::ActionBindingDescription::new("Mouse.LeftButton"),
	// 	input::ActionBindingDescription::new("Gamepad.RightTrigger"),
	// ],));

	let scale = maths_rs::Mat4f::from_scale(Vector3::new(0.1, 0.1, 0.1));
	
	let _: EntityHandle<mesh::Mesh> = core::spawn_as_child(space_handle.clone(), mesh::Mesh::new("Box", "Solid", maths_rs::Mat4f::from_translation(Vector3::new(0.0, 0.0, 2.0)) * scale));
	
	let _sun: EntityHandle<PointLight> = core::spawn_as_child(space_handle.clone(), PointLight::new(Vector3::new(0.0, 2.5, -1.5), 4500.0));

	let _ = core::spawn_as_child(space_handle.clone(), byte_engine::camera::Camera::new(Vector3::new(0.0, 0.0, 0.0)));

	app.do_loop();
}