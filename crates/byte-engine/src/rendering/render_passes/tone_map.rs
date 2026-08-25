use crate::rendering::render_pass::{
	RenderPassBuilder,
	simple_compute::{Descriptor, Pass, Pipeline, Resource},
};
use crate::rendering::render_passes::blit::SwapchainBlitPass;

/// The `Configuration` struct keeps algorithm-specific tonemap resource names behind the shared pass implementation.
pub(super) struct Configuration {
	pub pipeline_id: &'static str,
	pub descriptor_set_name: &'static str,
	pub shader_error: &'static str,
}

/// Creates the reusable pipeline for one tonemap algorithm.
pub(super) fn create_pipeline(render_pass_builder: &mut RenderPassBuilder<'_>, configuration: &Configuration) -> Pipeline {
	Pipeline::compile(render_pass_builder, Descriptor::new("Tonemap", configuration.pipeline_id))
		.expect(configuration.shader_error)
}

/// The `ToneMapPasses` struct keeps the active tonemap and its swapchain-forwarding bypass together.
pub(super) struct ToneMapPasses {
	pub active: Pass,
	pub bypass: SwapchainBlitPass,
}

/// Binds one sink's source to both its tonemap and bypass swapchain destinations.
pub(super) fn create_passes(
	render_pass_builder: &mut RenderPassBuilder<'_>,
	pipeline: &Pipeline,
	configuration: &Configuration,
) -> ToneMapPasses {
	let source: ghi::BaseImageHandle = render_pass_builder.read_from("main").into();
	let destination = render_pass_builder.render_to_swapchain();
	let active = pipeline
		.bind(
			render_pass_builder,
			configuration.descriptor_set_name,
			&[
				Resource::image("source", source),
				Resource::swapchain("result", destination),
			],
		)
		.expect(
			"Failed to bind tonemap resources. The most likely cause is that the tonemap BESL interface changed without updating its resources.",
		);
	let bypass = SwapchainBlitPass::from_source(render_pass_builder, source);

	ToneMapPasses { active, bypass }
}
