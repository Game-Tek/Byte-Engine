use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	context::{Context as _, ContextCreate as _},
};
use resource_management::{
	resources::material, shader::generator::ShaderGenerationSettings, types::ShaderTypes as ResourceShaderTypes,
};
use utils::Extent;

use crate::rendering::{
	render_pass::{RenderPassBuilder, RenderPassReturn},
	shader_store::{ShaderSourceDefinition, ShaderSourceDescriptor},
	Sink,
};

pub(super) const SOURCE_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub(super) const DESTINATION_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

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
	pub entry_point_error: &'static str,
}

/// The `Pipeline` struct provides tonemap wrappers with the shared compute pipeline and descriptor interface.
#[derive(Clone)]
pub(super) struct Pipeline {
	pub pipeline: ghi::PipelineHandle,
	pub descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
}

impl Pipeline {
	/// Creates the canonical two-image tonemap pipeline for one algorithm configuration.
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>, configuration: &Configuration) -> Self {
		let descriptor_set_layout = render_pass_builder.context().create_descriptor_set_template(
			Some(configuration.set_layout_name),
			&[SOURCE_BINDING_TEMPLATE, DESTINATION_BINDING_TEMPLATE],
		);
		let shader = render_pass_builder
			.create_shader(&ShaderSourceDescriptor {
				id: configuration.shader_id,
				name: configuration.shader_name,
				stage: ResourceShaderTypes::Compute,
				source: ShaderSourceDefinition::Besl {
					settings: ShaderGenerationSettings::compute(Extent::square(32))
						.name(configuration.settings_name.to_string()),
					main_node: create_program(configuration),
				},
				interface: material::ShaderInterface {
					workgroup_size: Some((32, 32, 1)),
					bindings: vec![
						material::Binding::new(0, 0, true, false),
						material::Binding::new(0, 1, false, true),
					],
				},
			})
			.expect(configuration.shader_error);
		let pipeline = render_pass_builder
			.context()
			.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
				&[descriptor_set_layout],
				&[],
				ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
			));

		Self {
			pipeline,
			descriptor_set_layout,
		}
	}
}

/// Creates the canonical source and destination bindings for one tonemap pass instance.
pub(super) fn create_descriptor_set(
	render_pass_builder: &mut RenderPassBuilder<'_>,
	pipeline: &Pipeline,
	configuration: &Configuration,
) -> ghi::DescriptorSetHandle {
	let source = render_pass_builder.read_from("main");
	let destination = render_pass_builder.render_to_swapchain();
	let context = render_pass_builder.context();
	let descriptor_set =
		context.create_descriptor_set(Some(configuration.descriptor_set_name), &pipeline.descriptor_set_layout);
	let _source_binding = context.create_descriptor_binding(
		descriptor_set,
		ghi::BindingConstructor::image(&SOURCE_BINDING_TEMPLATE, source),
	);
	let _destination_binding = context.create_descriptor_binding(
		descriptor_set,
		ghi::BindingConstructor::swapchain(&DESTINATION_BINDING_TEMPLATE, destination),
	);
	descriptor_set
}

/// Builds the shared BESL image interface around one tonemap algorithm.
pub(super) fn create_program(configuration: &Configuration) -> besl::NodeReference {
	let mut root = besl::Node::root();
	root.add_child(
		besl::Node::binding(
			"source",
			besl::BindingTypes::Image {
				format: "rgba16".to_string(),
			},
			0,
			0,
			true,
			false,
		)
		.into(),
	);
	root.add_child(
		besl::Node::binding(
			"result",
			besl::BindingTypes::Image {
				format: "unknown".to_string(),
			},
			0,
			1,
			false,
			true,
		)
		.into(),
	);

	let program = besl::compile_to_besl(configuration.source, Some(root)).expect(configuration.syntax_error);
	program.get_main().expect(configuration.entry_point_error)
}

/// Records the shared tonemap compute dispatch for a per-view descriptor set.
pub(super) fn prepare<'a>(
	pipeline: ghi::PipelineHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	sink: &Sink,
	frame_allocator: &'a bumpalo::Bump,
) -> Option<RenderPassReturn<'a>> {
	let extent = sink.extent();
	Some(crate::rendering::render_pass::allocate_render_command(
		frame_allocator,
		move |command_buffer, _| {
			command_buffer.region(
				|label| label.write_str("Tonemap"),
				|command_buffer| {
					let recording = command_buffer.bind_compute_pipeline(pipeline);
					recording.bind_descriptor_sets(&[descriptor_set]);
					recording.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));
				},
			);
		},
	))
}
