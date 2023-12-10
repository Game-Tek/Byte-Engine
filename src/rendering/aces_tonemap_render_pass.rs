use crate::{Extent, orchestrator::{self, Entity, System}};

use super::{render_system, tonemap_render_pass};

pub struct AcesToneMapPass {
	pipeline_layout: render_system::PipelineLayoutHandle,
	pipeline: render_system::PipelineHandle,
	descriptor_set_layout: render_system::DescriptorSetTemplateHandle,
	descriptor_set: render_system::DescriptorSetHandle,

	source_image_handle: render_system::ImageHandle,
	result_image_handle: render_system::ImageHandle,
}

impl AcesToneMapPass {
    fn new(render_system: &mut dyn render_system::RenderSystem, source_image: render_system::ImageHandle, result_image: render_system::ImageHandle) -> AcesToneMapPass {
        let bindings = [
			render_system::DescriptorSetBindingTemplate::new(0, render_system::DescriptorType::StorageImage, render_system::Stages::COMPUTE),
			render_system::DescriptorSetBindingTemplate::new(1, render_system::DescriptorType::StorageImage, render_system::Stages::COMPUTE),
		];

		let descriptor_set_layout = render_system.create_descriptor_set_template(Some("Tonemap Pass Set Layout"), &bindings);

		let pipeline_layout = render_system.create_pipeline_layout(&[descriptor_set_layout], &[]);

		let descriptor_set = render_system.create_descriptor_set(Some("Tonemap Pass Descriptor Set"), &descriptor_set_layout);

		let albedo_binding = render_system.create_descriptor_binding(descriptor_set, &bindings[0]);
		let result_binding = render_system.create_descriptor_binding(descriptor_set, &bindings[1]);

		render_system.write(&[
			render_system::DescriptorWrite {
				binding_handle: albedo_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::Image{ handle: source_image, layout: render_system::Layouts::General },
			},
			render_system::DescriptorWrite {
				binding_handle: result_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::Image{ handle: result_image, layout: render_system::Layouts::General },
			},
		]);

		let tone_mapping_shader = render_system.create_shader(render_system::ShaderSource::GLSL(TONE_MAPPING_SHADER), render_system::ShaderTypes::Compute,);
		let tone_mapping_pipeline = render_system.create_compute_pipeline(&pipeline_layout, (&tone_mapping_shader, render_system::ShaderTypes::Compute, vec![]));

		AcesToneMapPass {
			descriptor_set_layout,
			pipeline_layout,
			descriptor_set,
			pipeline: tone_mapping_pipeline,

			source_image_handle: source_image,
			result_image_handle: result_image,
		}
    }

	pub fn new_as_system(render_system: &mut dyn render_system::RenderSystem, source_image: render_system::ImageHandle, result_image: render_system::ImageHandle) -> orchestrator::EntityReturn<Self> {
		orchestrator::EntityReturn::new_from_function(move |orchestrator| {
			AcesToneMapPass::new(render_system, source_image, result_image)
		})
	}
}

impl tonemap_render_pass::ToneMapRenderPass for AcesToneMapPass {
	fn render(&self, command_buffer_recording: &mut dyn render_system::CommandBufferRecording,) {
		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Image(self.source_image_handle),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Image(self.result_image_handle),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);

		command_buffer_recording.bind_compute_pipeline(&self.pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
		command_buffer_recording.dispatch(render_system::DispatchExtent { workgroup_extent: Extent::square(32), dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });
	}
}

impl Entity for AcesToneMapPass {}
impl System for AcesToneMapPass {}

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