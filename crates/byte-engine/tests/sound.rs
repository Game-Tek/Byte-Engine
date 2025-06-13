//! This is a smoke test that tries to play a sound on speakers.
//! It's purpose is to check if an entire application can be created and run.
//! It does not check if the sound is rendered correctly, or if the application
//! is working correctly.

use byte_engine::{application::{Application, Parameter}, audio::synthesizer::Synthesizer, core::Entity, gameplay::space::Spawner};

#[test]
fn sound() {
    let mut app = byte_engine::application::GraphicsApplication::new("Sound Smoke Test", &[Parameter::new("kill-after", "60")]);

    let space_handle = app.get_root_space_handle();

	space_handle.spawn(Synthesizer::new().builder());

	app.do_loop();
}
