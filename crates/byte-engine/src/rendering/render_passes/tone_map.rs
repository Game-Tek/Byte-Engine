use utils::Extent;

use crate::rendering::render_pass::{
	simple_compute::{Descriptor, Pass, Pipeline, Resource},
	RenderPassBuilder,
};

/// The `Configuration` struct keeps algorithm-specific tonemap names and source behind the shared pass implementation.
pub(super) struct Configuration {
	pub shader_id: &'static str,
	pub shader_name: &'static str,
	pub settings_name: &'static str,
	pub set_layout_name: &'static str,
	pub descriptor_set_name: &'static str,
	pub source: &'static str,
	pub shader_error: &'static str,
	pub syntax_error: &'static str,
}

/// Creates the reusable pipeline for one tonemap algorithm.
pub(super) fn create_pipeline(render_pass_builder: &mut RenderPassBuilder<'_>, configuration: &Configuration) -> Pipeline {
	Pipeline::compile(
		render_pass_builder,
		Descriptor::new(
			"Tonemap",
			configuration.shader_id,
			configuration.shader_name,
			create_program(configuration),
			Extent::square(32),
		)
		.generation_name(configuration.settings_name)
		.layout_name(configuration.set_layout_name),
	)
	.expect(configuration.shader_error)
}

/// Binds one sink's source and swapchain destination to a reusable tonemap pipeline.
pub(super) fn create_pass(
	render_pass_builder: &mut RenderPassBuilder<'_>,
	pipeline: &Pipeline,
	configuration: &Configuration,
) -> Pass {
	let source = render_pass_builder.read_from("main");
	let destination = render_pass_builder.render_to_swapchain();
	pipeline
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
		)
}

/// Builds the shared BESL image interface around one tonemap algorithm.
pub(super) fn create_program(configuration: &Configuration) -> besl::NodeReference {
	let mut program = crate::rendering::render_pass::simple_compute::Program::new();
	program.binding(
		"source",
		besl::BindingTypes::Image {
			format: "rgba16".to_string(),
		},
		0,
		true,
		false,
	);
	program.binding(
		"result",
		besl::BindingTypes::Image {
			format: "unknown".to_string(),
		},
		1,
		false,
		true,
	);
	program.compile(configuration.source).expect(configuration.syntax_error)
}
