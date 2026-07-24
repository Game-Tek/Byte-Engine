//! Renders a 2D triangle as an application startup smoke test.
//!
//! This example verifies that the complete application can start and run. It
//! does not verify the rendered image.

use byte_engine::application::{Application, Parameter};

fn main() {
	let mut app = byte_engine::application::graphics::GraphicsApplication::new(
		"Triangle Smoke Test",
		&[
			Parameter::new("kill-after", "60"),
			Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
		],
	);

	byte_engine::application::graphics::default_setup(&mut app);

	app.do_loop();
}
