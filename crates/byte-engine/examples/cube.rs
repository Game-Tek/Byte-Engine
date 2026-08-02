//! Renders a 3D cube as an application startup smoke test.
//!
//! This example verifies that the complete application can start and run. It
//! does not verify the rendered image.

use byte_engine::application::{Application, Parameter};

fn main() {
	let mut app = byte_engine::application::graphics::GraphicsApplication::new(
		"Cube Smoke Test",
		&[
			Parameter::new("kill-after", "60"),
			Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
		],
	);

	// space_handle.spawn(Camera::new().builder());
	// Add a `PointLight` through the world's light factory when this example needs direct lighting.

	app.do_loop();
}
