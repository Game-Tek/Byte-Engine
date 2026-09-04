use super::tone_map;
use crate::core::Entity;
use crate::rendering::{
	Sink,
	render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
};

const CONFIGURATION: tone_map::Configuration = tone_map::Configuration {
	pipeline_id: "byte-engine/rendering/aces/tone-mapping.pipeline",
	descriptor_set_name: "Tonemap Pass Descriptor Set",
	output_name: "ACES Tonemap Output",
	shader_error: "Failed to create ACES tone mapping shader. The most likely cause is an incompatible shader interface.",
};

/// The `BaseAcesToneMapPass` struct provides shared ACES compute pipeline state to per-view passes.
#[derive(Clone)]
pub struct BaseAcesToneMapPass {
	pipeline: crate::rendering::render_pass::simple_compute::Pipeline,
}

impl Entity for BaseAcesToneMapPass {}

impl BaseAcesToneMapPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		Self {
			pipeline: tone_map::create_pipeline(render_pass_builder, &CONFIGURATION),
		}
	}
}

/// The `AcesToneMapPass` struct provides one view with ACES tonemapping descriptor bindings.
pub struct AcesToneMapPass {
	render_pass: crate::rendering::render_pass::simple_compute::Pass,
	bypass_pass: crate::rendering::render_passes::blit::ImageBypassPass,
}

impl AcesToneMapPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let base = BaseAcesToneMapPass::new(render_pass_builder);
		let passes = tone_map::create_passes(render_pass_builder, &base.pipeline, &CONFIGURATION);
		AcesToneMapPass {
			render_pass: passes.active,
			bypass_pass: passes.bypass,
		}
	}
}

impl Entity for AcesToneMapPass {}

impl RenderPass for AcesToneMapPass {
	fn name(&self) -> &'static str {
		"aces"
	}

	crate::rendering::render_pass::forward_to_inner_pass!(prepare = render_pass);

	crate::rendering::render_pass::forward_to_inner_pass!(bypass = bypass_pass);
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot};

	use crate::rendering::render_pass::simple_compute;
	use crate::rendering::shader_vm_test::{assert_rgba_close, run_tone_mapping_vm};

	const TONE_MAPPING_SHADER: &str = include_str!("../../../assets/rendering/aces/tone-mapping.besl");

	/// Verifies reference colors and bounded high-dynamic-range behavior through the VM.
	#[test]
	fn aces_tonemap_besl_vm_produces_bounded_reference_colors() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(TONE_MAPPING_SHADER));

		assert_rgba_close(
			run_tone_mapping_vm(&program, [0.0, 0.0, 0.0, 0.25]),
			[0.0, 0.0, 0.0, 1.0],
			1e-6,
		);
		assert_rgba_close(
			run_tone_mapping_vm(&program, [1.0, 1.0, 1.0, 0.25]),
			[0.9054924, 0.9054924, 0.9054924, 1.0],
			1e-5,
		);

		for input in [0.18, 4.0, 16.0] {
			let output = run_tone_mapping_vm(&program, [input, input, input, 0.0]);

			assert!(
				output[..3]
					.iter()
					.all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel)),
				"Invalid ACES VM output. The most likely cause is unstable tone-mapping arithmetic: {output:?}"
			);
		}
	}
}
