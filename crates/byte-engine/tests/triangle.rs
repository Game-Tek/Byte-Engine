//! This is a smoke test that tries to render a 2D triangle to a window.
//! It's purpose is to check if an entire application can be created and run.
//! It does not check if the triangle is rendered correctly, or if the application
//! is working correctly.

use byte_engine::{application::{Application, Parameter}, gameplay::space::Spawn};

#[test]
fn triangle() {
    let mut app = byte_engine::application::GraphicsApplication::new("Triangle Smoke Test", &[Parameter::new("kill-after", "60")]);

	byte_engine::application::graphics_application::default_setup(&mut app);

    let space_handle = app.get_root_space_handle();

	app.do_loop();
}
