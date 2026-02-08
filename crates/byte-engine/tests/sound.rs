//! This is a smoke test that tries to play a sound on speakers.
//! It's purpose is to check if an entire application can be created and run.
//! It does not check if the sound is rendered correctly, or if the application
//! is working correctly.

use byte_engine::{application::{Application, Parameter}, audio::synthesizer::Synthesizer, core::Entity, gameplay::space::Spawner};

#[test]
fn sound() {
    let mut app = byte_engine::application::GraphicsApplication::new("Sound Smoke Test", &[
		Parameter::new("kill-after", "60"),
		Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
	]);

    let space_handle = app.get_root_space_handle();

	space_handle.spawn(TestSynthesizer{}.builder());

	app.do_loop();
}

struct TestSynthesizer;

impl Synthesizer for TestSynthesizer {
	fn render<'a>(&self, _current_sample: u32, buffer: &'a mut [f32]) -> &'a [f32] {
		buffer.fill(0.0);
		buffer
	}
}

impl Entity for TestSynthesizer {}
