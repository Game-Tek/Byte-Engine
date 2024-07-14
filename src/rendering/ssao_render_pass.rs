use besl::parser::Node;
use ghi::{BoundComputePipelineMode, CommandBufferRecording, DeviceAccesses, GraphicsHardwareInterface, Uses};
use resource_management::{asset::{asset_manager::AssetManager, material_asset_handler::ProgramGenerator}, image::Image, resource::resource_manager::ResourceManager, shader_generation::{ShaderGenerationSettings, ShaderGenerator}, Reference};
use core::{Entity, EntityHandle};

use utils::{Extent, RGBA};

use super::common_shader_generator::CommonShaderGenerator;

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

const CAMERA_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
const DEPTH_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
const SOURCE_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
const RESULT_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const NOISE_BINDING_TEMPLATE: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(3, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);

impl ScreenSpaceAmbientOcclusionPass {
	pub async fn new(ghi: &mut ghi::GHI, resource_manager: EntityHandle<ResourceManager>, parent_descriptor_set_layout: ghi::DescriptorSetTemplateHandle, occlusion_target: ghi::ImageHandle, depth_target: ghi::ImageHandle) -> ScreenSpaceAmbientOcclusionPass {
		let descriptor_set_layout = ghi.create_descriptor_set_template(Some("HBAO Pass Set Layout"), &[DEPTH_BINDING_TEMPLATE.clone(), SOURCE_BINDING_TEMPLATE.clone(), RESULT_BINDING_TEMPLATE.clone(), NOISE_BINDING_TEMPLATE.clone()]);

		let pipeline_layout = ghi.create_pipeline_layout(&[parent_descriptor_set_layout, descriptor_set_layout], &[]);

		let descriptor_set = ghi.create_descriptor_set(Some("HBAO Descriptor Set"), &descriptor_set_layout);
		let blur_x_descriptor_set = ghi.create_descriptor_set(Some("HBAO Blur X Descriptor Set"), &descriptor_set_layout);
		let blur_y_descriptor_set = ghi.create_descriptor_set(Some("HBAO Blur Y Descriptor Set"), &descriptor_set_layout);

		let sampler = ghi.create_sampler(ghi::FilteringModes::Linear, ghi::SamplingReductionModes::WeightedAverage, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32);

		let depth_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DEPTH_BINDING_TEMPLATE, depth_target, sampler, ghi::Layouts::Read));
		let result_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&RESULT_BINDING_TEMPLATE, occlusion_target, ghi::Layouts::General));

		let x_blur_target = ghi.create_image(Some("X Blur"), Extent::new(1920, 1080, 1), ghi::Formats::R8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

		let blur_x_depth_binding = ghi.create_descriptor_binding(blur_x_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DEPTH_BINDING_TEMPLATE, depth_target, sampler, ghi::Layouts::Read));
		let blur_x_source_binding = ghi.create_descriptor_binding(blur_x_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&SOURCE_BINDING_TEMPLATE, occlusion_target, sampler, ghi::Layouts::Read));
		let blur_x_result_binding = ghi.create_descriptor_binding(blur_x_descriptor_set, ghi::BindingConstructor::image(&RESULT_BINDING_TEMPLATE, x_blur_target, ghi::Layouts::General));

		let blur_y_depth_binding = ghi.create_descriptor_binding(blur_y_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DEPTH_BINDING_TEMPLATE, depth_target, sampler, ghi::Layouts::Read));
		let blur_y_source_binding = ghi.create_descriptor_binding(blur_y_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&SOURCE_BINDING_TEMPLATE, x_blur_target, sampler, ghi::Layouts::Read));
		let blur_y_result_binding = ghi.create_descriptor_binding(blur_y_descriptor_set, ghi::BindingConstructor::image(&RESULT_BINDING_TEMPLATE, occlusion_target, ghi::Layouts::General));

		let shader = ghi.create_shader(Some("HBAO Shader"), ghi::ShaderSource::GLSL(get_source()), ghi::ShaderTypes::Compute, &[
			CAMERA_BINDING_TEMPLATE.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			DEPTH_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
			RESULT_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
			NOISE_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
		]).expect("Failed to create SSAO shader");

		let pipeline = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute,));

		let blur_shader = ghi.create_shader(Some("SSAO Blur Shader"), ghi::ShaderSource::GLSL(BLUR_SHADER.to_string()), ghi::ShaderTypes::Compute, &[
			CAMERA_BINDING_TEMPLATE.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			DEPTH_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
			SOURCE_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
			RESULT_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
		]).expect("Failed to create SSAO blur shader");

		let blur_x_pipeline = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&blur_shader, ghi::ShaderTypes::Compute).with_specialization_map(&[ghi::SpecializationMapEntry::new(0, "vec2f".to_string(), [1f32, 0f32,])]));
		let blur_y_pipeline = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&blur_shader, ghi::ShaderTypes::Compute).with_specialization_map(&[ghi::SpecializationMapEntry::new(0, "vec2f".to_string(), [0f32, 1f32,])]));

		let resource_manager = resource_manager.read_sync();

		let mut blue_noise = resource_manager.request::<Image>("stbn_unitvec3_2Dx1D_128x128x64_0.png").await.unwrap();

		let format = ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized);

		let image = ghi.create_image(blue_noise.id().into(), blue_noise.resource().extent.into(), format, Uses::Image, DeviceAccesses::GpuRead | DeviceAccesses::CpuWrite, ghi::UseCases::STATIC);
		let sampler = ghi.create_sampler(ghi::FilteringModes::Closest, ghi::SamplingReductionModes::WeightedAverage, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Repeat, None, 0f32, 0f32);

		let buffer = ghi.get_texture_slice_mut(image);

		blue_noise.load(buffer.into()).await;

		let noise_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&NOISE_BINDING_TEMPLATE, image, sampler, ghi::Layouts::Read));

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

	pub fn render(&self, command_buffer_recording: &mut impl ghi::CommandBufferRecording, extent: Extent) {
		command_buffer_recording.start_region("SSAO");
		command_buffer_recording.clear_images(&[(self.result, ghi::ClearValue::Color(RGBA::white())),]);
		if true {
			command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]).bind_compute_pipeline(&self.pipeline).dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));
			command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.blur_x_descriptor_set]).bind_compute_pipeline(&self.blur_x_pipeline).dispatch(ghi::DispatchExtent::new(extent, Extent::line(128)));
			command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.blur_y_descriptor_set]).bind_compute_pipeline(&self.blur_y_pipeline).dispatch(ghi::DispatchExtent::new(extent, Extent::line(128)));
		}
		command_buffer_recording.end_region();
	}

	pub fn resize(&mut self, ghi: &mut ghi::GHI, extent: Extent) {
		ghi.resize_image(self.x_blur_target, extent);
	}
}

