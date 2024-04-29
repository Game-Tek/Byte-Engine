#![feature(const_mut_refs)]
#![feature(async_closure)]
#![feature(closure_lifetime_binder)]

use core::{self, EntityHandle};
use std::sync::{Arc, Mutex};
use byte_engine::{application::Application, camera, input, rendering::{directional_light, mesh, point_light}, Vector3};
use maths_rs::{exp, prelude::MatTranslate};
use serde::de;

#[ignore]
#[test]
fn  revolver() {
	let mut app = byte_engine::application::GraphicsApplication::new("Revolver");

	app.initialize(std::env::args());

	let space_handle = app.get_root_space_handle();

	let lookaround_action_handle = core::spawn_as_child(space_handle.clone(), input::Action::<Vector3>::new("Lookaround", &[
		input::ActionBindingDescription::new("Mouse.Position").mapped(input::Value::Vector3(Vector3::new(1f32, 1f32, 1f32)), input::Function::Sphere),
		input::ActionBindingDescription::new("Gamepad.RightStick"),
	],));

	let zoom_action_handle = core::spawn_as_child(space_handle.clone(), input::Action::<f32>::new("Zoom", &[
		input::ActionBindingDescription::new("Mouse.Scroll"),
	],));

	// let trigger_action = core::spawn_as_child(space_handle.clone(), input::Action::new("Trigger", &[
	// 	input::ActionBindingDescription::new("Mouse.LeftButton"),
	// 	input::ActionBindingDescription::new("Gamepad.RightTrigger"),
	// ],));
	
	let camera: EntityHandle<camera::Camera> = core::spawn_as_child(space_handle.clone(), camera::Camera::new(Vector3::new(0.0, 0.0, -0.25),));
	let _: EntityHandle<directional_light::DirectionalLight> = core::spawn_as_child(space_handle.clone(), directional_light::DirectionalLight::new(Vector3::new(0.0, 0.0, 1.0), 4000f32));
	let _: EntityHandle<point_light::PointLight> = core::spawn_as_child(space_handle.clone(), point_light::PointLight::new(Vector3::new(0.3, 0.3, 0.25), 2500f32));
	let _: EntityHandle<point_light::PointLight> = core::spawn_as_child(space_handle.clone(), point_light::PointLight::new(Vector3::new(-0.3, 0.3, 0.45), 6500f32));
	let mesh: EntityHandle<mesh::Mesh> = core::spawn_as_child(space_handle.clone(), mesh::Mesh::new("Revolver.glb", "pbr.json", maths_rs::Mat4f::from_translation(Vector3::new(0.018, 0.0275, 0.0))));

	struct Animation {
		value: Vector3,
		speed: f32,
	}

	impl Animation {
		fn new(value: Vector3, speed: f32) -> Self {
			Self {
				value,
				speed,
			}
		}

		fn evaluate(&mut self, target: Vector3, dt: f32) -> Vector3 {
			self.value += (target - self.value) * (1f32 - exp(-dt * self.speed));
			self.value
		}
	}

	let mut target = Arc::new(Mutex::new(Vector3::new(0f32, 0f32, 1f32)));
	let mut animation = Animation::new(*target.lock().unwrap(), 2f32);

	{
		let target = Arc::clone(&target);

		app.get_tick_handle().sync_get_mut(|tick| {
			tick.event().add(move |dt| {
				let value = animation.evaluate(*target.lock().unwrap(), dt.as_secs_f32());
				mesh.sync_get_mut(move |mesh| {
					mesh.set_orientation(value);
				});
			});
		});
	}

	{
		let target = Arc::clone(&target);

		lookaround_action_handle.sync_get_mut(move |action| {
			action.value_mut().add(move |r| {
				*target.lock().unwrap() = *r;
			});
		});
	}


	zoom_action_handle.sync_get_mut(|action| {
		action.value_mut().add(move |r| {
			camera.sync_get_mut(|camera| {
				camera.set_fov(camera.get_fov() + -r * 2f32);
			});
		});
	});

	app.do_loop();
}