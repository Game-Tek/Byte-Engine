use std::borrow::Borrow;

use crate::core::EntityHandle;

use ghi::{Device, CommandBufferRecordable, BoundComputePipelineMode};
use utils::Extent;

use crate::core::{orchestrator::{self,}, Entity, entity::EntityBuilder};

use super::{render_pass::{RenderPass, RenderPassBuilder}, tonemap_render_pass};

pub struct AcesToneMapPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	descriptor_set: ghi::DescriptorSetHandle,
}

const SOURCE_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const DESTINATION_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

impl Entity for AcesToneMapPass {}

impl RenderPass for AcesToneMapPass {
	fn get_read_attachments() -> Vec<&'static str> where Self: Sized {
		vec!["main"]
	}

	fn get_write_attachments() -> Vec<&'static str> where Self: Sized {
		vec!["result"]
	}

	fn create<'a>(render_pass_builder: &mut RenderPassBuilder<'a>) -> EntityBuilder<'static, Self> where Self: Sized {
		let read_from_main = render_pass_builder.read_from("main");
		let render_to_main = render_pass_builder.render_to("result");

		let ghi = render_pass_builder.ghi();
		let mut ghi = ghi.write();

		let descriptor_set_layout = ghi.create_descriptor_set_template(Some("Tonemap Pass Set Layout"), &[SOURCE_BINDING_TEMPLATE, DESTINATION_BINDING_TEMPLATE]);

		let pipeline_layout = ghi.create_pipeline_layout(&[descriptor_set_layout], &[]);

		let descriptor_set = ghi.create_descriptor_set(Some("Tonemap Pass Descriptor Set"), &descriptor_set_layout);

		let source_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&SOURCE_BINDING_TEMPLATE, read_from_main.into(), ghi::Layouts::General));
		let destination_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&DESTINATION_BINDING_TEMPLATE, render_to_main.into(), ghi::Layouts::General));

		let tone_mapping_shader = ghi.create_shader(Some("ACES Tone Mapping Compute Shader"), ghi::ShaderSource::GLSL(TONE_MAPPING_SHADER.to_string()), ghi::ShaderTypes::Compute, &[
			SOURCE_BINDING_TEMPLATE.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			DESTINATION_BINDING_TEMPLATE.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
		]).expect("Failed to create tone mapping shader");

		let tone_mapping_pipeline = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&tone_mapping_shader, ghi::ShaderTypes::Compute,));

		AcesToneMapPass {
			descriptor_set_layout,
			pipeline_layout,
			descriptor_set,
			pipeline: tone_mapping_pipeline,
		}.into()
	}

	fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: Extent, attachments: &[ghi::AttachmentInformation]) {
		command_buffer_recording.start_region("Tonemap");
		let r = command_buffer_recording.bind_compute_pipeline(&self.pipeline);
		r.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
		r.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));
		command_buffer_recording.end_region();
	}
}

const TONE_MAPPING_SHADER: &'static str = r#"
#version 450
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable

layout(set=0, binding=0, rgba16) uniform readonly image2D source;
layout(set=0, binding=1, rgba8) uniform image2D result;

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
