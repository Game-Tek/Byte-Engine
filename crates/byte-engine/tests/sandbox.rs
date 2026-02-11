//! This is a smoke test that creates a sandbox environment for physics.
//! It's purpose is to check if an entire application can be created and run.
//! It does not check if the application is working correctly.

use byte_engine::application::{Application, Parameter};

#[test]
fn sandbox() {
    let mut app = byte_engine::application::GraphicsApplication::new("Sandbox Smoke Test", &[
		Parameter::new("kill-after", "60"),
		Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
	]);

    // space_handle.spawn(Cube::new(Vector3::new(0.5f32, 0.5f32, 0.5f32)).builder());

	app.do_loop();
}
