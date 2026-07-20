//! Plays a sound through the default audio output as an application startup smoke test.
//!
//! This example verifies that the complete application can start and run. It
//! does not verify the generated audio.

#![allow(clippy::no_effect)]

use byte_engine::{
	application::{Application, Parameter},
	audio::synthesizer::Synthesizer,
};

fn main() {
	let mut app = byte_engine::application::graphics::GraphicsApplication::new(
		"Sound Smoke Test",
		&[
			Parameter::new("kill-after", "60"),
			Parameter::new("render.ghi.features.mesh-shading", "false"), // Many devices don't support this feature and it is not necessary for this test.
		],
	);

	TestSynthesizer {}; // TODO: wire to audio system

	app.do_loop();
}

struct TestSynthesizer;

impl Synthesizer for TestSynthesizer {
	fn render<'a>(&self, current_sample: u32, buffer: &'a mut [f32]) -> &'a [f32] {
		let pitch = 440f32;
		let gain = 1f32;
		let sample_rate = 44100;

		let tau = std::f64::consts::TAU;
		let sample_rate = sample_rate as f64;
		let phase_step = tau * pitch as f64 / sample_rate;
		let mut phase = (current_sample as f64 * phase_step).rem_euclid(tau);

		for b in buffer.iter_mut() {
			let sample = phase.sin() as f32;
			*b += sample * gain;

			phase += phase_step;
			if phase >= tau {
				phase -= tau;
			} else if phase < 0.0 {
				phase += tau;
			}
		}

		buffer
	}
}
