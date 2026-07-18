use super::tone_map;
use crate::core::Entity;
use crate::rendering::{
	render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
	Sink,
};

const CONFIGURATION: tone_map::Configuration = tone_map::Configuration {
	shader_id: "byte-engine/rendering/aces/tone-mapping.besl",
	shader_name: "ACES Tone Mapping Compute Shader",
	descriptor_set_name: "Tonemap Pass Descriptor Set",
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
}

impl AcesToneMapPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let base = BaseAcesToneMapPass::new(render_pass_builder);
		let render_pass = tone_map::create_pass(render_pass_builder, &base.pipeline, &CONFIGURATION);
		AcesToneMapPass { render_pass }
	}
}

impl Entity for AcesToneMapPass {}

impl RenderPass for AcesToneMapPass {
	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.render_pass.prepare(frame, sink, frame_allocator)
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot};

	use crate::rendering::render_pass::simple_compute;
	use crate::rendering::shader_vm_test::{assert_rgba_close, empty_image, rgba, run_at, texture_2d};

	const TONE_MAPPING_SHADER: &str = include_str!("../../../assets/rendering/aces/tone-mapping.besl");

	/// Executes the compiled ACES program for one source color.
	fn run_aces_vm(program: &besl::vm::ExecutableProgram, source_color: [f32; 4]) -> [f32; 4] {
		let mut source = texture_2d(1, 1, &[source_color]);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_image(ResourceSlot::new(0), &mut source);
		descriptors.bind_image(ResourceSlot::new(1), &mut result);
		run_at(program, &mut descriptors, [0, 0]);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	/// Verifies reference colors and bounded high-dynamic-range behavior through the VM.
	#[test]
	fn aces_tonemap_besl_vm_produces_bounded_reference_colors() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(TONE_MAPPING_SHADER));

		assert_rgba_close(run_aces_vm(&program, [0.0, 0.0, 0.0, 0.25]), [0.0, 0.0, 0.0, 1.0], 1e-6);
		assert_rgba_close(
			run_aces_vm(&program, [1.0, 1.0, 1.0, 0.25]),
			[0.9054924, 0.9054924, 0.9054924, 1.0],
			1e-5,
		);

		for input in [0.18, 4.0, 16.0] {
			let output = run_aces_vm(&program, [input, input, input, 0.0]);
			assert!(
				output[..3]
					.iter()
					.all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel)),
				"Invalid ACES VM output. The most likely cause is unstable tone-mapping arithmetic: {output:?}"
			);
		}
	}
}
