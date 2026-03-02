//! This is a smoke test that tries to render a 2D triangle to a window.
//! It's purpose is to check if an entire application can be created and run.
//! It does not check if the triangle is rendered correctly, or if the application
//! is working correctly.

use byte_engine::application::{Application, Parameter};

#[test]
#[ignore]
fn triangle() {
	let mut app = byte_engine::application::GraphicsApplication::new(
		"Triangle Smoke Test",
		&[
			Parameter::new("kill-after", "60"),
			Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
		],
	);

	byte_engine::application::graphics_application::default_setup(&mut app);

	app.do_loop();
}
