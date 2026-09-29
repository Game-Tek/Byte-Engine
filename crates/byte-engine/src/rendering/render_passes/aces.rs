use super::image_transform::Configuration;

/// The ACES v1 tone mapper, installed by [`crate::application::graphics::setup_aces_tonemap_render_pass`].
pub const TONE_MAPPING: Configuration = Configuration {
	name: "aces",
	label: "ACES Tonemap",
	pipeline_id: "byte-engine/rendering/aces/tone-mapping.pipeline",
	output_name: "ACES Tonemap Output",
	requires_float_input: false,
};

#[cfg(test)]
mod tests {

	use crate::rendering::render_pass::simple_compute;
	use crate::rendering::shader_vm_test::{assert_rgba_close, run_image_transform_vm};

	const TONE_MAPPING_SHADER: &str = include_str!("../../../assets/rendering/aces/tone-mapping.besl");

	/// Verifies that an overflowed half-float highlight maps to white instead of NaN.
	#[test]
	fn aces_tonemap_besl_vm_maps_infinite_input_to_white() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(TONE_MAPPING_SHADER));

		assert_rgba_close(
			run_image_transform_vm(&program, [f32::INFINITY, f32::INFINITY, f32::INFINITY, 1.0]),
			[1.0, 1.0, 1.0, 1.0],
			1e-6,
		);
	}

	/// Verifies reference colors and bounded high-dynamic-range behavior through the VM.
	#[test]
	fn aces_tonemap_besl_vm_produces_bounded_reference_colors() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(TONE_MAPPING_SHADER));

		assert_rgba_close(
			run_image_transform_vm(&program, [0.0, 0.0, 0.0, 0.25]),
			[0.0, 0.0, 0.0, 1.0],
			1e-6,
		);
		assert_rgba_close(
			run_image_transform_vm(&program, [1.0, 1.0, 1.0, 0.25]),
			[0.9054924, 0.9054924, 0.9054924, 1.0],
			1e-5,
		);

		for input in [0.18, 4.0, 16.0] {
			let output = run_image_transform_vm(&program, [input, input, input, 0.0]);

			assert!(
				output[..3]
					.iter()
					.all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel)),
				"Invalid ACES VM output. The most likely cause is unstable tone-mapping arithmetic: {output:?}"
			);
		}
	}
}
