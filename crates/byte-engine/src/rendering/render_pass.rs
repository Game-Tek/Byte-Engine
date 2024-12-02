use core::EntityHandle;

use ghi::{BoundComputePipelineMode, CommandBufferRecordable, GraphicsHardwareInterface};
use maths_rs::Vec2f;
use utils::Extent;

pub trait RenderPass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>);

	fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {}
	fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: Extent);

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent);
}

pub struct FullScreenRenderPass {
	pipeline: ghi::PipelineHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	pipeline_layout: ghi::PipelineLayoutHandle,
}

impl FullScreenRenderPass {
	pub fn new(ghi: &mut ghi::GHI, shader: &str, bindings: &[ghi::DescriptorSetBindingTemplate], (source_image, source_sampler): &(ghi::ImageHandle, ghi::SamplerHandle), destination_image: ghi::ImageHandle) -> FullScreenRenderPass {
		let descriptor_set_layout = ghi.create_descriptor_set_template(Some("Fullscreen Pass Set Layout"), bindings);
		let pipeline_layout = ghi.create_pipeline_layout(&[descriptor_set_layout], &[]);

		let descriptor_set = ghi.create_descriptor_set(Some("Fullscreen Pass Descriptor Set"), &descriptor_set_layout);
		
		let source_image_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&bindings[0], *source_image, *source_sampler, ghi::Layouts::Read));
		let destination_image_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&bindings[1], destination_image, ghi::Layouts::General));
		
		ghi.write(&[ghi::DescriptorWrite::combined_image_sampler(source_image_binding, *source_image, *source_sampler, ghi::Layouts::Read), ghi::DescriptorWrite::image(destination_image_binding, destination_image, ghi::Layouts::General)]);

		let shader = ghi.create_shader(Some("Fullscreen Pass Shader"), ghi::ShaderSource::GLSL(shader.to_string()), ghi::ShaderTypes::Compute, &[bindings[0].into_shader_binding_descriptor(0, ghi::AccessPolicies::READ), bindings[1].into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE)]).expect("Failed to create fullscreen shader");

		let pipeline = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute,));

		FullScreenRenderPass {
			pipeline,
			descriptor_set,
			pipeline_layout,
		}
	}
}

impl RenderPass for FullScreenRenderPass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {
		unimplemented!()
	}

	fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: Extent) {
		command_buffer_recording.region("Downsample", |command_buffer_recording: &mut ghi::CommandBufferRecording<'_>| {
			command_buffer_recording.bind_compute_pipeline(&self.pipeline);
			command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
			command_buffer_recording.dispatch(ghi::DispatchExtent::new(extent, Extent::square(16)));
		});
	}

	fn resize(&self, _ghi: &mut ghi::GHI, _extent: Extent) {}
}

const BLUR_DEPTH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
const BLUR_SOURCE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
const BLUR_RESULT_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

pub struct BilateralBlurPass {
	pipeline_x: ghi::PipelineHandle,
	pipeline_y: ghi::PipelineHandle,
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set_x: ghi::DescriptorSetHandle,
	descriptor_set_y: ghi::DescriptorSetHandle,
}

impl BilateralBlurPass {
	pub async fn new<'c>(ghi: &mut ghi::GHI, (depth_image, depth_sampler): (ghi::ImageHandle, ghi::SamplerHandle), source_map: ghi::ImageHandle, x_blur_map: ghi::ImageHandle, y_blur_map: ghi::ImageHandle) -> Self {
		let descriptor_set_template = ghi.create_descriptor_set_template(Some("SSGI Blur"), &[BLUR_DEPTH_BINDING, BLUR_SOURCE_BINDING, BLUR_RESULT_BINDING]);

		let pipeline_layout = ghi.create_pipeline_layout(&[descriptor_set_template], &[]);

		let descriptor_set_x = ghi.create_descriptor_set(Some("X SSGI Blur"), &descriptor_set_template);
		let descriptor_set_y = ghi.create_descriptor_set(Some("Y SSGI Blur"), &descriptor_set_template);

		let sampler = ghi.build_sampler(ghi::sampler::Builder::new());

		ghi.create_descriptor_binding(descriptor_set_x, ghi::BindingConstructor::combined_image_sampler(&BLUR_DEPTH_BINDING, depth_image, depth_sampler, ghi::Layouts::Read));
		ghi.create_descriptor_binding(descriptor_set_x, ghi::BindingConstructor::combined_image_sampler(&BLUR_SOURCE_BINDING, source_map, sampler, ghi::Layouts::Read));
		ghi.create_descriptor_binding(descriptor_set_x, ghi::BindingConstructor::image(&BLUR_RESULT_BINDING, x_blur_map, ghi::Layouts::General));

		ghi.create_descriptor_binding(descriptor_set_y, ghi::BindingConstructor::combined_image_sampler(&BLUR_DEPTH_BINDING, depth_image, depth_sampler, ghi::Layouts::Read));
		ghi.create_descriptor_binding(descriptor_set_y, ghi::BindingConstructor::combined_image_sampler(&BLUR_SOURCE_BINDING, x_blur_map, sampler, ghi::Layouts::Read));
		ghi.create_descriptor_binding(descriptor_set_y, ghi::BindingConstructor::image(&BLUR_RESULT_BINDING, y_blur_map, ghi::Layouts::General));

