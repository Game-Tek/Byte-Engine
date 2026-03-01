use std::borrow::Borrow;

use crate::{core::EntityHandle, rendering::{Viewport, render_pass::{FramePrepare, RenderPassBuilder, RenderPass, RenderPassReturn}, view::View}};

use ghi::{command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _, CommonCommandBufferMode as _}, device::Device as _, Device as _, FrameKey};
use resource_management::glsl;
use utils::{Extent, Box};

use crate::core::{Entity};

#[derive(Clone)]
pub struct BaseAcesToneMapPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
}

const SOURCE_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const DESTINATION_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

impl Entity for BaseAcesToneMapPass {}

impl BaseAcesToneMapPass {
	pub fn new<'a>(render_pass_builder: &'a mut RenderPassBuilder<'_>) -> Self {
		let read_from_main = render_pass_builder.read_from("main");
		let render_to_main = render_pass_builder.render_to("result");

		let device = render_pass_builder.device();

		let descriptor_set_layout = device.create_descriptor_set_template(Some("Tonemap Pass Set Layout"), &[SOURCE_BINDING_TEMPLATE, DESTINATION_BINDING_TEMPLATE]);

		let pipeline_layout = device.create_pipeline_layout(&[descriptor_set_layout], &[]);

		let tonemapping_shader_artifact = glsl::compile(TONE_MAPPING_SHADER, "ACES Tonemapping").unwrap();

		let tone_mapping_shader = device.create_shader(Some("ACES Tone Mapping Compute Shader"), ghi::ShaderSource::SPIRV(tonemapping_shader_artifact.borrow().into()), ghi::ShaderTypes::Compute, [
			SOURCE_BINDING_TEMPLATE.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			DESTINATION_BINDING_TEMPLATE.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
		]).expect("Failed to create tone mapping shader");

		let tone_mapping_pipeline = device.create_compute_pipeline(pipeline_layout, ghi::ShaderParameter::new(&tone_mapping_shader, ghi::ShaderTypes::Compute,));

		Self {
			descriptor_set_layout,
			pipeline_layout,
			pipeline: tone_mapping_pipeline,
		}
	}
}

pub struct AcesToneMapPass {
	render_pass: BaseAcesToneMapPass,
	descriptor_set: ghi::DescriptorSetHandle,
}

impl AcesToneMapPass {
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let render_pass = BaseAcesToneMapPass::new(render_pass_builder);

		let read_from_main = render_pass_builder.read_from("main");
		let render_to_main = render_pass_builder.render_to("result");

		let device = render_pass_builder.device();

		let descriptor_set = device.create_descriptor_set(Some("Tonemap Pass Descriptor Set"), &render_pass.descriptor_set_layout);

		let source_binding = device.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&SOURCE_BINDING_TEMPLATE, read_from_main.into(), ghi::Layouts::General));
		let destination_binding = device.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&DESTINATION_BINDING_TEMPLATE, render_to_main.into(), ghi::Layouts::General));

		AcesToneMapPass {
			render_pass,
			descriptor_set,
		}
	}
}

impl Entity for AcesToneMapPass {}

impl RenderPass for AcesToneMapPass {
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

vec3 ACESNarkowicz(vec3 x) {
	const float a = 2.51;
	const float b = 0.03;
	const float c = 2.43;
	const float d = 0.59;
	const float e = 0.14;
	return clamp((x*(a*x+b))/(x*(c*x+d)+e), 0.0, 1.0);
}

const mat3 ACES_INPUT_MAT = mat3(
	vec3( 0.59719,  0.35458,  0.04823),
	vec3( 0.07600,  0.90834,  0.01566),
	vec3( 0.02840,  0.13383,  0.83777)
);

const mat3 ACES_OUTPUT_MAT = mat3(
	vec3( 1.60475, -0.53108, -0.07367),
	vec3(-0.10208,  1.10813, -0.00605),
	vec3(-0.00327, -0.07276,  1.07602)
);

vec3 RRTAndODTFit(vec3 v) {
	vec3 a = v * (v + 0.0245786) - 0.000090537;
	vec3 b = v * (0.983729 * v + 0.4329510) + 0.238081;
	return a / b;
}

vec3 ACESFitted(vec3 x) {
	return clamp(ACES_OUTPUT_MAT * RRTAndODTFit(ACES_INPUT_MAT * x), 0.0, 1.0);
}

layout(local_size_x=32, local_size_y=32) in;
void main() {
	if (gl_GlobalInvocationID.x >= imageSize(source).x || gl_GlobalInvocationID.y >= imageSize(source).y) { return; }

	vec4 source_color = imageLoad(source, ivec2(gl_GlobalInvocationID.xy));

	vec3 result_color = ACESNarkowicz(source_color.rgb);

	result_color = pow(result_color, vec3(1.0 / 2.2));

	imageStore(result, ivec2(gl_GlobalInvocationID.xy), vec4(result_color, 1.0));
}
"#;
