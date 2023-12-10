use crate::Extent;

use super::{render_system, world_render_domain::WorldRenderDomain};

struct ShadowRenderingPass {
	pipeline: render_system::PipelineHandle,
	pipeline_layout: render_system::PipelineLayoutHandle,
	descriptor_set: render_system::DescriptorSetHandle,
	shadow_map: render_system::ImageHandle,
}

impl ShadowRenderingPass {
	fn new(render_system: &mut dyn render_system::RenderSystem, render_domain: &impl WorldRenderDomain) -> ShadowRenderingPass {
		let shadow_map_binding_template = render_system::DescriptorSetBindingTemplate::new(0, render_system::DescriptorType::StorageImage, render_system::Stages::MESH);
		let depth_binding_template = render_system::DescriptorSetBindingTemplate::new(1, render_system::DescriptorType::CombinedImageSampler, render_system::Stages::MESH);

		let bindings = [shadow_map_binding_template.clone(), depth_binding_template.clone()];

		let descriptor_set_template = render_system.create_descriptor_set_template(Some("Shadow Rendering Set Layout"), &bindings);

		let pipeline_layout = render_system.create_pipeline_layout(&[render_domain.get_descriptor_set_template(), descriptor_set_template], &[]);

		let descriptor_set = render_system.create_descriptor_set(Some("Shadow Rendering Descriptor Set"), &descriptor_set_template);

		let shadow_map_binding = render_system.create_descriptor_binding(descriptor_set, &shadow_map_binding_template);
		let depth_binding = render_system.create_descriptor_binding(descriptor_set, &depth_binding_template);

		let colored_shadow: bool = false;

		let shadow_map_resolution = Extent::square(4096);

		let shadow_map = render_system.create_image(Some("Shadow Map"), shadow_map_resolution, render_system::Formats::Depth32, None, render_system::Uses::Image, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

		render_system.write(&[
			render_system::DescriptorWrite {
				binding_handle: shadow_map_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::Image{ handle: shadow_map, layout: render_system::Layouts::General },
			},
		]);

		let ray_gen_shader = render_system.create_shader(render_system::ShaderSource::GLSL(SHADOW_RAY_GEN_SHADER), render_system::ShaderTypes::Raygen);
		let hit_shader = render_system.create_shader(render_system::ShaderSource::GLSL(SHADOW_HIT_SHADER), render_system::ShaderTypes::ClosestHit);
		let miss_shader = render_system.create_shader(render_system::ShaderSource::GLSL(SHADOW_MISS_SHADER), render_system::ShaderTypes::Miss);

		let pipeline = render_system.create_ray_tracing_pipeline(&pipeline_layout, &[
			(&ray_gen_shader, render_system::ShaderTypes::Raygen, vec![]),
			(&hit_shader, render_system::ShaderTypes::ClosestHit, vec![]),
			(&miss_shader, render_system::ShaderTypes::Miss, vec![]),
		]);

		ShadowRenderingPass { pipeline, pipeline_layout, descriptor_set, shadow_map }
	}

	fn render(&self, command_buffer_recording: &mut dyn render_system::CommandBufferRecording) {
		command_buffer_recording.start_region("Shadow Rendering");

		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Image(self.shadow_map),
				stages: render_system::Stages::MESH,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);

		command_buffer_recording.bind_raster_pipeline(&self.pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
		command_buffer_recording.dispatch_meshes(1, 1, 1);

		command_buffer_recording.end_region();
	}
}

const SHADOW_RAY_GEN_SHADER: &'static str = "
#version 460 core
#pragma shader_stage(raygen)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(row_major) uniform; layout(row_major) buffer;

struct Camera {
	mat4 view_matrix;
	mat4 projection_matrix;
	mat4 view_projection;
};

layout(set=0,binding=0,scalar) buffer readonly CameraBuffer {
	Camera camera;
};

layout(set=1,binding=0) 				uniform accelerationStructureEXT top_level_acceleration_structure;
layout(set=1,binding=1, r32ui) coherent uniform uimage2D shadow_map;
layout(set=1,binding=2) 				uniform sampler2D depth;

layout(location = 0) rayPayloadEXT float hit_distance;

mat3 matrix_from_direction_vector(vec3 d) {
	// TODO: check for colinearity
	vec3 u = cross(vec3(0.0, 1.0, 0.0), d);
	vec3 v = cross(d, u);
	return mat3(u, v, d);
}

vec3 get_view_position(uvec2 coords) {
	float depth_value = texelFetch(depth, ivec2(coords), 0).r;
	vec2 uv = (vec2(coords) + vec2(0.5)) / vec2(textureSize(depth, 0).xy);
	vec4 clip_space = vec4(uv * 2.0 - 1.0, depth_value, 1.0);
	vec4 view_space = inverse(camera.projection_matrix) * clip_space;
	view_space /= view_space.w;
	return view_space.xyz;
}

vec3 get_view_position(vec2 uv) {
	// snap to center of pixel
	uv *= textureSize(depth, 0).xy;
	uv = floor(uv) + vec2(0.5);
	uv /= textureSize(depth, 0).xy;
	float depth_value = texture(depth, uv).r;
	vec4 clip_space = vec4(uv * 2.0 - 1.0, depth_value, 1.0);
	vec4 view_space = inverse(camera.projection_matrix) * clip_space;
	view_space /= view_space.w;
	return view_space.xyz;
}

float length_squared(float v) { return v * v; }
float length_squared(vec2 v) { return dot(v, v); }
float length_squared(vec3 v) { return dot(v, v); }

vec3 min_diff(vec3 p, vec3 a, vec3 b) {
    vec3 ap = a - p;
    vec3 bp = p - b;
    return (length_squared(ap) < length_squared(bp)) ? ap : bp;
}

void main() {
	const vec2 pixel_center = vec2(gl_LaunchIDEXT.xy) + vec2(0.5);
	const vec2 uv = pixel_center / vec2(gl_LaunchSizeEXT.xy);
	vec2 d = uv * 2.0 - 1.0;

	uvec2 coords = uvec2(gl_LaunchIDEXT.xy);

	vec3 p = get_view_position(coords + uvec2(0, 0));
	vec3 pt = get_view_position(coords + uvec2(0, 1));
	vec3 pl = get_view_position(coords + uvec2(-1, 0));
	vec3 pr = get_view_position(coords + uvec2(1, 0));
	vec3 pb = get_view_position(coords + uvec2(0, -1));

	vec3 n = normalize(cross(min_diff(p, pr, pl), min_diff(p, pt, pb)));

	vec3 direction = vec3(0, 1, 0); // Overhead light
	vec3 position = get_view_position(uv);

	vec2 shadow_d = position.xz / vec2(4.0); // Assuming overhead light, map 8 x 8 area to shadow map
	vec2 shadow_uv = shadow_d * 0.5 + 0.5;
	ivec2 shadow_texel_coord = ivec2(shadow_uv * imageSize(shadow_map));

	if (dot(n, direction) <= 0.0) {
		imageAtomicMin(shadow_map, shadow_texel_coord, 0);
		return;
	}

	vec3 origin = position + n * 0.001; // Offset origin slightly to avoid self-intersection

	const float ray_distance = 10.0; // Maximum distance to check for intersection

	uint ray_flags = 0;
	uint cull_mask = 0xff;
	float t_min = 0.0f;
	float t_max = ray_distance;

	traceRayEXT(top_level_acceleration_structure, ray_flags, cull_mask, 0, 0, 0, origin, t_min, direction, t_max, 0);

	imageAtomicMin(shadow_map, shadow_texel_coord, (1 << 32) - 1);
}";

const SHADOW_HIT_SHADER: &'static str = "
#version 460 core
#pragma shader_stage(closest)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(row_major) uniform; layout(row_major) buffer;

layout(location = 0) rayPayloadInEXT float hit_distance;

void main() {
	hit_distance = gl_HitTEXT / gl_RayTmaxEXT;
}";

const SHADOW_MISS_SHADER: &'static str = "
#version 460 core
#pragma shader_stage(miss)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(row_major) uniform; layout(row_major) buffer;

layout(location = 0) rayPayloadInEXT float hit_distance;

void main() {
	hit_distance = 1.0f;
}";

struct ShadowMappingPass {
	pipeline_layout: render_system::PipelineLayoutHandle,
	pipeline: render_system::PipelineHandle,
	descriptor_set_template: render_system::DescriptorSetTemplateHandle,
	descriptor_set: render_system::DescriptorSetHandle,

