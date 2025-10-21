//! This is a smoke test that tries runs the application with nothing but a window created by the user.
//! It's purpose is to check if an entire application can be created and run.
//! It does not check if the application is working correctly, just that it can be created and run.

use byte_engine::application::{Application, Parameter};

#[test]
fn window() {
    let mut app = byte_engine::application::GraphicsApplication::new("Window Smoke Test", &[
        Parameter::new("kill-after", "60"),
        Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
    ]);

    byte_engine::application::graphics_application::setup_default_window(&mut app);

	app.do_loop();
}