impl Entity for ScreenSpaceAmbientOcclusionPass {}

const BLUR_SHADER: &'static str = include_str!("../../assets/engine/shaders/blur.comp");

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
	Camera camera = camera.camera;

	vec3 p = get_view_space_position_from_depth(depth_map, uv, camera.inverse_projection_matrix);

	/* Sample neighboring pixels */
    vec3 pr = get_view_space_position_from_depth(depth_map, uv + (render_target_pixel_size * vec2( 1, 0)), camera.inverse_projection_matrix);
    vec3 pl = get_view_space_position_from_depth(depth_map, uv + (render_target_pixel_size * vec2(-1, 0)), camera.inverse_projection_matrix);
    vec3 pt = get_view_space_position_from_depth(depth_map, uv + (render_target_pixel_size * vec2( 0, 1)), camera.inverse_projection_matrix);
    vec3 pb = get_view_space_position_from_depth(depth_map, uv + (render_target_pixel_size * vec2( 0,-1)), camera.inverse_projection_matrix);

    /* Calculate tangent basis vectors using the minimu difference */
    vec3 dPdu = min_diff(p, pr, pl);
    vec3 dPdv = min_diff(p, pt, pb);
	UVDerivatives derivatives = UVDerivatives(dPdu, dPdv);

    /* Get the random samples from the noise texture */
	vec3 random = texture(noise_texture, uv * noise_scale).rgb;

	/* Calculate the projected size of the hemisphere */
    vec2 uv_ray_radius = 0.5 * R * camera.fov / p.z;
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
			S = get_view_space_position_from_depth(depth_map, uv, camera.camera.inverse_projection_matrix);
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
	"#, &["out_ao", "depth_map", "camera", "get_view_space_position_from_depth", "biased_tangent", "sin_from_tan", "snap_uv", "tangent", "vec3f_squared_length"], Vec::new())]);

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

	let camera_binding = Node::binding("camera", Node::buffer("CameraBuffer", vec![Node::member("camera", "Camera")]), 0, CAMERA_BINDING_TEMPLATE.binding(), true, false);
	let out_ao = Node::binding("out_ao", Node::image("r8"), 1, RESULT_BINDING_TEMPLATE.binding(), false, true);
	let depth = Node::binding("depth_map",besl::parser::Node::combined_image_sampler(), 1, DEPTH_BINDING_TEMPLATE.binding(), true, false);
	let noise_texture_binding = besl::parser::Node::binding("noise_texture", besl::parser::Node::combined_image_sampler(), 1, NOISE_BINDING_TEMPLATE.binding(), true, false);
	let main = besl::parser::Node::function("main", Vec::new(), "void", vec![besl::parser::Node::glsl(main_code, &["noise_texture", "out_ao", "depth_map", "camera", "compute_trace", "min_diff", "get_view_space_position_from_depth", "compute_occlusion", "UVDerivatives", "rotate_directions"], Vec::new())]);

	let root_node = besl::parser::Node::root();

	use json::object;

	let mut root = shader_generator.transform(root_node, &object! {});

	root.add(vec![camera_binding, noise_texture_binding, depth, out_ao, biased_tangent, trace_settings_struct, compute_trace, compute_occlusion, main]);

	let root_node = besl::lex(root).unwrap();

	let main_node = root_node.borrow().get_main().unwrap();

	let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&ShaderGenerationSettings::compute(Extent::square(32)), &main_node);

	glsl
}