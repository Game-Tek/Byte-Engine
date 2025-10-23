//! This is a smoke test that tries to render a 3D cube to a window.
//! It's purpose is to check if an entire application can be created and run.
//! It does not check if the cube is rendered correctly, or if the application
//! is working correctly.

use byte_engine::{application::{Application, Parameter}, camera::Camera, core::Entity, gameplay::space::Spawner, rendering::lights::PointLight};
use math::Vector3;

#[test]
fn cube() {
    let mut app = byte_engine::application::GraphicsApplication::new("Cube Smoke Test", &[
		Parameter::new("kill-after", "60"),
		Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
	]);

    let space_handle = app.get_root_space_handle();

	space_handle.spawn(Camera::new().builder());
	space_handle.spawn(PointLight::new(Vector3::new(0f32, 0f32, -2f32), 4500f32).builder());

	app.do_loop();
}
