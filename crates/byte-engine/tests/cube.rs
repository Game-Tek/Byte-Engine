//! This is a smoke test that tries to render a 3D cube to a window.
//! It's purpose is to check if an entire application can be created and run.
//! It does not check if the cube is rendered correctly, or if the application
//! is working correctly.

use byte_engine::{application::{Application, Parameter}, camera::Camera, gameplay::space::Spawn, rendering::{cube::Cube, point_light::PointLight}};
use maths_rs::vec::Vec3;

#[test]
fn cube() {
    let mut app = byte_engine::application::GraphicsApplication::new("Cube Smoke Test", &[Parameter::new("kill-after", "60")]);

    let space_handle = app.get_root_space_handle();

	space_handle.spawn(Camera::new(Vec3::new(0.0, 0.0, -2.0)));
	space_handle.spawn(PointLight::new(Vec3::new(0f32, 0f32, -2f32), 4500f32));
    space_handle.spawn::<Cube>(Cube::new()); // TODO: fix listeners not being called when entity is not built from an entity builder

	app.do_loop();
}
