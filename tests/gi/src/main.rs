#![feature(const_mut_refs)]

use std::ops::DerefMut;

use byte_engine::{application::Application, Vec3f, core::{self, EntityHandle}, rendering::{directional_light::DirectionalLight, mesh}, rendering::point_light::PointLight,};
use maths_rs::prelude::{MatTranslate, MatScale,};

fn main() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter");
	app.initialize(std::env::args());

	log::set_max_level(log::LevelFilter::Warn);

	let space_handle = app.get_root_space_handle();

	let mut space = space_handle.write_sync();

	core::spawn_in_domain(space.deref_mut(), byte_engine::camera::Camera::new(Vec3f::new(0.0, 0.5, -2.0),));

	let _floor: EntityHandle<mesh::Mesh> = core::spawn_in_domain(space.deref_mut(), mesh::Mesh::new("Box", "white_solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.0, -0.5, 1.0)) * maths_rs::Mat4f::from_scale(Vec3f::new(5.0, 1.0, 5.0))));
	let _wall: EntityHandle<mesh::Mesh> = core::spawn_in_domain(space.deref_mut(), mesh::Mesh::new("Box", "white_solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.0, -1.0, 1.0)) * maths_rs::Mat4f::from_scale(Vec3f::new(5.0, 10.0, 1.0))));
	let _a: EntityHandle<mesh::Mesh> = core::spawn_in_domain(space.deref_mut(), mesh::Mesh::new("Suzanne", "white_solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.5, 0.0)) * maths_rs::Mat4f::from_scale(Vec3f::new(0.4, 0.4, 0.4))));
	let _b: EntityHandle<mesh::Mesh> = core::spawn_in_domain(space.deref_mut(), mesh::Mesh::new("Box", "red_solid", maths_rs::Mat4f::from_translation(Vec3f::new(-0.6, 0.17, -0.1)) * maths_rs::Mat4f::from_scale(Vec3f::new(0.34, 0.34, 0.34))));
	let _c: EntityHandle<mesh::Mesh> = core::spawn_in_domain(space.deref_mut(), mesh::Mesh::new("Box", "green_solid", maths_rs::Mat4f::from_translation(Vec3f::new(0.5, 0.13, -0.3)) * maths_rs::Mat4f::from_scale(Vec3f::new(0.26, 0.26, 0.26))));

	let _sun: EntityHandle<DirectionalLight> = core::spawn_in_domain(space.deref_mut(), DirectionalLight::new(maths_rs::normalize(Vec3f::new(1.0, 1.0, -1.0)), 4500.0));
	let _helper_light: EntityHandle<PointLight> = core::spawn_in_domain(space.deref_mut(), PointLight::new(Vec3f::new(-2.0, 0.5, -1.0f32), 4500.0));
	let _helper_light: EntityHandle<PointLight> = core::spawn_in_domain(space.deref_mut(), PointLight::new(Vec3f::new(2.0, 0.5, -1.0f32), 4500.0));

	app.do_loop();

	app.deinitialize();
}