	depth_target: render_system::ImageHandle,
	shadow_map: render_system::ImageHandle,
	occlusion_map: render_system::ImageHandle,
}

impl ShadowMappingPass {
	fn new(render_system: &mut dyn render_system::RenderSystem, shadow_rendering_pass: &ShadowRenderingPass, occlusion_map: render_system::ImageHandle, parent_descriptor_set_template: render_system::DescriptorSetTemplateHandle, depth_target: render_system::ImageHandle) -> ShadowMappingPass {
		let shadow_map_binding_template = render_system::DescriptorSetBindingTemplate::new(0, render_system::DescriptorType::CombinedImageSampler, render_system::Stages::COMPUTE);
		let depth_binding_template = render_system::DescriptorSetBindingTemplate::new(1, render_system::DescriptorType::CombinedImageSampler, render_system::Stages::COMPUTE);
		let result_binding_template = render_system::DescriptorSetBindingTemplate::new(2, render_system::DescriptorType::StorageImage, render_system::Stages::COMPUTE);

		let descriptor_set_template = render_system.create_descriptor_set_template(Some("Shadow Mapping Pass Set Layout"), &[shadow_map_binding_template.clone(), depth_binding_template.clone(), result_binding_template.clone()]);
		let pipeline_layout = render_system.create_pipeline_layout(&[parent_descriptor_set_template, descriptor_set_template], &[]);
		let descriptor_set = render_system.create_descriptor_set(Some("Shadow Mapping Pass Descriptor Set"), &descriptor_set_template);

		let shader = render_system.create_shader(render_system::ShaderSource::GLSL(BUILD_SHADOW_MAP_SHADER), render_system::ShaderTypes::Compute,);
		let pipeline = render_system.create_compute_pipeline(&pipeline_layout, (&shader, render_system::ShaderTypes::Compute, vec![]));

		let shadow_map_binding = render_system.create_descriptor_binding(descriptor_set, &shadow_map_binding_template);
		let depth_binding = render_system.create_descriptor_binding(descriptor_set, &depth_binding_template);
		let result_binding = render_system.create_descriptor_binding(descriptor_set, &result_binding_template);

		let sampler = render_system.create_sampler(render_system::FilteringModes::Linear, render_system::FilteringModes::Linear, render_system::SamplerAddressingModes::Clamp, None, 0f32, 0f32);

		render_system.write(&[
			render_system::DescriptorWrite {
				binding_handle: shadow_map_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::CombinedImageSampler { image_handle: shadow_rendering_pass.shadow_map, sampler_handle: sampler, layout: render_system::Layouts::Read },
			},
			render_system::DescriptorWrite {
				binding_handle: depth_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::CombinedImageSampler { image_handle: depth_target, sampler_handle: sampler, layout: render_system::Layouts::Read },
			},
			render_system::DescriptorWrite {
				binding_handle: result_binding,
				array_element: 0,
				descriptor: render_system::Descriptor::Image{ handle: occlusion_map, layout: render_system::Layouts::General },
			},
		]);

		ShadowMappingPass {
			pipeline_layout,
			pipeline,
			descriptor_set_template,
			descriptor_set,

			depth_target,
			shadow_map: shadow_rendering_pass.shadow_map,
			occlusion_map,
		}
	}

