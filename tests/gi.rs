#![feature(const_mut_refs)]

use byte_engine::{application::Application, Vec3f, orchestrator::EntityHandle, render_domain::Mesh, rendering::point_light::PointLight};
use maths_rs::prelude::{MatTranslate, MatScale,};

#[ignore]
#[test]
fn gi() {
	let mut app = byte_engine::application::GraphicsApplication::new("Gallery Shooter");
	app.initialize(std::env::args());

	let orchestrator = app.get_mut_orchestrator();

	orchestrator.spawn(byte_engine::camera::Camera {
		position: Vec3f::new(0.0, 0.5, -2.0),
		direction: Vec3f::new(0.0, -0.2, 0.85),
		fov: 90.0,
		aspect_ratio: 1.0,
		aperture: 0.0,
		focus_distance: 0.0,
	});

	let _floor: EntityHandle<Mesh> = orchestrator.spawn(Mesh{ resource_id: "Box", material_id: "white_solid", transform: maths_rs::Mat4f::from_translation(Vec3f::new(0.0, -0.5, 0.0)), });
	let _a: EntityHandle<Mesh> = orchestrator.spawn(Mesh{ resource_id: "Box", material_id: "white_solid", transform: maths_rs::Mat4f::from_translation(Vec3f::new(0.0, 0.25, 2.0)) * maths_rs::Mat4f::from_scale(Vec3f::new(0.5, 0.5, 0.5)), });
	let _b: EntityHandle<Mesh> = orchestrator.spawn(Mesh{ resource_id: "Box", material_id: "red_solid", transform: maths_rs::Mat4f::from_translation(Vec3f::new(-0.8, 0.17, 1.7)) * maths_rs::Mat4f::from_scale(Vec3f::new(0.34, 0.34, 0.34)), });
	let _c: EntityHandle<Mesh> = orchestrator.spawn(Mesh{ resource_id: "Box", material_id: "green_solid", transform: maths_rs::Mat4f::from_translation(Vec3f::new(0.7, 0.13, 1.8)) * maths_rs::Mat4f::from_scale(Vec3f::new(0.26, 0.26, 0.26)), });

	let _sun: EntityHandle<PointLight> = orchestrator.spawn(PointLight::new(Vec3f::new(-1.0, 1.5, 0.0), 4500.0));

	app.do_loop();

	app.deinitialize();
}