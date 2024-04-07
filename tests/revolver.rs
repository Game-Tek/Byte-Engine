#![feature(const_mut_refs)]
#![feature(async_closure)]
#![feature(closure_lifetime_binder)]

use core::{self, EntityHandle};
use byte_engine::{application::Application, camera, rendering::{directional_light, mesh}, Vector3};
use maths_rs::prelude::MatTranslate;

#[ignore]
#[test]
fn  revolver() {
	let mut app = byte_engine::application::GraphicsApplication::new("Revolver");

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
	
	let _: EntityHandle<camera::Camera> = core::spawn_as_child(space_handle.clone(), camera::Camera::new(Vector3::new(0.0, 0.0, 0.0),));
	let _: EntityHandle<directional_light::DirectionalLight> = core::spawn_as_child(space_handle.clone(), directional_light::DirectionalLight::new(Vector3::new(0.0, 0.0, 1.0), 4000f32));
	let _: EntityHandle<mesh::Mesh> = core::spawn_as_child(space_handle.clone(), mesh::Mesh::new("Revolver.glb", "pbr.json", maths_rs::Mat4f::from_translation(Vector3::new(0.0, 0.0, 0.4))));

	app.do_loop();

	app.deinitialize();
}