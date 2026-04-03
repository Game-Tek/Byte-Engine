use std::borrow::Borrow as _;

use ghi::{
	command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _},
	device::{Device as _, DeviceCreate as _},
};
use math::Vector2;
use utils::{Box, Extent};

use crate::rendering::{
	render_pass::{FramePrepare, RenderPassBuilder, RenderPassReturn},
	RenderPass, Viewport,
};

const BLUR_DEPTH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const BLUR_SOURCE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	1,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const BLUR_RESULT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

#[derive(Clone)]
pub struct BaseBilateralBlurPass {
	pipeline_x: ghi::PipelineHandle,
	pipeline_y: ghi::PipelineHandle,
	descriptor_set_template: ghi::DescriptorSetTemplateHandle,
}

impl BaseBilateralBlurPass {
	fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let device = render_pass_builder.device();

		let descriptor_set_template = device.create_descriptor_set_template(
			Some("SSGI Blur"),
			&[BLUR_DEPTH_BINDING, BLUR_SOURCE_BINDING, BLUR_RESULT_BINDING],
		);

		let shader =
			resource_management::glsl::compile(BLUR_SHADER, "blur_shader").expect("Failed to compile the SSGI blur shader.");

		let shader = device
			.create_shader(
				Some("SSGI Blur"),
				ghi::shader::Sources::SPIRV(shader.as_binary_u8()),
				ghi::ShaderTypes::Compute,
				[
					BLUR_DEPTH_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					BLUR_SOURCE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					BLUR_RESULT_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
				],
			)
			.expect("Failed to create the ray march shader.");
		let pipeline_x = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[descriptor_set_template],
			&[],
			ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute).with_specialization_map(&[
				ghi::pipelines::SpecializationMapEntry::new(0, "vec2f".to_string(), Vector2::new(1f32, 0f32)),
			]),
		));
		let pipeline_y = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[descriptor_set_template],
			&[],
			ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute).with_specialization_map(&[
				ghi::pipelines::SpecializationMapEntry::new(0, "vec2f".to_string(), Vector2::new(0f32, 1f32)),
			]),
		));

		Self {
			pipeline_x,
			pipeline_y,
			descriptor_set_template,
		}
	}
}

struct BilateralBlurPass {
	render_pass: BaseBilateralBlurPass,
	descriptor_set_x: ghi::DescriptorSetHandle,
	descriptor_set_y: ghi::DescriptorSetHandle,
}

impl BilateralBlurPass {
	pub fn new(
		render_pass_builder: &mut RenderPassBuilder,
		render_pass: &BaseBilateralBlurPass,
		source: ghi::BaseImageHandle,
	) -> Self {
		let read_depth = render_pass_builder.read_from("depth");
		let depth_image: ghi::BaseImageHandle = (*read_depth.borrow()).into();

		let device = render_pass_builder.device();

		let descriptor_set_template = render_pass.descriptor_set_template;

		let descriptor_set_x = device.create_descriptor_set(Some("X SSGI Blur"), &descriptor_set_template);
		let descriptor_set_y = device.create_descriptor_set(Some("Y SSGI Blur"), &descriptor_set_template);

		let x_blur_map = device.build_image(ghi::image::Builder::new(
			ghi::Formats::RGB16UNORM,
			ghi::Uses::Image | ghi::Uses::Storage,
		));
		let y_blur_map = device.build_image(ghi::image::Builder::new(
			ghi::Formats::RGB16UNORM,
			ghi::Uses::Image | ghi::Uses::Storage,
		));

		let sampler = device.build_sampler(ghi::sampler::Builder::new());
		let depth_sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear),
		);

		device.create_descriptor_binding(
			descriptor_set_x,
			ghi::BindingConstructor::combined_image_sampler(
				&BLUR_DEPTH_BINDING,
				depth_image.clone(),
				depth_sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		device.create_descriptor_binding(
			descriptor_set_x,
			ghi::BindingConstructor::combined_image_sampler(&BLUR_SOURCE_BINDING, source, sampler.clone(), ghi::Layouts::Read),
		);
		device.create_descriptor_binding(
			descriptor_set_x,
			ghi::BindingConstructor::image(&BLUR_RESULT_BINDING, x_blur_map),
		);

		device.create_descriptor_binding(
			descriptor_set_y,
			ghi::BindingConstructor::combined_image_sampler(
				&BLUR_DEPTH_BINDING,
				depth_image.clone(),
				depth_sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		device.create_descriptor_binding(
			descriptor_set_y,
			ghi::BindingConstructor::combined_image_sampler(
				&BLUR_SOURCE_BINDING,
				x_blur_map,
				sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		device.create_descriptor_binding(
			descriptor_set_y,
			ghi::BindingConstructor::image(&BLUR_RESULT_BINDING, y_blur_map),
		);

		BilateralBlurPass {
			render_pass: render_pass.clone(),
			descriptor_set_x,
			descriptor_set_y,
		}
	}
}

impl RenderPass for BilateralBlurPass {
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> Option<RenderPassReturn> {
		let execute_in_axis = |command_buffer: &mut ghi::implementation::CommandBufferRecording,
		                       pipeline: ghi::PipelineHandle,
		                       descriptor_set: ghi::DescriptorSetHandle,
		                       extent: Extent| {
			let c = command_buffer.bind_compute_pipeline(pipeline);
			c.bind_descriptor_sets(&[descriptor_set]);
			c.dispatch(ghi::DispatchExtent::new(extent, Extent::line(128)));
		};

		let pipeline_x = self.render_pass.pipeline_x;
		let pipeline_y = self.render_pass.pipeline_y;
		let descriptor_set_x = self.descriptor_set_x;
		let descriptor_set_y = self.descriptor_set_y;

		let extent = viewport.extent();

		Some(Box::new(move |command_buffer, _| {
			command_buffer.region("Bilateral Blur", |command_buffer| {
				execute_in_axis(command_buffer, pipeline_x, descriptor_set_x, extent);
				execute_in_axis(command_buffer, pipeline_y, descriptor_set_y, extent);
			});
		}))
	}
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
