use besl::parser::Node;
use ghi::{BoundComputePipelineMode, CommandBufferRecordable, DeviceAccesses, GraphicsHardwareInterface, Uses};
use resource_management::{asset::{asset_manager::AssetManager, material_asset_handler::ProgramGenerator}, image::Image, resource::resource_manager::ResourceManager, shader_generation::{ShaderGenerationSettings, ShaderGenerator}, Reference};
use core::{Entity, EntityHandle};
use std::{rc::Rc, sync::Arc};

use utils::{json, sync::RwLock, Extent, RGBA};

use super::{common_shader_generator::CommonShaderGenerator, render_pass::{BilateralBlurPass, RenderPass}, texture_manager::TextureManager};

pub struct ScreenSpaceAmbientOcclusionPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	depth_binding: ghi::DescriptorSetBindingHandle,
	result: ghi::ImageHandle,

	x_blur_map: ghi::ImageHandle,
	y_blur_map: ghi::ImageHandle,

	blur: BilateralBlurPass,

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

		let depth_sampler = ghi.build_sampler(ghi::sampler::Builder::new().filtering_mode(ghi::FilteringModes::Closest).reduction_mode(ghi::SamplingReductionModes::Min).addressing_mode(ghi::SamplerAddressingModes::Mirror));

		let depth_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DEPTH_BINDING_TEMPLATE, depth_target, depth_sampler, ghi::Layouts::Read));
		let result_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&RESULT_BINDING_TEMPLATE, occlusion_target, ghi::Layouts::General));

		let x_blur_map = ghi.create_image(Some("X Blur"), Extent::new(1920, 1080, 1), ghi::Formats::R8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC, 1);
		let y_blur_map = ghi.create_image(Some("Y Blur"), Extent::new(1920, 1080, 1), ghi::Formats::R8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC, 1);

		let source = get_source();

		let shader = ghi.create_shader(Some("HBAO Shader"), ghi::ShaderSource::GLSL(source), ghi::ShaderTypes::Compute, &[
			VIEWS_BINDING_TEMPLATE.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			DEPTH_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
			RESULT_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
			NOISE_BINDING_TEMPLATE.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
		]).expect("Failed to create SSAO shader");

		let pipeline = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute,));

		let format = ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized);

		let noise_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&NOISE_BINDING_TEMPLATE, noise_texture, noise_sampler, ghi::Layouts::Read));

		let blur = BilateralBlurPass::new(&mut ghi, (depth_target, depth_sampler), occlusion_target, x_blur_map, occlusion_target).await;

		ScreenSpaceAmbientOcclusionPass {
			pipeline_layout,
			descriptor_set_layout,
			descriptor_set,
			pipeline,
			depth_binding,
			result: occlusion_target,
			x_blur_map,
			y_blur_map,

			blur,

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
		self.blur.record(command_buffer_recording, extent);
		command_buffer_recording.end_region();
	}

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {
		ghi.resize_image(self.x_blur_map, extent);
		ghi.resize_image(self.y_blur_map, extent);
	}
}

pub fn get_source() -> String {
	let shader_generator = {
		let common_shader_generator = CommonShaderGenerator::new_with_params(false, true, false, true, false, true, false, false);
		common_shader_generator
	};

	let main_code = r#"
	if (gl_GlobalInvocationID.x >= imageSize(out_ao).x || gl_GlobalInvocationID.y >= imageSize(out_ao).y) { return; }

	uint32_t direction_count = 8;
	float R = 0.3f;

	vec2 render_target_pixel_size = vec2(1.0f) / vec2(imageSize(out_ao));
	vec2 noise_scale = vec2(imageSize(out_ao)) / 128.0f; /* Scale by noise size */

	uvec2 texel = uvec2(gl_GlobalInvocationID.xy);
	vec2 uv = (vec2(texel) + vec2(0.5f)) / vec2(imageSize(out_ao));
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
    float pixel_ray_radius = uv_ray_radius.x * vec2(imageSize(out_ao)).x;

	
    /* Make sure the radius of the evaluated hemisphere is more than a pixel */
    if(pixel_ray_radius <= 1.0) {
		imageStore(out_ao, ivec2(texel), vec4(1.0));
		return;
	}
		
	float ao = 0.0;

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
		uint32_t step_count = min(8/* SAMPLE_COUNT */, uint32_t(pixel_ray_radius));

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
		return TraceSettings(step_count, stepSizePix * (vec2(1.0) / vec2(imageSize(out_ao))));
	"#, &["out_ao"], Vec::new())]);

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