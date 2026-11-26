use super::image_transform::Configuration;

/// The AgX tone mapper, installed by [`crate::application::graphics::setup_agx_tonemap_render_pass`].
pub const TONE_MAPPING: Configuration = Configuration {
	name: "agx",
	pipeline_id: "byte_engine/rendering/agx/tone-mapping.pipeline",
	output_name: "AGX Tonemap Output",
	requires_float_input: false,
};

#[cfg(test)]
mod tests {
	use crate::rendering::shader_vm_test::{assert_rgba_close, compile_pixel_chain, run_image_transform_vm};

	/// Verifies display-encoded reference colors, neutral highlights, channel ordering, and bounded output through the VM.
	#[test]
	fn agx_tonemap_besl_vm_produces_bounded_reference_colors() {
		let program = compile_pixel_chain(&["agx/tone-mapping.besl"]);
		let run = |color| run_image_transform_vm(&program, color);

		assert_rgba_close(run([0.0, 0.0, 0.0, 0.25]), [0.0, 0.0, 0.0, 1.0], 1e-6);
		assert_rgba_close(run([1.0, 1.0, 1.0, 0.25]), [0.7919241, 0.7918683, 0.7918481, 1.0], 2e-5);
		let highlight = run([16.0, 16.0, 16.0, 0.0]);
		assert!(
			highlight[0] > 0.98 && (highlight[0] - highlight[1]).abs() < 2e-4 && (highlight[1] - highlight[2]).abs() < 2e-4,
			"Invalid AGX neutral highlight. The most likely cause is missing display encoding or an incorrect color-space transform: {highlight:?}"
		);

		let warm = run([1.0, 0.5, 0.25, 0.0]);
		assert!(
			warm[0] > warm[1] && warm[1] > warm[2],
			"Invalid AGX channel ordering. The most likely cause is an incorrect color-space transform: {warm:?}"
		);
		assert!(
			warm.iter()
				.all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel)),
			"Invalid AGX VM output. The most likely cause is unstable tone-mapping arithmetic: {warm:?}"
		);
	}
}
