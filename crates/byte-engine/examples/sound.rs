//! Plays a sound through the default audio output as an application startup smoke test.
//!
//! This example verifies that the complete application can start, publish a
//! procedural [`Generator`], and run. It does not verify the generated audio.

use byte_engine::{
	application::Parameter,
	audio::{Generator, PlaybackSettings, PlaybackState},
};

fn main() {
	let mut app = byte_engine::application::graphics::GraphicsApplication::new(
		"Sound Smoke Test",
		&[
			Parameter::new("kill-after", "60"),
			Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
		],
	);

	app.generator_factory().create(Box::new(TestTone));

	app.do_loop();
}

/// The `TestTone` struct plays a one-second 440 Hz sine tone for the smoke test.
#[derive(Clone)]
struct TestTone;

impl TestTone {
	const PITCH: f64 = 440.0;
	const GAIN: f32 = 1.0;
	const DURATION_SECONDS: u64 = 1;
}

impl Generator for TestTone {
	fn render<'a>(&self, settings: PlaybackSettings, state: PlaybackState, buffer: &'a mut [f32]) -> Option<&'a [f32]> {
		let tau = std::f64::consts::TAU;
		let phase_step = tau * Self::PITCH / f64::from(settings.sample_rate);
		let mut phase = (state.current_sample as f64 * phase_step).rem_euclid(tau);

		// Mix into the buffer, because other sources share it.
		for sample in buffer.iter_mut() {
			*sample += phase.sin() as f32 * Self::GAIN;
			phase = (phase + phase_step).rem_euclid(tau);
		}

		Some(buffer)
	}

	fn done(&self, settings: PlaybackSettings, state: PlaybackState) -> bool {
		state.current_sample >= u64::from(settings.sample_rate) * Self::DURATION_SECONDS
	}
}
