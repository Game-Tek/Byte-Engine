//! Creates a physics sandbox as an application startup smoke test.
//!
//! This example verifies that the complete application can start and run. It
//! does not verify simulation results.

use byte_engine::application::{Application, Parameter};

fn main() {
	let mut app = byte_engine::application::graphics::GraphicsApplication::new(
		"Sandbox Smoke Test",
		&[
			Parameter::new("kill-after", "60"),
			Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
		],
	);

	// space_handle.spawn(Cube::new(Vector3::new(0.5f32, 0.5f32, 0.5f32)).builder());

	app.do_loop();
}