	fn render(&self, command_buffer_recording: &mut dyn render_system::CommandBufferRecording) {
		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Image(self.shadow_map),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::Read,
			},
			render_system::Consumption{
				handle: render_system::Handle::Image(self.depth_target),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::Read,
			},
			render_system::Consumption{
				handle: render_system::Handle::Image(self.occlusion_map),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);

		command_buffer_recording.bind_compute_pipeline(&self.pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
		command_buffer_recording.dispatch(render_system::DispatchExtent::new(Extent::plane(1920, 1080), Extent::square(32)));
	}
}

const BUILD_SHADOW_MAP_SHADER: &'static str = "
#version 460 core
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(row_major) uniform; layout(row_major) buffer;

struct Camera {
	mat4 view_matrix;
	mat4 projection_matrix;
	mat4 view_projection;
};

layout(set=0,binding=0,scalar) buffer readonly CameraBuffer {
	Camera camera;
};

layout(set=1,binding=0)		uniform sampler2D shadow_map;
layout(set=1,binding=1)		uniform sampler2D depth;
layout(set=1,binding=2, r8) uniform image2D occlusion_map;

vec3 get_view_position(vec2 uv) {
	// snap to center of pixel
	uv *= textureSize(depth, 0).xy;
	uv = floor(uv) + vec2(0.5);
	uv /= textureSize(depth, 0).xy;
	float depth_value = texture(depth, uv).r;
	vec4 clip_space = vec4(uv * 2.0 - 1.0, depth_value, 1.0);
	vec4 view_space = inverse(camera.projection_matrix) * clip_space;
	view_space /= view_space.w;
	return view_space.xyz;
}

layout(local_size_x=32, local_size_y=32) in;
void main() {
	const vec2 pixel_center = vec2(gl_GlobalInvocationID.xy) + vec2(0.5);
	const vec2 uv = pixel_center / vec2(gl_WorkGroupSize.xy * gl_NumWorkGroups.xy);
	vec2 d = uv * 2.0 - 1.0;

	vec3 position = get_view_position(uv);

	vec2 shadow_d = position.xz / vec2(4.0); // Assuming overhead light, map 8 x 8 area to shadow map
	vec2 shadow_uv = shadow_d * 0.5 + 0.5;
	ivec2 shadow_texel_coord = ivec2(shadow_uv * textureSize(shadow_map, 0));
	
	float shadow = texelFetch(shadow_map, shadow_texel_coord, 0).r;

	imageStore(occlusion_map, ivec2(gl_GlobalInvocationID.xy), vec4(shadow, shadow, shadow, 1.0));
}";