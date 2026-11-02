use super::image_transform::Configuration;

/// Stephen Hill's fitted approximation of the ACES v1 SDR tone mapper, installed by
/// [`crate::application::graphics::setup_aces_tonemap_render_pass`].
pub const TONE_MAPPING: Configuration = Configuration {
	name: "aces",
	label: "ACES Tonemap",
	pipeline_id: "byte_engine/rendering/aces/tone-mapping.pipeline",
	output_name: "ACES Tonemap Output",
	requires_float_input: true,
};

#[cfg(test)]
mod tests {

	use crate::rendering::render_pass::simple_compute;
	use crate::rendering::shader_vm_test::{assert_rgba_close, run_image_transform_vm};

	static TONE_MAPPING_SHADER: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("aces/tone-mapping.besl"));

	/// Verifies that an overflowed half-float highlight maps to white instead of NaN.
	#[test]
	fn aces_tonemap_besl_vm_maps_infinite_input_to_white() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*TONE_MAPPING_SHADER));

		assert_rgba_close(
			run_image_transform_vm(&program, [f32::INFINITY, f32::INFINITY, f32::INFINITY, 1.0]),
			[1.0, 1.0, 1.0, 1.0],
			1e-6,
		);
	}

	/// Verifies black and display-encoded neutral reference colors through the VM.
	#[test]
	fn aces_tonemap_besl_vm_produces_display_encoded_reference_colors() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*TONE_MAPPING_SHADER));

		assert_rgba_close(
			run_image_transform_vm(&program, [0.0, 0.0, 0.0, 0.25]),
			[0.0, 0.0, 0.0, 1.0],
			1e-6,
		);
		// Hill's fitted ACES followed by the exact sRGB encoding. The fit's matrices are rounded to five decimals, so a
		// neutral input's channels may differ in the last digits.
		for (input, expected) in [
			(0.18, 0.358457),
			(0.5, 0.645538),
			(1.0, 0.808954),
			(4.0, 0.958888),
			(16.0, 0.995563),
		] {
			assert_rgba_close(
				run_image_transform_vm(&program, [input, input, input, 0.25]),
				[expected, expected, expected, 1.0],
				2e-4,
			);
		}
	}

	/// Verifies that bright highlights stay apart below white and that a warm color keeps its channel order.
	#[test]
	fn aces_tonemap_besl_vm_separates_highlights_and_keeps_channel_order() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*TONE_MAPPING_SHADER));

		let bright = run_image_transform_vm(&program, [4.0, 4.0, 4.0, 0.0]);
		let brighter = run_image_transform_vm(&program, [16.0, 16.0, 16.0, 0.0]);

		assert!(
			bright[0] < brighter[0] && brighter[0] < 1.0,
			"ACES highlights did not stay apart below white: 4 gave {bright:?} and 16 gave {brighter:?}. The most likely cause is a tone curve that reaches white too early. Compare the curve's coefficients with Hill's `RRTAndODTFit`."
		);

		let warm = run_image_transform_vm(&program, [1.0, 0.5, 0.25, 0.0]);

		assert!(
			warm[0] > warm[1] && warm[1] > warm[2],
			"ACES changed the channel order of a warm color: {warm:?}. The most likely cause is a mistyped or transposed fit matrix. Compare each row of the color layout's ACES matrices with Hill's `ACESInputMat` and `ACESOutputMat`."
		);
		assert!(
			warm.iter()
				.all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel)),
			"ACES output left the display range: {warm:?}. The most likely cause is a missing clamp after the fit's output matrix. Clamp the fitted color to [0, 1] before encoding it."
		);
	}
}
