#![feature(const_mut_refs)]

use byte_engine::camera::Camera;
use byte_engine::rendering::directional_light::DirectionalLight;
use byte_engine::rendering::mesh::Mesh;
use byte_engine::rendering::point_light::PointLight;
use byte_engine::{application::Application, Vector3};
use byte_engine::core::{self, EntityHandle};
use maths_rs::prelude::{MatTranslate, MatScale,};

fn main() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter");
	app.initialize(std::env::args());

	log::set_max_level(log::LevelFilter::Warn);

	let space_handle = app.get_root_space_handle();

	core::spawn_as_child(space_handle.clone(), Camera::new(Vector3::new(0.0, 0.5, -2.0),));

	let _floor: EntityHandle<Mesh> = core::spawn_as_child(space_handle.clone(), Mesh::new("Box", "white_solid", maths_rs::Mat4f::from_translation(Vector3::new(0.0, -0.5, 1.0)) * maths_rs::Mat4f::from_scale(Vector3::new(5.0, 1.0, 5.0))));
	let _wall: EntityHandle<Mesh> = core::spawn_as_child(space_handle.clone(), Mesh::new("Box", "white_solid", maths_rs::Mat4f::from_translation(Vector3::new(0.0, -1.0, 1.0)) * maths_rs::Mat4f::from_scale(Vector3::new(5.0, 10.0, 1.0))));
	let _a: EntityHandle<Mesh> = core::spawn_as_child(space_handle.clone(), Mesh::new("Suzanne", "white_solid", maths_rs::Mat4f::from_translation(Vector3::new(0.0, 0.5, 0.0)) * maths_rs::Mat4f::from_scale(Vector3::new(0.4, 0.4, 0.4))));
	let _b: EntityHandle<Mesh> = core::spawn_as_child(space_handle.clone(), Mesh::new("Box", "red_solid", maths_rs::Mat4f::from_translation(Vector3::new(-0.6, 0.17, -0.1)) * maths_rs::Mat4f::from_scale(Vector3::new(0.34, 0.34, 0.34))));
	let _c: EntityHandle<Mesh> = core::spawn_as_child(space_handle.clone(), Mesh::new("Box", "green_solid", maths_rs::Mat4f::from_translation(Vector3::new(0.5, 0.13, -0.3)) * maths_rs::Mat4f::from_scale(Vector3::new(0.26, 0.26, 0.26))));

	let _sun: EntityHandle<DirectionalLight> = core::spawn_as_child(space_handle.clone(), DirectionalLight::new(maths_rs::normalize(Vector3::new(1.0, 1.0, -1.0)), 4500.0));
	let _helper_light: EntityHandle<PointLight> = core::spawn_as_child(space_handle.clone(), PointLight::new(Vector3::new(-2.0, 0.5, -1.0f32), 4500.0));
	let _helper_light: EntityHandle<PointLight> = core::spawn_as_child(space_handle.clone(), PointLight::new(Vector3::new(2.0, 0.5, -1.0f32), 4500.0));

	app.do_loop();

	app.deinitialize();
}