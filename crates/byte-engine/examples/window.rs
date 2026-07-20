//! Runs an application with one user-created window as a startup smoke test.
//!
//! This example verifies only that the application can start and run.

use byte_engine::application::{Application, Parameter};

fn main() {
	let mut app = byte_engine::application::graphics::GraphicsApplication::new(
		"Window Smoke Test",
		&[
			Parameter::new("kill-after", "60"),
			Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
		],
	);

	byte_engine::application::graphics::setup_default_window(&mut app);

	app.do_loop();
}
