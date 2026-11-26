use super::image_transform::Configuration;

/// John Hable's filmic tone curve from Uncharted 2, installed by
/// [`crate::application::graphics::setup_hable_tonemap_render_pass`].
pub const TONE_MAPPING: Configuration = Configuration {
	name: "hable",
	pipeline_id: "byte_engine/rendering/hable/tone-mapping.pipeline",
	output_name: "Hable Tonemap Output",
	requires_float_input: true,
};

#[cfg(test)]
mod tests {
	use crate::rendering::shader_vm_test::{assert_rgba_close, compile_pixel_chain, run_image_transform_vm};

	/// Verifies display-encoded reference colors, a warm color's channel order, and that brighter neutral input never
	/// shows darker, rises until it reaches white, and stays in the display range, overflowed half floats included.
	#[test]
	fn hable_tonemap_besl_vm_matches_reference_colors_and_rises_monotonically() {
		let program = compile_pixel_chain(&["hable/tone-mapping.besl"]);
		let run = |color| run_image_transform_vm(&program, color);

		assert_rgba_close(run([0.0, 0.0, 0.0, 0.25]), [0.0, 0.0, 0.0, 1.0], 1e-6);
		// Hable's curve at twice the input, divided by the curve at the linear white of 11.2, then the exact sRGB
		// encoding. Inputs from 5.6 up reach the linear white, so 16 shows as white.
		for (input, expected) in [(0.18, 0.3934711), (1.0, 0.7306735), (4.0, 0.9630666), (16.0, 1.0)] {
			assert_rgba_close(run([input, input, input, 0.25]), [expected, expected, expected, 1.0], 1e-5);
		}

		// Inputs below 5.6 map below the linear white, so each must show strictly brighter than the one before it.
		let rising = [0.0, 0.001, 0.01, 0.05, 0.18, 0.5, 1.0, 2.0, 4.0, 5.5];
		// Inputs from 5.6 up reach the linear white, so each shows as white and none may show darker.
		let white = [8.0, 16.0, 1000.0, 65504.0, f32::INFINITY];
		let mut previous: Option<(f32, f32)> = None;
		for (index, input) in rising.into_iter().chain(white).enumerate() {
			let output = run([input, input, input, 0.0])[0];
			assert!(
				output.is_finite() && (0.0..=1.0).contains(&output),
				"Hable output left the display range: {input} gave {output}. The most likely cause is a missing clamp after dividing by the curve at the linear white. Clamp the mapped color to [0, 1] before encoding it."
			);
			if let Some((previous_input, previous_output)) = previous {
				let rises = if index < rising.len() {
					output > previous_output
				} else {
					output >= previous_output
				};
				assert!(
					rises,
					"Hable output fell or stalled as input grew: {previous_input} gave {previous_output} and {input} gave {output}. The most likely cause is a mistyped curve constant. Compare A through F, W, and the exposure bias with Hable's published `Uncharted2Tonemap`."
				);
			}
			previous = Some((input, output));
		}

		let warm = run([1.0, 0.5, 0.25, 0.0]);
		assert!(
			warm[0] > warm[1] && warm[1] > warm[2] && warm.iter().all(|channel| (0.0..=1.0).contains(channel)),
			"Hable changed the channel order of a warm color or left the display range: {warm:?}. The most likely cause is a channel mapped through the wrong input component. Map each of x, y, and z through the curve from its own input component."
		);
	}
}
