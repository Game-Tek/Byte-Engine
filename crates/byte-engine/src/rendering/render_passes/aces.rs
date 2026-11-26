use super::image_transform::Configuration;

/// Stephen Hill's fitted approximation of the ACES v1 SDR tone mapper, installed by
/// [`crate::application::graphics::setup_aces_tonemap_render_pass`].
pub const TONE_MAPPING: Configuration = Configuration {
	name: "aces",
	pipeline_id: "byte_engine/rendering/aces/tone-mapping.pipeline",
	output_name: "ACES Tonemap Output",
	requires_float_input: true,
};

#[cfg(test)]
mod tests {
	use crate::rendering::shader_vm_test::{assert_rgba_close, compile_pixel_chain, run_image_transform_vm};

	/// Verifies display-encoded reference colors, white for overflowed half floats, separated highlights, and channel
	/// order through the VM.
	#[test]
	fn aces_tonemap_besl_vm_matches_reference_colors() {
		let program = compile_pixel_chain(&["aces/tone-mapping.besl"]);
		let run = |color| run_image_transform_vm(&program, color);

		assert_rgba_close(run([f32::INFINITY, f32::INFINITY, f32::INFINITY, 1.0]), [1.0; 4], 1e-6);
		assert_rgba_close(run([0.0, 0.0, 0.0, 0.25]), [0.0, 0.0, 0.0, 1.0], 1e-6);
		// Hill's fitted ACES followed by the exact sRGB encoding. The fit's matrices are rounded to five decimals, so a
		// neutral input's channels may differ in the last digits.
		for (input, expected) in [
			(0.18, 0.358457),
			(0.5, 0.645538),
			(1.0, 0.808954),
			(4.0, 0.958888),
			(16.0, 0.995563),
		] {
			assert_rgba_close(run([input, input, input, 0.25]), [expected, expected, expected, 1.0], 2e-4);
		}

		let warm = run([1.0, 0.5, 0.25, 0.0]);
		assert!(
			warm[0] > warm[1] && warm[1] > warm[2],
			"ACES changed the channel order of a warm color: {warm:?}. The most likely cause is a mistyped or transposed fit matrix. Compare each row of the color layout's ACES matrices with Hill's `ACESInputMat` and `ACESOutputMat`."
		);
		assert!(
			warm.iter().all(|channel| (0.0..=1.0).contains(channel)),
			"ACES output left the display range: {warm:?}. The most likely cause is a missing clamp after the fit's output matrix. Clamp the fitted color to [0, 1] before encoding it."
		);
	}
}
