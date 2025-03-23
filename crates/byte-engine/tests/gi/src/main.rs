#![feature(const_mut_refs)]

use byte_engine::camera::Camera;
use byte_engine::rendering::directional_light::DirectionalLight;
use byte_engine::rendering::mesh::{self, Mesh};
use byte_engine::rendering::point_light::PointLight;
use byte_engine::{application::Application, Vector3};
use byte_engine::core::{self, EntityHandle};

fn main() {
	let mut app = byte_engine::application::GraphicsApplication::new("GI");
	app.initialize(std::env::args());

	log::set_max_level(log::LevelFilter::Warn);

	let space_handle = app.get_root_space_handle();
	
	let runtime = app.get_runtime();
	
	runtime.block_on(async move {
		core::spawn_as_child(space_handle.clone(), Camera::new(Vector3::new(0.0, 0.5, -2.0),)).await;

		let _floor: EntityHandle<Mesh> = crate::core::spawn_as_child(space_handle.clone(), Mesh::new("Box.glb", mesh::Transform::default().position(Vector3::new(0.0, -0.5, 1.0)).scale(Vector3::new(5.0, 1.0, 5.0)))).await;
		let _wall: EntityHandle<Mesh> = crate::core::spawn_as_child(space_handle.clone(), Mesh::new("Box.glb", mesh::Transform::default().position(Vector3::new(0.0, -1.0, 1.0)).scale(Vector3::new(5.0, 10.0, 1.0)))).await;
		let _a: EntityHandle<Mesh> = crate::core::spawn_as_child(space_handle.clone(), Mesh::new("Suzanne.gltf", mesh::Transform::default().position(Vector3::new(0.0, 0.5, 0.0)).scale(Vector3::new(0.4, 0.4, 0.4)))).await;
		let _b: EntityHandle<Mesh> = crate::core::spawn_as_child(space_handle.clone(), Mesh::new("Box.glb", mesh::Transform::default().position(Vector3::new(-0.6, 0.17, -0.1)).scale(Vector3::new(0.34, 0.34, 0.34)))).await;
		let _c: EntityHandle<Mesh> = crate::core::spawn_as_child(space_handle.clone(), Mesh::new("Box.glb", mesh::Transform::default().position(Vector3::new(0.5, 0.13, -0.3)).scale(Vector3::new(0.26, 0.26, 0.26)))).await;
	
		// let _wall: EntityHandle<Mesh> = crate::core::spawn_as_child(space_handle.clone(), Mesh::new("mountainside_2k.gltf", "white_solid.json", mesh::Transform::default().position(Vector3::new(0.0, -1.0, 5.0))));
	
		let _sun: EntityHandle<DirectionalLight> = crate::core::spawn_as_child(space_handle.clone(), DirectionalLight::new(maths_rs::normalize(Vector3::new(-1.0, -1.0, 1.0)), 4500.0)).await;
		let _helper_light: EntityHandle<PointLight> = crate::core::spawn_as_child(space_handle.clone(), PointLight::new(Vector3::new(-2.0, 0.5, -1.0f32), 4500.0)).await;
		let _helper_light: EntityHandle<PointLight> = crate::core::spawn_as_child(space_handle.clone(), PointLight::new(Vector3::new(2.0, 0.5, -1.0f32), 4500.0)).await;
	});

	app.do_loop();
}