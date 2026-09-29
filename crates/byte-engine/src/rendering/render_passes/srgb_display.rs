//! Display encoding for scene-linear color without tone mapping.

use super::image_transform::Configuration;

/// Converts scene-linear RGB into display-encoded sRGB, installed by
/// [`crate::application::graphics::setup_srgb_display_render_pass`].
///
/// Install this as the final post-scene pass when the application needs SDR presentation without tone mapping.
/// Bypassing the pass forwards the scene color unchanged.
pub const ENCODING: Configuration = Configuration {
	name: "srgb-display",
	label: "sRGB Display Encoding",
	pipeline_id: "byte-engine/rendering/srgb-display/encode.pipeline",
	output_name: "sRGB Display Output",
	requires_float_input: true,
};

#[cfg(test)]
mod tests {
	use crate::rendering::{
		render_pass::simple_compute,
		shader_vm_test::{assert_rgba_close, run_image_transform_vm},
	};

	const SHADER: &str = include_str!("../../../assets/rendering/srgb-display/encode.besl");

	#[test]
	fn display_encoding_matches_the_srgb_transfer_function_and_preserves_alpha() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(SHADER));

		assert_rgba_close(
			run_image_transform_vm(&program, [-1.0, 0.0031308, 0.18, 0.4]),
			[0.0, 0.040449936, 0.46135613, 0.4],
			1e-6,
		);
		assert_rgba_close(
			run_image_transform_vm(&program, [1.0, 2.0, 0.0, 0.75]),
			[1.0, 1.0, 0.0, 0.75],
			1e-6,
		);
	}
}
