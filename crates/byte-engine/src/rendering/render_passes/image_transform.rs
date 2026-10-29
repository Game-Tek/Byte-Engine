//! One tile transform from the incoming `main` into its replacement, shared by tone mappers and display encoders.

use crate::{
	core::Entity,
	rendering::{
		Sink,
		pipeline_compilation::TilePipelineRequest,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_tile},
		render_passes::blit::ImageBypassPass,
	},
};

/// The `Configuration` struct names what one image transform pass reads, runs, and writes.
///
/// Each transform is a `const` next to its shader tests, such as [`super::aces::TONE_MAPPING`]. Pass it to
/// [`ImageTransformPass::new`] from a post-scene render pass factory.
pub struct Configuration {
	/// The stable pass name that `render.pass.<name>` enables or bypasses.
	pub name: &'static str,
	/// The label of the pass's GPU region and descriptor set.
	pub label: &'static str,
	/// The baked tile pipeline, which reads `source` and writes its `result` output.
	pub pipeline_id: &'static str,
	/// The render-target name of the replacement `main`, which screenshots can capture.
	pub output_name: &'static str,
	/// Whether the transform needs scene-linear floating-point input, so a preceding display-referred `main` is a
	/// setup mistake.
	pub requires_float_input: bool,
}

/// The `ImageTransformPass` struct runs one sink's image transform, such as a tone mapper, as a post-scene pass.
///
/// Register it with [`crate::rendering::renderer::Renderer::add_post_scene_render_pass_for_all_sinks`]. The renderer
/// hands it the swapchain directly when it is the final pass.
pub struct ImageTransformPass {
	name: &'static str,
	pass: simple_tile::Pass,
}

impl Entity for ImageTransformPass {}

impl ImageTransformPass {
	/// Starts the transform and bypass pipelines for outputs of `format` while window creation is still pending.
	///
	/// A transform is usually the final pass, which writes the swapchain, so pass the context's
	/// [`ghi::context::Context::swapchain_format`]. A transform placed earlier requests its own format when it binds.
	pub(crate) fn request_pipelines(
		manager: &crate::rendering::PipelineManagerClient,
		configuration: &Configuration,
		format: ghi::Formats,
	) {
		manager.request_tile_pipeline(TilePipelineRequest::new(configuration.pipeline_id, vec![format]));
		ImageBypassPass::request_forwarding_pipeline(manager, format);
	}

	/// Binds one sink's current `main` to a new display-color `main` written by the configured transform.
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>, configuration: &Configuration) -> Self {
		let source = render_pass_builder.read_from("main");
		if configuration.requires_float_input {
			assert_eq!(
				render_pass_builder.format_of("main").encoding(),
				Some(ghi::Encodings::FloatingPoint),
				"{} requires scene-linear floating-point input. The most likely cause is a preceding pass that replaced `main` with another format.",
				configuration.label
			);
		}
		// The replacement is the transform's tile output, which the builder gives the uses that needs.
		let destination = render_pass_builder.create_main_render_target(
			ghi::image::Builder::new(crate::rendering::DISPLAY_COLOR_FORMAT, ghi::Uses::Image).name(configuration.output_name),
		);
		let pass = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new(configuration.label, configuration.pipeline_id, &[destination.format()]),
		)
		.bind(
			configuration.label,
			&[simple_tile::Resource::image("source", source)],
			&[simple_tile::Output::new("result", destination)],
		);

		Self {
			name: configuration.name,
			pass,
		}
	}
}

impl RenderPass for ImageTransformPass {
	fn name(&self) -> &'static str {
		self.name
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.pass.prepare(frame, sink, frame_allocator)
	}
}
