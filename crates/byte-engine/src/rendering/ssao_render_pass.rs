use besl::parser::Node;
use ghi::{BoundComputePipelineMode, CommandBufferRecordable, DeviceAccesses, GraphicsHardwareInterface, Uses};
use resource_management::{asset::{asset_manager::AssetManager, material_asset_handler::ProgramGenerator}, image::Image, resource::resource_manager::ResourceManager, shader_generation::{ShaderGenerationSettings, ShaderGenerator}, Reference};
use core::{Entity, EntityHandle};
use std::{rc::Rc, sync::Arc};

use utils::{json, sync::RwLock, Extent, RGBA};

use super::{common_shader_generator::CommonShaderGenerator, render_pass::RenderPass, texture_manager::TextureManager};

pub struct ScreenSpaceAmbientOcclusionPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,
	blur_x_pipeline: ghi::PipelineHandle,
	blur_y_pipeline: ghi::PipelineHandle,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	blur_x_descriptor_set: ghi::DescriptorSetHandle,
	blur_y_descriptor_set: ghi::DescriptorSetHandle,
	depth_binding: ghi::DescriptorSetBindingHandle,
	result: ghi::ImageHandle,
	x_blur_target: ghi::ImageHandle,

	// Not owned by this render pass
	depth_target: ghi::ImageHandle,
}

const VIEWS_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
const DEPTH_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
const SOURCE_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
const RESULT_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const NOISE_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(3, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);

impl ScreenSpaceAmbientOcclusionPass {
	pub async fn new(ghi_lock: Rc<RwLock<ghi::GHI>>, resource_manager: EntityHandle<ResourceManager>, texture_manager: Arc<utils::r#async::RwLock<TextureManager>>, parent_descriptor_set_layout: ghi::DescriptorSetTemplateHandle, occlusion_target: ghi::ImageHandle, depth_target: ghi::ImageHandle) -> ScreenSpaceAmbientOcclusionPass {
		let resource_manager = resource_manager.read_sync();

		let mut blue_noise = resource_manager.request::<Image>("stbn_unitvec3_2Dx1D_128x128x64_0.png").await.unwrap();

		let (_, noise_texture, noise_sampler) = texture_manager.write().await.load(&mut blue_noise, ghi_lock.clone()).await.unwrap();

		let mut ghi = ghi_lock.write();

		let descriptor_set_layout = ghi.create_descriptor_set_template(Some("HBAO Pass Set Layout"), &[DEPTH_BINDING_TEMPLATE.clone(), SOURCE_BINDING_TEMPLATE.clone(), RESULT_BINDING_TEMPLATE.clone(), NOISE_BINDING_TEMPLATE.clone()]);

		let pipeline_layout = ghi.create_pipeline_layout(&[parent_descriptor_set_layout, descriptor_set_layout], &[]);

		let descriptor_set = ghi.create_descriptor_set(Some("HBAO Descriptor Set"), &descriptor_set_layout);
		let blur_x_descriptor_set = ghi.create_descriptor_set(Some("HBAO Blur X Descriptor Set"), &descriptor_set_layout);
		let blur_y_descriptor_set = ghi.create_descriptor_set(Some("HBAO Blur Y Descriptor Set"), &descriptor_set_layout);

		let sampler = ghi.create_sampler(ghi::FilteringModes::Linear, ghi::SamplingReductionModes::WeightedAverage, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32);

		let depth_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DEPTH_BINDING_TEMPLATE, depth_target, sampler, ghi::Layouts::Read));
		let result_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&RESULT_BINDING_TEMPLATE, occlusion_target, ghi::Layouts::General));

		let x_blur_target = ghi.create_image(Some("X Blur"), Extent::new(1920, 1080, 1), ghi::Formats::R8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC, 1);

		let blur_x_depth_binding = ghi.create_descriptor_binding(blur_x_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DEPTH_BINDING_TEMPLATE, depth_target, sampler, ghi::Layouts::Read));
		let blur_x_source_binding = ghi.create_descriptor_binding(blur_x_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&SOURCE_BINDING_TEMPLATE, occlusion_target, sampler, ghi::Layouts::Read));
		let blur_x_result_binding = ghi.create_descriptor_binding(blur_x_descriptor_set, ghi::BindingConstructor::image(&RESULT_BINDING_TEMPLATE, x_blur_target, ghi::Layouts::General));

		let blur_y_depth_binding = ghi.create_descriptor_binding(blur_y_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DEPTH_BINDING_TEMPLATE, depth_target, sampler, ghi::Layouts::Read));
		let blur_y_source_binding = ghi.create_descriptor_binding(blur_y_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&SOURCE_BINDING_TEMPLATE, x_blur_target, sampler, ghi::Layouts::Read));
		let blur_y_result_binding = ghi.create_descriptor_binding(blur_y_descriptor_set, ghi::BindingConstructor::image(&RESULT_BINDING_TEMPLATE, occlusion_target, ghi::Layouts::General));

		let source = get_source();

		let shader = ghi.create_shader(Some("HBAO Shader"), ghi::ShaderSource::GLSL(source), ghi::ShaderTypes::Compute, &[
			VIEWS_BINDING_TEMPLATE.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			DEPTH_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
			RESULT_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
			NOISE_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
		]).expect("Failed to create SSAO shader");

		let pipeline = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute,));

		let blur_shader = ghi.create_shader(Some("SSAO Blur Shader"), ghi::ShaderSource::GLSL(BLUR_SHADER.to_string()), ghi::ShaderTypes::Compute, &[
			VIEWS_BINDING_TEMPLATE.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			DEPTH_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
			SOURCE_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
			RESULT_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
		]).expect("Failed to create SSAO blur shader");

		let blur_x_pipeline = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&blur_shader, ghi::ShaderTypes::Compute).with_specialization_map(&[ghi::SpecializationMapEntry::new(0, "vec2f".to_string(), [1f32, 0f32,])]));
		let blur_y_pipeline = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&blur_shader, ghi::ShaderTypes::Compute).with_specialization_map(&[ghi::SpecializationMapEntry::new(0, "vec2f".to_string(), [0f32, 1f32,])]));

		let format = ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized);

		let noise_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&NOISE_BINDING_TEMPLATE, noise_texture, sampler, ghi::Layouts::Read));

		ScreenSpaceAmbientOcclusionPass {
			pipeline_layout,
			descriptor_set_layout,
			descriptor_set,
			blur_x_descriptor_set,
			blur_y_descriptor_set,
			blur_x_pipeline,
			blur_y_pipeline,
			pipeline,
			depth_binding,
			result: occlusion_target,
			x_blur_target,

			depth_target,
		}
	}
}