		let shader = ghi.create_shader(Some("SSGI Blur"), ghi::ShaderSource::GLSL(BLUR_SHADER.into()), ghi::ShaderTypes::Compute, &vec![BLUR_DEPTH_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ), BLUR_SOURCE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ), BLUR_RESULT_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE)]).expect("Failed to create the ray march shader.");
		let pipeline_x = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute).with_specialization_map(&[ghi::SpecializationMapEntry::new::<Vec2f>(0, "vec2f".to_string(), Vec2f::new(1f32, 0f32))]));
		let pipeline_y = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute).with_specialization_map(&[ghi::SpecializationMapEntry::new::<Vec2f>(0, "vec2f".to_string(), Vec2f::new(0f32, 1f32))]));

		BilateralBlurPass {
			pipeline_x,
			pipeline_y,
			pipeline_layout,
			descriptor_set_x,
			descriptor_set_y,
		}
	}
}

impl RenderPass for BilateralBlurPass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {
		unimplemented!()
	}
	
	fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {}

	fn record(&self, command_buffer: &mut ghi::CommandBufferRecording, extent: Extent) {
		command_buffer.region("Blur", |command_buffer| {
			let command_buffer = command_buffer.bind_compute_pipeline(&self.pipeline_x);
			command_buffer.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set_x]);
			command_buffer.dispatch(ghi::DispatchExtent::new(extent, Extent::line(128)));
			let command_buffer = command_buffer.bind_compute_pipeline(&self.pipeline_y);
			command_buffer.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set_y]);
			command_buffer.dispatch(ghi::DispatchExtent::new(extent, Extent::line(128)));
		});
	}

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {}
}

const BLUR_SHADER: &'static str = r#"
#version 460 core
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_shader_explicit_arithmetic_types: enable

layout(row_major) uniform; layout(row_major) buffer;

layout(set=0, binding=0) uniform sampler2D depth;
layout(set=0, binding=1) uniform sampler2D source;
layout(set=0, binding=2) uniform writeonly image2D result;

layout(constant_id=0) const float DIRECTION_X = 1;
layout(constant_id=1) const float DIRECTION_Y = 0;
const vec2 DIRECTION = vec2(DIRECTION_X, DIRECTION_Y);

const uint32_t M = 16;
const uint32_t SAMPLE_COUNT = M + 1;

const float OFFSETS[17] = float[17](
    -15.153610827558811,
    -13.184471765481433,
    -11.219917592867032,
    -9.260003189282239,
    -7.304547036499911,
    -5.353083811756559,
    -3.4048471718931532,
    -1.4588111840004858,
    0.48624268466894843,
    2.431625915613778,
    4.378621204796657,
    6.328357272092126,
    8.281739853232981,
    10.239385576926011,
    12.201613265873693,
    14.1684792568739,
    16
);

const float WEIGHTS[17] = float[17](
    6.531899156556559e-7,
    0.000014791298968627152,
    0.00021720986764341157,
    0.0020706559053401204,
    0.012826757713634169,
    0.05167714650813829,
    0.13552110360479683,
    0.23148784424126953,
    0.25764630768379954,
    0.18686497997661272,
    0.0882961181645837,
    0.027166770533840135,
    0.0054386298156352516,
    0.0007078187356988374,
    0.00005983099317322662,
    0.0000032814299066650715,
    1.0033704349693544e-7
);

float linearize_depth(float reversedDepth) {
    return (0.1 * 100.0) / (100.0 + reversedDepth * (0.1 - 100.0));
}

float gaussian_depth(float centerDepth, float sampleDepth) {
    float depthDiff = linearize_depth(centerDepth) - linearize_depth(sampleDepth);
	if (abs(depthDiff) > 0.001) { return 0.0; } else { return 1.0; }
    float adjustedDepthDiff = abs(depthDiff);
    
    return exp(-adjustedDepthDiff * adjustedDepthDiff / (2.0 * 0.0005 * 0.0005));
}

// The sourceTexture to be blurred MUST use linear filtering!
vec4 blur(in sampler2D sourceTexture, vec2 blurDirection, vec2 uv)
{
    vec4 result = vec4(0.0);
	float center_center = texture(depth, uv).r;
    for (int i = 0; i < SAMPLE_COUNT; ++i) {
        vec2 offset = blurDirection * OFFSETS[i] / vec2(textureSize(sourceTexture, 0));
		float depth_sample = texture(depth, uv + offset).r;
		float weight = WEIGHTS[i] * gaussian_depth(center_center, depth_sample);
		result += texture(source, uv + offset) * weight;
    }
    return result;
}

layout(local_size_x=128) in;
void main() {
	if (gl_GlobalInvocationID.x >= imageSize(result).x || gl_GlobalInvocationID.y >= imageSize(result).y) { return; }

	uvec2 texel = uvec2(gl_GlobalInvocationID.xy);
	vec2 uv = (vec2(texel) + vec2(0.5)) / vec2(imageSize(result).xy);

	float value = blur(source, DIRECTION, uv).r;

	imageStore(result, ivec2(gl_GlobalInvocationID.xy), vec4(vec3(value), 1.0));
}"#;