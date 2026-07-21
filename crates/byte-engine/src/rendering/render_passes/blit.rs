use crate::{
	core::Entity,
	rendering::{
		render_pass::{simple_compute, RenderPassBuilder, RenderPassReturn},
		RenderPass, Sink,
	},
};

/// The `ImageBypassPass` struct preserves an intermediate image result when an effect is bypassed.
pub(crate) struct ImageBypassPass {
	render_pass: simple_compute::Pass,
}

impl ImageBypassPass {
	/// Creates a compute copy from the pass input to the output consumed by downstream passes.
	pub(crate) fn new(
		render_pass_builder: &mut RenderPassBuilder<'_>,
		source: impl Into<ghi::BaseImageHandle>,
		destination: impl Into<ghi::BaseImageHandle>,
	) -> Self {
		let pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new(
				"Render Pass Bypass",
				"byte-engine/rendering/blit/image.besl",
				"Render Pass Bypass Compute Shader",
			),
		)
		.expect("Failed to create the render-pass bypass shader. The most likely cause is an incompatible shader interface.");
		let render_pass = pipeline
			.bind(
				render_pass_builder,
				"Render Pass Bypass Descriptor Set",
				&[
					simple_compute::Resource::image("source", source),
					simple_compute::Resource::image("result", destination),
				],
			)
			.expect(
				"Failed to bind render-pass bypass resources. The most likely cause is a mismatch between the BESL bindings and pass resources.",
			);

		Self { render_pass }
	}

	/// Prepares the forwarding copy for the current sink.
	pub(crate) fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.render_pass.prepare(frame, sink, frame_allocator)
	}
}

#[derive(Clone)]
pub struct BaseSwapchainBlitPass {
	pipeline: simple_compute::Pipeline,
}

impl Entity for BaseSwapchainBlitPass {}

impl BaseSwapchainBlitPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		let pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new(
				"Swapchain Blit",
				"byte-engine/rendering/blit/swapchain.besl",
				"Swapchain Blit Compute Shader",
			),
		)
		.expect("Failed to create swapchain blit shader");

		Self { pipeline }
	}
}

pub struct SwapchainBlitPass {
	render_pass: simple_compute::Pass,
}

impl SwapchainBlitPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let read_from_main = render_pass_builder.read_from("main");
		Self::from_source(render_pass_builder, read_from_main)
	}

	/// Creates a swapchain forwarding pass for a source already declared by another pass.
	pub(crate) fn from_source(render_pass_builder: &mut RenderPassBuilder, source: impl Into<ghi::BaseImageHandle>) -> Self {
		let base = BaseSwapchainBlitPass::new(render_pass_builder);
		let render_to_swapchain = render_pass_builder.render_to_swapchain();
		let render_pass = base
			.pipeline
			.bind(
				render_pass_builder,
				"Swapchain Blit Pass Descriptor Set",
				&[
					simple_compute::Resource::image("source", source),
					simple_compute::Resource::swapchain("result", render_to_swapchain),
				],
			)
			.expect(
				"Failed to bind swapchain blit resources. The most likely cause is a mismatch between the BESL bindings and pass resources.",
			);

		Self { render_pass }
	}
}

impl Entity for SwapchainBlitPass {}

impl RenderPass for SwapchainBlitPass {
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
		self.render_pass.prepare(frame, sink, frame_allocator)
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot};

	use super::simple_compute;
	use crate::rendering::shader_vm_test::{assert_rgba_close, empty_image, rgba, run_at, texture_2d};

	const IMAGE_BYPASS_SHADER: &str = include_str!("../../../assets/rendering/blit/image.besl");
	const SWAPCHAIN_BLIT_SHADER: &str = include_str!("../../../assets/rendering/blit/swapchain.besl");

	#[test]
	fn image_bypass_besl_vm_copies_pixels_and_ignores_out_of_bounds_invocations() {
		assert_copy_shader_behavior(IMAGE_BYPASS_SHADER);
	}

	/// Verifies exact production blits and the dispatch guard through the VM.
	#[test]
	fn swapchain_blit_besl_vm_copies_pixels_and_ignores_out_of_bounds_invocations() {
		assert_copy_shader_behavior(SWAPCHAIN_BLIT_SHADER);
	}

	/// Executes one production copy shader and verifies forwarding and dispatch-boundary behavior.
	fn assert_copy_shader_behavior(source_code: &str) {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(source_code));
		let expected = [
			[0.1, 0.2, 0.3, 0.4],
			[0.5, 0.6, 0.7, 0.8],
			[0.9, 0.8, 0.7, 0.6],
			[0.4, 0.3, 0.2, 0.1],
		];
		let mut source = texture_2d(2, 2, &expected);
		let mut result = empty_image(2, 2);

		for y in 0..2 {
			for x in 0..2 {
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_image(ResourceSlot::new(0), &mut source);
				descriptors.bind_image(ResourceSlot::new(1), &mut result);
				run_at(&program, &mut descriptors, [x, y]);
			}
		}

		for (index, expected) in expected.into_iter().enumerate() {
			assert_rgba_close(rgba(&result, [(index % 2) as u32, (index / 2) as u32]), expected, 0.0);
		}

		// Dispatch rounding may produce excess invocations, so the production guard must make those invocations true no-ops.
		for coordinate in [[2, 0], [0, 2]] {
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_image(ResourceSlot::new(0), &mut source);
			descriptors.bind_image(ResourceSlot::new(1), &mut result);
			run_at(&program, &mut descriptors, coordinate);
		}
		for (index, expected) in expected.into_iter().enumerate() {
			assert_rgba_close(rgba(&result, [(index % 2) as u32, (index / 2) as u32]), expected, 0.0);
		}
	}
}