impl Entity for ScreenSpaceAmbientOcclusionPass {}

impl RenderPass for ScreenSpaceAmbientOcclusionPass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {
		unimplemented!()
	}

	fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: Extent) {
		command_buffer_recording.start_region("SSAO");
		command_buffer_recording.clear_images(&[(self.result, ghi::ClearValue::Color(RGBA::white())),]);
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]).bind_compute_pipeline(&self.pipeline).dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.blur_x_descriptor_set]).bind_compute_pipeline(&self.blur_x_pipeline).dispatch(ghi::DispatchExtent::new(extent, Extent::line(128)));
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.blur_y_descriptor_set]).bind_compute_pipeline(&self.blur_y_pipeline).dispatch(ghi::DispatchExtent::new(extent, Extent::line(128)));
		command_buffer_recording.end_region();
	}

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {
		ghi.resize_image(self.x_blur_target, extent);
	}
}

const BLUR_SHADER: &'static str = r#"
#version 460 core
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_shader_explicit_arithmetic_types: enable

layout(row_major) uniform; layout(row_major) buffer;

layout(set=1, binding=0) uniform sampler2D depth;
layout(set=1, binding=1) uniform sampler2D source;
layout(set=1, binding=2) uniform writeonly image2D result;

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

// blurDirection is:
//     vec2(1,0) for horizontal pass
//     vec2(0,1) for vertical pass
// The sourceTexture to be blurred MUST use linear filtering!
vec4 blur(in sampler2D sourceTexture, vec2 blurDirection, vec2 uv)
{
    vec4 result = vec4(0.0);
	float center_depth = texture(depth, uv).r;
    for (int i = 0; i < SAMPLE_COUNT; ++i) {
        vec2 offset = blurDirection * OFFSETS[i] / vec2(textureSize(sourceTexture, 0));
		float depth_sample = texture(depth, uv + offset).r;
        result += texture(sourceTexture, uv + offset) * WEIGHTS[i] * exp(-abs(center_depth - depth_sample) * 10.0);
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

pub fn get_source() -> String {
	let shader_generator = {
		let common_shader_generator = CommonShaderGenerator::new_with_params(false, true, false, true, false, true, false, false);
		common_shader_generator
	};

	let main_code = r#"
	if (gl_GlobalInvocationID.x >= 1920 || gl_GlobalInvocationID.y >= 1080) { return; }

	uint32_t direction_count = 6;
	float R = 0.3f;

	vec2 render_target_extent = vec2(1920.0, 1080.0);
	vec2 render_target_pixel_size = vec2(1.0f) / render_target_extent;
	vec2 noise_scale = render_target_extent / 128.0f; /* Scale by noise size */

	uvec2 texel = uvec2(gl_GlobalInvocationID.xy);
	vec2 uv = (vec2(texel) + vec2(0.5f)) / vec2(1920, 1080);
	View view = views.views[0];

	vec3 p = get_view_space_position_from_depth(depth_map, uv, view.inverse_projection);

	/* Sample neighboring pixels */
    vec3 pr = get_view_space_position_from_depth(depth_map, uv + (render_target_pixel_size * vec2( 1, 0)), view.inverse_projection);
    vec3 pl = get_view_space_position_from_depth(depth_map, uv + (render_target_pixel_size * vec2(-1, 0)), view.inverse_projection);
    vec3 pt = get_view_space_position_from_depth(depth_map, uv + (render_target_pixel_size * vec2( 0, 1)), view.inverse_projection);
    vec3 pb = get_view_space_position_from_depth(depth_map, uv + (render_target_pixel_size * vec2( 0,-1)), view.inverse_projection);

    /* Calculate tangent basis vectors using the minimu difference */
    vec3 dPdu = min_diff(p, pr, pl);
    vec3 dPdv = min_diff(p, pt, pb);
	UVDerivatives derivatives = UVDerivatives(dPdu, dPdv);

    /* Get the random samples from the noise texture */
	vec3 random = texture(noise_texture, uv * noise_scale).rgb;

	/* Calculate the projected size of the hemisphere */
    vec2 uv_ray_radius = 0.5 * R * view.fov / p.z;
    float pixel_ray_radius = uv_ray_radius.x * render_target_extent.x;

    float ao = 1.0;

    /* Make sure the radius of the evaluated hemisphere is more than a pixel */
    if(pixel_ray_radius <= 1.0) {
		imageStore(out_ao, ivec2(texel), vec4(vec3(ao), 1.0));
		return;
	}

	ao = 0.0;

	TraceSettings trace = compute_trace(pixel_ray_radius, random.z);

	float alpha = 2.0 * PI / float(direction_count);

	/* Calculate the horizon occlusion of each direction */
	for(uint32_t d = 0; d < direction_count; ++d) {
		float theta = alpha * float(d);

		/* Apply noise to the direction */
		vec2 dir = rotate_directions(vec2(cos(theta), sin(theta)), random.xy);
		vec2 delta_uv = dir * trace.uv_step_size;

		/* Sample the pixels along the direction */
		ao += compute_occlusion(uv, delta_uv, p, derivatives, random.z, trace.step_count);
	}

	/* Average the results and produce the final AO */
	ao = 1.0 - ao / float(direction_count) * 3.0f;

	imageStore(out_ao, ivec2(texel), vec4(vec3(ao), 1.0));
	"#;

	let trace_settings_struct = besl::parser::Node::r#struct("TraceSettings", vec![besl::parser::Node::member("step_count", "u32"), besl::parser::Node::member("uv_step_size", "vec2f")]);

	// Compute the horizon based occlusion
	let compute_occlusion = besl::parser::Node::function("compute_occlusion", vec![Node::parameter("render_target_uv", "vec2f"), besl::parser::Node::parameter("delta_uv", "vec2f"), besl::parser::Node::parameter("p", "vec3f"), besl::parser::Node::parameter("derivatives", "UVDerivatives"), besl::parser::Node::parameter("randstep", "f32"), besl::parser::Node::parameter("step_count", "u32")], "f32", vec![Node::glsl(r#"
		float ao = 0;

		/* Offset the first coord with some noise */
		vec2 uv = render_target_uv + snap_uv(randstep * delta_uv, uvec2(imageSize(out_ao)));
		delta_uv = snap_uv(delta_uv, uvec2(imageSize(out_ao)));

		/* Calculate the tangent vector */
		vec3 T = delta_uv.x * derivatives.du + delta_uv.y * derivatives.dv;

		/* Get the angle of the tangent vector from the viewspace axis */
		float tanH = biased_tangent(T);
		float sinH = sin_from_tan(tanH);

		float tanS = 0;
		float d2 = 0;
		vec3 S = vec3(0);

		/* Sample to find the maximum angle */
		for(uint32_t s = 1; s <= step_count; ++s) {
			uv += delta_uv;
			S = get_view_space_position_from_depth(depth_map, uv, views.views[0].inverse_projection);
			tanS = tangent(p, S);
			d2 = vec3f_squared_length(S - p);

			/* Is the sample within the radius and the angle greater? */
			if(d2 < (0.3*0.3)/* R2 */ && tanS > tanH) {
				float sinS = sin_from_tan(tanS);
				/* Apply falloff based on the distance */
				ao += (d2 * (-1.0 / (0.3*0.3)) + 1.0f) * (sinS - sinH);

				tanH = tanS;
				sinH = sinS;
			}
		}
		
		return ao;
	"#, &["out_ao", "depth_map", "views", "get_view_space_position_from_depth", "biased_tangent", "sin_from_tan", "snap_uv", "tangent", "vec3f_squared_length"], Vec::new())]);

	// Compute the step size (in uv space) from the number of steps
	let compute_trace = besl::parser::Node::function("compute_trace", vec![besl::parser::Node::member("pixel_ray_radius", "f32"), besl::parser::Node::member("rand", "f32")], "TraceSettings", vec![Node::glsl(r#"
		/* Avoid oversampling if numSteps is greater than the kernel radius in pixels */
		uint32_t step_count = min(6/* SAMPLE_COUNT */, uint32_t(pixel_ray_radius));

		/* Divide by Ns+1 so that the farthest samples are not fully attenuated */
		float stepSizePix = pixel_ray_radius / (step_count + 1);

		float max_pixel_radius = 75.f; /* Tweak this for performance and effect */

		/* Clamp numSteps if it is greater than the max kernel footprint */
		float maxNumSteps = max_pixel_radius / stepSizePix;
		if (maxNumSteps < step_count) {
			/* Use dithering to avoid AO discontinuities */
			step_count = uint32_t(floor(maxNumSteps + rand));
			step_count = max(step_count, 1);
			stepSizePix = max_pixel_radius / step_count;
		}

		/* Step size in uv space */
		return TraceSettings(step_count, stepSizePix * (vec2(1.0) / vec2(1920, 1080)));
	"#, &[], Vec::new())]);

	let biased_tangent = Node::function("biased_tangent", vec![Node::parameter("v", "vec3f")], "f32", vec![Node::glsl("return -v.z * inversesqrt(dot(v,v)) + tan(30.0 * PI / 180.0)", &[], Vec::new())]);

	let views_binding = Node::binding("views", Node::buffer("ViewsBuffer", vec![Node::member("views", "View[8]")]), 0, VIEWS_BINDING_TEMPLATE.binding(), true, false);
	let out_ao = Node::binding("out_ao", Node::image("r8"), 1, RESULT_BINDING_TEMPLATE.binding(), false, true);
	let depth = Node::binding("depth_map",besl::parser::Node::combined_image_sampler(), 1, DEPTH_BINDING_TEMPLATE.binding(), true, false);
	let noise_texture_binding = besl::parser::Node::binding("noise_texture", besl::parser::Node::combined_image_sampler(), 1, NOISE_BINDING_TEMPLATE.binding(), true, false);
	let main = besl::parser::Node::function("main", Vec::new(), "void", vec![besl::parser::Node::glsl(main_code, &["noise_texture", "out_ao", "depth_map", "views", "compute_trace", "min_diff", "get_view_space_position_from_depth", "compute_occlusion", "UVDerivatives", "rotate_directions"], Vec::new())]);

	let root_node = besl::parser::Node::root();

	let mut root = shader_generator.transform(root_node, &json::object!{});

	root.add(vec![views_binding, noise_texture_binding, depth, out_ao, biased_tangent, trace_settings_struct, compute_trace, compute_occlusion, main]);

	let root_node = besl::lex(root).unwrap();

	let main_node = root_node.borrow().get_main().unwrap();

	let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&ShaderGenerationSettings::compute(Extent::square(32)), &main_node);

	glsl
}