const CONFIGURATION: tone_map::Configuration = tone_map::Configuration {
	pipeline_id: "byte-engine/rendering/agx/tone-mapping.pipeline",
	descriptor_set_name: "AGX Tonemap Pass Descriptor Set",
	shader_error: "Failed to create AGX tone mapping shader",
};

/// The `BaseAgxToneMapPass` struct defines the shared GPU state required for AGX tonemapping.
#[derive(Clone)]
pub struct BaseAgxToneMapPass {
	pipeline: crate::rendering::render_pass::simple_compute::Pipeline,
}

impl Entity for BaseAgxToneMapPass {}

impl BaseAgxToneMapPass {
	/// Creates the shared AGX compute pipeline resources used by per-view tonemap passes.
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		Self {
			pipeline: tone_map::create_pipeline(render_pass_builder, &CONFIGURATION),
		}
	}
}

/// The `AgxToneMapPass` struct defines a per-view AGX tonemapping pass instance.
pub struct AgxToneMapPass {
	render_pass: crate::rendering::render_pass::simple_compute::Pass,
	bypass_pass: crate::rendering::render_passes::blit::SwapchainBlitPass,
}

impl AgxToneMapPass {
	/// Creates the per-view descriptor bindings for the AGX tonemap pass.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let base = BaseAgxToneMapPass::new(render_pass_builder);
		let passes = tone_map::create_passes(render_pass_builder, &base.pipeline, &CONFIGURATION);
		AgxToneMapPass {
			render_pass: passes.active,
			bypass_pass: passes.bypass,
		}
	}
}

impl Entity for AgxToneMapPass {}

impl RenderPass for AgxToneMapPass {
	fn name(&self) -> &'static str {
		"agx"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.render_pass.prepare(frame, sink, frame_allocator)
	}

	fn bypass<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.bypass_pass.prepare(frame, sink, frame_allocator)
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot};
	use resource_management::shader::{besl::backends::msl::MSLTranspiler, generator::ShaderGenerationSettings};

	use crate::rendering::render_pass::simple_compute;
	use crate::rendering::shader_vm_test::{assert_rgba_close, empty_image, rgba, run_at, texture_2d};

	const TONE_MAPPING_SHADER: &str = include_str!("../../../assets/rendering/agx/tone-mapping.besl");

	/// Executes the compiled AGX program for one source color.
	fn run_agx_vm(program: &besl::vm::ExecutableProgram, source_color: [f32; 4]) -> [f32; 4] {
		let mut source = texture_2d(1, 1, &[source_color]);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_image(ResourceSlot::new(0), &mut source);
		descriptors.bind_image(ResourceSlot::new(1), &mut result);
		run_at(program, &mut descriptors, [0, 0]);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	/// Verifies display-encoded reference colors, neutral highlights, channel ordering, and bounded output through the VM.
	#[test]
	fn agx_tonemap_besl_vm_produces_bounded_reference_colors() {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(TONE_MAPPING_SHADER));

		assert_rgba_close(run_agx_vm(&program, [0.0, 0.0, 0.0, 0.25]), [0.0, 0.0, 0.0, 1.0], 1e-6);
		assert_rgba_close(
			run_agx_vm(&program, [1.0, 1.0, 1.0, 0.25]),
			[0.7919241, 0.7918683, 0.7918481, 1.0],
			2e-5,
		);
		let highlight = run_agx_vm(&program, [16.0, 16.0, 16.0, 0.0]);

		assert!(
			highlight[0] > 0.98 && (highlight[0] - highlight[1]).abs() < 2e-4 && (highlight[1] - highlight[2]).abs() < 2e-4,
			"Invalid AGX neutral highlight. The most likely cause is missing display encoding or an incorrect color-space transform: {highlight:?}"
		);

		let warm = run_agx_vm(&program, [1.0, 0.5, 0.25, 0.0]);

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

use super::tone_map;
use crate::core::Entity;
use crate::rendering::{
	Sink,
	render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
};
