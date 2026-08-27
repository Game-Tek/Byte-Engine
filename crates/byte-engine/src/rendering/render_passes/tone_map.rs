use crate::rendering::render_pass::{
	RenderPassBuilder,
	simple_compute::{Descriptor, Pass, Pipeline, Resource},
};
use crate::rendering::render_passes::blit::ImageBypassPass;

/// The `Configuration` struct keeps algorithm-specific tonemap resource names behind the shared pass implementation.
pub(super) struct Configuration {
	pub pipeline_id: &'static str,
	pub descriptor_set_name: &'static str,
	pub output_name: &'static str,
	pub shader_error: &'static str,
}

/// Creates the reusable pipeline for one tonemap algorithm.
pub(super) fn create_pipeline(render_pass_builder: &mut RenderPassBuilder<'_>, configuration: &Configuration) -> Pipeline {
	Pipeline::compile(render_pass_builder, Descriptor::new("Tonemap", configuration.pipeline_id))
		.expect(configuration.shader_error)
}

/// The `ToneMapPasses` struct keeps the active tonemap and its image-forwarding bypass together.
pub(super) struct ToneMapPasses {
	pub active: Pass,
	pub bypass: ImageBypassPass,
}

/// Binds one sink's source to a new `main` result shared by the active and bypass paths.
pub(super) fn create_passes(
	render_pass_builder: &mut RenderPassBuilder<'_>,
	pipeline: &Pipeline,
	configuration: &Configuration,
) -> ToneMapPasses {
	let source: ghi::BaseImageHandle = render_pass_builder.read_from("main").into();
	let format = render_pass_builder.format_of("main");
	let destination = render_pass_builder.create_main_render_target(
		ghi::image::Builder::new(format, ghi::Uses::Storage | ghi::Uses::Image).name(configuration.output_name),
	);
	let active = pipeline
		.bind(
			render_pass_builder,
			configuration.descriptor_set_name,
			&[
				Resource::image("source", source),
				Resource::image("result", destination),
			],
		)
		.expect(
			"Failed to bind tonemap resources. The most likely cause is that the tonemap BESL interface changed without updating its resources.",
		);
	let bypass = ImageBypassPass::new(render_pass_builder, source, destination);

	ToneMapPasses { active, bypass }
}
