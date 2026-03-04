use std::borrow::Borrow;

use crate::{
	core::EntityHandle,
	rendering::{
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
		view::View,
		Viewport,
	},
};

use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
	},
	device::Device as _,
};
use resource_management::glsl;
use utils::{Box, Extent};

use crate::core::Entity;

/// The `BaseAgxToneMapPass` struct defines the shared GPU state required for AGX tonemapping.
#[derive(Clone)]
pub struct BaseAgxToneMapPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
}

const SOURCE_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const DESTINATION_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

impl Entity for BaseAgxToneMapPass {}

impl BaseAgxToneMapPass {
	/// Creates the shared AGX compute pipeline resources used by per-view tonemap passes.
	pub fn new<'a>(render_pass_builder: &'a mut RenderPassBuilder<'_>) -> Self {
		let read_from_main = render_pass_builder.read_from("main");
		let render_to_main = render_pass_builder.render_to("result");

		let device = render_pass_builder.device();

		let descriptor_set_layout = device.create_descriptor_set_template(
			Some("AGX Tonemap Pass Set Layout"),
			&[SOURCE_BINDING_TEMPLATE, DESTINATION_BINDING_TEMPLATE],
		);

		let pipeline_layout = device.create_pipeline_layout(&[descriptor_set_layout], &[]);

		let tonemapping_shader_artifact = glsl::compile(TONE_MAPPING_SHADER, "AGX Tonemapping").unwrap();

		let tone_mapping_shader = device
			.create_shader(
				Some("AGX Tone Mapping Compute Shader"),
				ghi::ShaderSource::SPIRV(tonemapping_shader_artifact.borrow().into()),
				ghi::ShaderTypes::Compute,
				[
					SOURCE_BINDING_TEMPLATE.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					DESTINATION_BINDING_TEMPLATE.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
				],
			)
			.expect("Failed to create AGX tone mapping shader");

		let tone_mapping_pipeline = device.create_compute_pipeline(
			pipeline_layout,
			ghi::ShaderParameter::new(&tone_mapping_shader, ghi::ShaderTypes::Compute),
		);

		Self {
			descriptor_set_layout,
			pipeline_layout,
			pipeline: tone_mapping_pipeline,
		}
	}
}

/// The `AgxToneMapPass` struct defines a per-view AGX tonemapping pass instance.
pub struct AgxToneMapPass {
	render_pass: BaseAgxToneMapPass,
	descriptor_set: ghi::DescriptorSetHandle,
}

impl AgxToneMapPass {
	/// Creates the per-view descriptor bindings for the AGX tonemap pass.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let render_pass = BaseAgxToneMapPass::new(render_pass_builder);

		let read_from_main = render_pass_builder.read_from("main");
		let render_to_main = render_pass_builder.render_to("result");

		let device = render_pass_builder.device();

		let descriptor_set =
			device.create_descriptor_set(Some("AGX Tonemap Pass Descriptor Set"), &render_pass.descriptor_set_layout);

		let source_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::image(&SOURCE_BINDING_TEMPLATE, read_from_main.into()),
		);
		let destination_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::image(&DESTINATION_BINDING_TEMPLATE, render_to_main.into()),
		);

		AgxToneMapPass {
			render_pass,
			descriptor_set,
		}
	}
}

impl Entity for AgxToneMapPass {}

impl RenderPass for AgxToneMapPass {
	fn prepare(&mut self, frame: &mut ghi::Frame, viewport: &Viewport) -> Option<RenderPassReturn> {
		let pipeline_layout = self.render_pass.pipeline_layout;
		let pipeline = self.render_pass.pipeline;
		let descriptor_set = self.descriptor_set;

		let extent = viewport.extent();

		Some(Box::new(move |c, _| {
			c.region("Tonemap", |c| {
				let c = c.bind_pipeline_layout(pipeline_layout);
				c.bind_descriptor_sets(&[descriptor_set]);
				let r = c.bind_compute_pipeline(pipeline);
				r.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));
			});
		}))
	}
}

const TONE_MAPPING_SHADER: &'static str = r#"
#version 450
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable

layout(set=0, binding=0, rgba16f) uniform readonly image2D source;
layout(set=0, binding=1, rgba8) uniform writeonly image2D result;

const mat3 LINEAR_REC2020_TO_LINEAR_SRGB = mat3(
	1.6605, -0.1246, -0.0182,
	-0.5876, 1.1329, -0.1006,
	-0.0728, -0.0083, 1.1187
);

const mat3 LINEAR_SRGB_TO_LINEAR_REC2020 = mat3(
	0.6274, 0.0691, 0.0164,
	0.3293, 0.9195, 0.0880,
	0.0433, 0.0113, 0.8956
);

const mat3 AGX_INSET_MATRIX = mat3(
	0.856627153315983, 0.137318972929847, 0.11189821299995,
	0.0951212405381588, 0.761241990602591, 0.0767994186031903,
	0.0482516061458583, 0.101439036467562, 0.811302368396859
);

const mat3 AGX_OUTSET_MATRIX = mat3(
	1.1271005818144368, -0.1413297634984383, -0.14132976349843826,
	-0.11060664309660323, 1.157823702216272, -0.11060664309660294,
	-0.016493938717834573, -0.016493938717834257, 1.2519364065950405
);

const float AGX_MIN_EV = -12.47393;
const float AGX_MAX_EV = 4.026069;

vec3 agx(vec3 color) {
	color = LINEAR_SRGB_TO_LINEAR_REC2020 * color;

	color = AGX_INSET_MATRIX * color;
	color = max(color, 1e-10);

	color = clamp(log2(color), AGX_MIN_EV, AGX_MAX_EV);
	color = (color - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV);
	color = clamp(color, 0.0, 1.0);

	vec3 x2 = color * color;
	vec3 x4 = x2 * x2;
	color = + 15.5 * x4 * x2
		- 40.14 * x4 * color
		+ 31.96 * x4
		- 6.868 * x2 * color
		+ 0.4298 * x2
		+ 0.1191 * color
		- 0.00232;

	color = AGX_OUTSET_MATRIX * color;
	color = pow(max(vec3(0.0), color), vec3(2.2));
	color = LINEAR_REC2020_TO_LINEAR_SRGB * color;

	return clamp(color, 0.0, 1.0);
}

layout(local_size_x=32, local_size_y=32) in;
void main() {
	if (gl_GlobalInvocationID.x >= imageSize(source).x || gl_GlobalInvocationID.y >= imageSize(source).y) { return; }

	vec4 source_color = imageLoad(source, ivec2(gl_GlobalInvocationID.xy));
	vec3 result_color = agx(source_color.rgb);

	imageStore(result, ivec2(gl_GlobalInvocationID.xy), vec4(result_color, 1.0));
}
"#;
