//! Display encoding for scene-linear color without tone mapping.

use crate::{
	core::Entity,
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_compute},
		render_passes::blit::ImageBypassPass,
	},
};

const PIPELINE: &str = "byte-engine/rendering/srgb-display/encode.pipeline";

/// The `SrgbDisplayPass` struct converts scene-linear RGB into display-encoded sRGB.
///
/// Install this as the final post-scene pass when the application needs SDR
/// presentation without tone mapping. Bypassing the pass forwards the scene
/// color unchanged.
pub struct SrgbDisplayPass {
	encode: simple_compute::Pass,
	bypass: ImageBypassPass,
}

impl Entity for SrgbDisplayPass {}

impl SrgbDisplayPass {
	/// Creates one sink-local display encoder from the current `main` image.
	///
	/// Register the pass before creating a window. The renderer then supplies
	/// the swapchain directly when this is the final post-scene pass.
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		let source = render_pass_builder.read_from("main");
		let format = render_pass_builder.format_of("main");
		assert_eq!(
			format,
			crate::rendering::SCENE_COLOR_FORMAT,
			"sRGB display encoding requires scene-linear RGBA16F input. The most likely cause is a preceding pass that replaced `main` with another format."
		);
		let destination = render_pass_builder.create_main_render_target(
			ghi::image::Builder::new(format, ghi::Uses::Storage | ghi::Uses::Image).name("sRGB Display Output"),
		);
		let pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("sRGB Display Encoding", PIPELINE),
		)
		.expect(
			"Failed to create the sRGB display-encoding shader. The most likely cause is an incompatible shader interface.",
		);
		let encode = pipeline
			.bind(
				render_pass_builder,
				"sRGB Display Descriptor Set",
				&[
					simple_compute::Resource::image("source", source),
					simple_compute::Resource::image("result", destination),
				],
			)
			.expect(
				"Failed to bind sRGB display resources. The most likely cause is a mismatch between the BESL bindings and pass resources.",
			);
		let bypass = ImageBypassPass::new(render_pass_builder, source, destination);

		Self { encode, bypass }
	}
}

impl RenderPass for SrgbDisplayPass {
	fn name(&self) -> &'static str {
		"srgb-display"
	}

	crate::rendering::render_pass::forward_to_inner_pass!(prepare = encode);
	crate::rendering::render_pass::forward_to_inner_pass!(bypass = bypass);
}

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
