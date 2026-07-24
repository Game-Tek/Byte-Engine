use std::borrow::Borrow as _;

use ghi::{command_buffer::CommonCommandBufferMode as _, context::ContextCreate as _};

use crate::rendering::{
	render_pass::{simple_compute, RenderPassBuilder, RenderPassReturn},
	RenderPass, Sink,
};

#[derive(Clone)]
pub struct BaseBilateralBlurPass {
	pipeline_x: simple_compute::Pipeline,
	pipeline_y: simple_compute::Pipeline,
}

impl BaseBilateralBlurPass {
	fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let pipeline_x = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Bilateral Blur", "byte-engine/rendering/bilateral-blur/x.besl", "SSGI Blur X"),
		)
		.expect("Failed to create the X SSGI blur shader. The most likely cause is invalid bilateral blur BESL.");

		let pipeline_y = pipeline_x
			.compile_variant(
				render_pass_builder,
				simple_compute::Descriptor::new("Bilateral Blur", "byte-engine/rendering/bilateral-blur/y.besl", "SSGI Blur Y"),
			)
			.expect("Failed to create the Y SSGI blur shader. The most likely cause is invalid bilateral blur BESL.");

		Self { pipeline_x, pipeline_y }
	}
}

struct BilateralBlurPass {
	pass_x: simple_compute::Pass,
	pass_y: simple_compute::Pass,
	bypass_pass: crate::rendering::render_passes::blit::ImageBypassPass,
}

impl BilateralBlurPass {
	pub fn new(
		render_pass_builder: &mut RenderPassBuilder,
		render_pass: &BaseBilateralBlurPass,
		source: ghi::BaseImageHandle,
	) -> Self {
		let read_depth = render_pass_builder.read_from("depth");
		let depth_image: ghi::BaseImageHandle = (*read_depth.borrow()).into();

		let context = render_pass_builder.context();
		let x_blur_map = context.build_image(ghi::image::Builder::new(
			ghi::Formats::RGB16UNORM,
			ghi::Uses::Image | ghi::Uses::Storage,
		));
		let y_blur_map = context.build_image(ghi::image::Builder::new(
			ghi::Formats::RGB16UNORM,
			ghi::Uses::Image | ghi::Uses::Storage,
		));
		let sampler = context.build_sampler(ghi::sampler::Builder::new());
		let depth_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear),
		);

		let pass_x = render_pass
			.pipeline_x
			.bind(
				render_pass_builder,
				"X SSGI Blur",
				&[
					simple_compute::Resource::combined_image_sampler(
						"depth",
						depth_image,
						depth_sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::combined_image_sampler("source", source, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result", x_blur_map),
				],
			)
			.expect(
				"Failed to bind X SSGI blur resources. The most likely cause is a mismatch between the BESL bindings and pass resources.",
			);
		let pass_y = render_pass
			.pipeline_y
			.bind(
				render_pass_builder,
				"Y SSGI Blur",
				&[
					simple_compute::Resource::combined_image_sampler(
						"depth",
						depth_image,
						depth_sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::combined_image_sampler("source", x_blur_map, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result", y_blur_map),
				],
			)
			.expect(
				"Failed to bind Y SSGI blur resources. The most likely cause is a mismatch between the BESL bindings and pass resources.",
			);

		let bypass_pass = crate::rendering::render_passes::blit::ImageBypassPass::new(render_pass_builder, source, y_blur_map);

		Self {
			pass_x,
			pass_y,
			bypass_pass,
		}
	}
}

impl RenderPass for BilateralBlurPass {
	fn name(&self) -> &'static str {
		"bilateral blur"
	}

	fn prepare<'a>(
		&mut self,
		_frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let pass_x = self.pass_x;
		let pass_y = self.pass_y;
		let extent = sink.extent();

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("Bilateral Blur"),
					|command_buffer| {
						pass_x.record(command_buffer, extent);
						pass_y.record(command_buffer, extent);
					},
				);
			},
		))
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

	use super::*;
	use crate::rendering::shader_vm_test::{assert_rgba_close, empty_image, rgba, run_at, texture_2d};

	const BILATERAL_X_SHADER: &str = include_str!("../../../assets/rendering/bilateral-blur/x.besl");
	const BILATERAL_Y_SHADER: &str = include_str!("../../../assets/rendering/bilateral-blur/y.besl");

	/// Executes one canonical bilateral shader against deterministic texture fixtures.
	fn run_bilateral_vm(
		shader: &str,
		extent: [u32; 2],
		depth_texels: &[[f32; 4]],
		source_texels: &[[f32; 4]],
		coordinate: [u32; 2],
	) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(shader));
		let mut depth = texture_2d(extent[0], extent[1], depth_texels);
		let mut source = texture_2d(extent[0], extent[1], source_texels);
		let mut result = empty_image(extent[0], extent[1]);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut depth);
		descriptors.bind_texture(ResourceSlot::new(1), &mut source);
		descriptors.bind_image(ResourceSlot::new(2), &mut result);
		run_at(&program, &mut descriptors, coordinate);
		drop(descriptors);
		rgba(&result, coordinate)
	}

	/// Verifies that both canonical blur axes preserve a constant source.
	#[test]
	fn bilateral_blur_besl_vm_preserves_constant_input_in_both_axes() {
		for shader in [BILATERAL_X_SHADER, BILATERAL_Y_SHADER] {
			let output = run_bilateral_vm(shader, [1, 1], &[[0.5, 0.0, 0.0, 1.0]], &[[0.375, 0.9, 0.1, 0.2]], [0, 0]);
			assert_rgba_close(output, [0.375, 0.375, 0.375, 1.0], 1e-5);
		}
	}

	/// Verifies that depth rejection blocks horizontal edge bleed while the vertical kernel preserves its column.
	#[test]
	fn bilateral_blur_besl_vm_respects_depth_edges_and_direction() {
		let depth = [
			[1.0, 0.0, 0.0, 1.0],
			[0.5, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.5, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.5, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
		];
		let source = [
			[1.0, 0.0, 0.0, 1.0],
			[0.2, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.2, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.2, 0.0, 0.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
		];

		let horizontal = run_bilateral_vm(BILATERAL_X_SHADER, [3, 3], &depth, &source, [1, 1]);
		let vertical = run_bilateral_vm(BILATERAL_Y_SHADER, [3, 3], &depth, &source, [1, 1]);

		assert_rgba_close(horizontal, [0.0, 0.0, 0.0, 1.0], 1e-6);
		assert_rgba_close(vertical, [0.2, 0.2, 0.2, 1.0], 1e-5);
	}
}
