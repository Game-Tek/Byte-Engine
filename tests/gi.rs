#![feature(const_mut_refs)]

use byte_engine::{application::Application, Vec3f, core::{orchestrator::{self}, EntityHandle}, rendering::mesh, rendering::point_light::PointLight};
use maths_rs::prelude::{MatTranslate, MatScale,};

#[ignore]
#[test]
fn gi() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter");
	app.initialize(std::env::args());

	let orchestrator_handle = app.get_orchestrator_handle();

	orchestrator::spawn(orchestrator_handle.clone(), byte_engine::camera::Camera::new(Vec3f::new(0.0, 0.5, -2.0),));

	let _floor: EntityHandle<mesh::Mesh> = orchestrator::spawn(orchestrator_handle.clone(), mesh::Mesh::new("Box", "white_solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.0, -0.5, 0.0)) * maths_rs::Mat4f::from_scale(Vec3f::new(5.0, 1.0, 2.5))));
	let _wall: EntityHandle<mesh::Mesh> = orchestrator::spawn(orchestrator_handle.clone(), mesh::Mesh::new("Box", "white_solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 1.0, 1.0)) * maths_rs::Mat4f::from_scale(Vec3f::new(5.0, 2.0, 1.0))));
	let _a: EntityHandle<mesh::Mesh> = orchestrator::spawn(orchestrator_handle.clone(), mesh::Mesh::new("Suzanne", "white_solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.5, 0.0)) * maths_rs::Mat4f::from_scale(Vec3f::new(0.4, 0.4, 0.4))));
	let _b: EntityHandle<mesh::Mesh> = orchestrator::spawn(orchestrator_handle.clone(), mesh::Mesh::new("Box", "red_solid", maths_rs::Mat4f::from_translation(Vec3f::new(-0.6, 0.17, -0.1)) * maths_rs::Mat4f::from_scale(Vec3f::new(0.34, 0.34, 0.34))));
	let _c: EntityHandle<mesh::Mesh> = orchestrator::spawn(orchestrator_handle.clone(), mesh::Mesh::new("Box", "green_solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.5, 0.13, -0.3)) * maths_rs::Mat4f::from_scale(Vec3f::new(0.26, 0.26, 0.26))));

	let _sun: EntityHandle<PointLight> = orchestrator::spawn(orchestrator_handle.clone(), PointLight::new(Vec3f::new(0.0, 2.5, -1.5), 4500.0));

	app.do_loop();

	app.deinitialize();
}