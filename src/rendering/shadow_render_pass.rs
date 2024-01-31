use maths_rs::mat::{MatProjection, MatTranslate, MatRotate3D};

use crate::{Extent, ghi};

use super::world_render_domain::WorldRenderDomain;

pub struct ShadowRenderingPass {
	pipeline: ghi::PipelineHandle,
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	shadow_map: ghi::ImageHandle,

	occlusion_map_build_pipeline: ghi::PipelineHandle,
}

impl ShadowRenderingPass {
	pub fn new(ghi: &mut dyn ghi::GraphicsHardwareInterface, render_domain: &impl WorldRenderDomain) -> ShadowRenderingPass {
		let light_matrics_binding_template = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH | ghi::Stages::COMPUTE);
		let light_depth_map = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
		let view_depth_map = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
		let occlusion_image = ghi::DescriptorSetBindingTemplate::new(3, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

		let bindings = [
			light_matrics_binding_template.clone(),
			light_depth_map.clone(),
			view_depth_map.clone(),
			occlusion_image.clone(),
		];

		let descriptor_set_template = ghi.create_descriptor_set_template(Some("Shadow Rendering Set Layout"), &bindings);

		let pipeline_layout = ghi.create_pipeline_layout(&[render_domain.get_descriptor_set_template(), descriptor_set_template], &[]);

		let descriptor_set = ghi.create_descriptor_set(Some("Shadow Rendering Descriptor Set"), &descriptor_set_template);

		let light_matrices_binding = ghi.create_descriptor_binding(descriptor_set, &light_matrics_binding_template);

		let colored_shadow: bool = false;

		let shadow_map_resolution = Extent::square(4096);

		let shadow_map = ghi.create_image(Some("Shadow Map"), shadow_map_resolution, ghi::Formats::Depth32, None, ghi::Uses::Image, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

		let light_matrices_buffer = ghi.create_buffer(Some("Light Matrices Buffer"), 256 * 4 * 4 * 4, ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

		let sampler = ghi.create_sampler(ghi::FilteringModes::Linear, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32);

		let shadow_map_binding = ghi.create_descriptor_binding(descriptor_set, &light_depth_map);
		let view_depth_map_binding = ghi.create_descriptor_binding(descriptor_set, &view_depth_map);
		let occlusion_image_binding = ghi.create_descriptor_binding(descriptor_set, &occlusion_image);

		ghi.write(&[
			ghi::DescriptorWrite::buffer(light_matrices_binding, light_matrices_buffer,),
			ghi::DescriptorWrite::combined_image_sampler(shadow_map_binding, shadow_map, sampler, ghi::Layouts::Read),
			ghi::DescriptorWrite::combined_image_sampler(view_depth_map_binding, render_domain.get_view_depth_image(), sampler, ghi::Layouts::Read),
			ghi::DescriptorWrite::image(occlusion_image_binding, render_domain.get_view_occlusion_image(), ghi::Layouts::General),
		]);

		let x = 4f32;

		let mut light_projection_matrix = maths_rs::Mat4f::create_ortho_matrix(-x, x, -x, x, 0.1f32, 100f32);

		light_projection_matrix[5] *= -1.0f32;

		let light_view_matrix = maths_rs::Mat4f::from_x_rotation(-std::f32::consts::FRAC_PI_2); // Looking down from +y axis

		let matric_buffer = ghi.get_mut_buffer_slice(light_matrices_buffer);

		matric_buffer[..64].copy_from_slice((light_projection_matrix * light_view_matrix).as_u8_slice());

		let mesh_shader = ghi.create_shader(ghi::ShaderSource::GLSL(VISIBILITY_PASS_MESH_SOURCE.to_string()), ghi::ShaderTypes::Mesh, &[
			ghi::ShaderBindingDescriptor::new(0, 0, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(0, 1, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(0, 2, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(0, 3, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(0, 4, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(0, 5, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(0, 6, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(1, 0, ghi::AccessPolicies::READ),
		]);

		let pipeline = ghi.create_raster_pipeline(&[
			ghi::PipelineConfigurationBlocks::Layout { layout: &pipeline_layout },
			ghi::PipelineConfigurationBlocks::Shaders { shaders: &[(&mesh_shader, ghi::ShaderTypes::Mesh, &[])], },
			ghi::PipelineConfigurationBlocks::RenderTargets { targets: &[ghi::AttachmentInformation::new(shadow_map, ghi::Formats::Depth32, ghi::Layouts::RenderTarget, ghi::ClearValue::Depth(0.0f32), false, true)] },
		]);

		let occlusion_map_shader = ghi.create_shader(ghi::ShaderSource::GLSL(SHADOW_TO_OCLUSSION_MAP_SOURCE.to_string()), ghi::ShaderTypes::Compute, &[
			ghi::ShaderBindingDescriptor::new(0, 0, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(1, 0, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(1, 1, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(1, 2, ghi::AccessPolicies::READ),
			ghi::ShaderBindingDescriptor::new(1, 3, ghi::AccessPolicies::WRITE),
		]);

		let occlusion_map_build_pipeline = ghi.create_compute_pipeline(&pipeline_layout, (&occlusion_map_shader, ghi::ShaderTypes::Compute, &[]));

		ShadowRenderingPass { pipeline, pipeline_layout, descriptor_set, shadow_map, occlusion_map_build_pipeline }
	}

	pub fn render(&self, command_buffer_recording: &mut dyn ghi::CommandBufferRecording, render_domain: &impl WorldRenderDomain) {
		command_buffer_recording.start_region("Shadow Rendering");

		let render_pass = command_buffer_recording.start_render_pass(Extent::square(4096), &[ghi::AttachmentInformation::new(self.shadow_map, ghi::Formats::Depth32, ghi::Layouts::RenderTarget, ghi::ClearValue::Depth(0.0f32), false, true)]);
		render_pass.bind_descriptor_sets(&self.pipeline_layout, &[render_domain.get_descriptor_set(), self.descriptor_set]);
		let pipeline = render_pass.bind_raster_pipeline(&self.pipeline);
		pipeline.dispatch_meshes(192, 1, 1);
		render_pass.end_render_pass();

		command_buffer_recording.end_region();
	}

	pub fn get_shadow_map_image(&self) -> ghi::ImageHandle { self.shadow_map }
}

const VISIBILITY_PASS_MESH_SOURCE: &'static str = "
#version 450
#pragma shader_stage(mesh)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_shader_explicit_arithmetic_types: enable
#extension GL_EXT_mesh_shader: require
#extension GL_EXT_debug_printf : enable

layout(row_major) uniform; layout(row_major) buffer;

layout(location=0) perprimitiveEXT out uint out_instance_index[126];
layout(location=1) perprimitiveEXT out uint out_primitive_index[126];

struct Camera {
	mat4 view_matrix;
	mat4 projection_matrix;
	mat4 view_projection;
};

struct Mesh {
	mat4 model;
	uint material_id;
	uint32_t base_vertex_index;
};

struct Meshlet {
	uint32_t instance_index;
	uint16_t vertex_offset;
	uint16_t triangle_offset;
	uint8_t vertex_count;
	uint8_t triangle_count;
};

layout(set=0,binding=0,scalar) buffer readonly CameraBuffer {
	Camera camera;
};

layout(set=0,binding=1,scalar) buffer readonly MeshesBuffer {
	Mesh meshes[];
};

layout(set=0,binding=2,scalar) buffer readonly MeshVertexPositions {
	vec3 vertex_positions[];
};

layout(set=0,binding=4,scalar) buffer readonly VertexIndices {
	uint16_t vertex_indices[];
};

layout(set=0,binding=5,scalar) buffer readonly PrimitiveIndices {
	uint8_t primitive_indices[];
};

layout(set=0,binding=6,scalar) buffer readonly MeshletsBuffer {
	Meshlet meshlets[];
};

layout(set=1,binding=0,scalar) buffer readonly LightMatrices {
	mat4 light_matrix;
};

layout(triangles, max_vertices=64, max_primitives=126) out;
layout(local_size_x=128) in;
void main() {
	uint meshlet_index = gl_WorkGroupID.x;

	Meshlet meshlet = meshlets[meshlet_index];
	Mesh mesh = meshes[meshlet.instance_index];

	uint instance_index = meshlet.instance_index;

	SetMeshOutputsEXT(meshlet.vertex_count, meshlet.triangle_count);

	if (gl_LocalInvocationID.x < uint(meshlet.vertex_count)) {
		uint vertex_index = mesh.base_vertex_index + uint32_t(vertex_indices[uint(meshlet.vertex_offset) + gl_LocalInvocationID.x]);
		gl_MeshVerticesEXT[gl_LocalInvocationID.x].gl_Position = light_matrix * meshes[instance_index].model * vec4(vertex_positions[vertex_index], 1.0);
	}
	
	if (gl_LocalInvocationID.x < uint(meshlet.triangle_count)) {
		uint triangle_index = uint(meshlet.triangle_offset) + gl_LocalInvocationID.x;
		uint triangle_indices[3] = uint[](primitive_indices[triangle_index * 3 + 0], primitive_indices[triangle_index * 3 + 1], primitive_indices[triangle_index * 3 + 2]);
		gl_PrimitiveTriangleIndicesEXT[gl_LocalInvocationID.x] = uvec3(triangle_indices[0], triangle_indices[1], triangle_indices[2]);
		out_instance_index[gl_LocalInvocationID.x] = instance_index;
		out_primitive_index[gl_LocalInvocationID.x] = (meshlet_index << 8) | (gl_LocalInvocationID.x & 0xFF);
	}
}";


const SHADOW_TO_OCLUSSION_MAP_SOURCE: &'static str = r#"
#version 450
#pragma shader_stage(compute)

layout(row_major) uniform; layout(row_major) buffer;

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_explicit_arithmetic_types : enable

struct Mesh {
	mat4 model;
	uint material_index;
	uint32_t base_vertex_index;
};

struct Camera {
	mat4 view_matrix;
	mat4 projection_matrix;
	mat4 view_projection;
};
layout(set=0,binding=0,scalar) buffer readonly CameraBuffer {
	Camera camera;
};

layout(set=1,binding=0,scalar) buffer readonly LightMatrices {
	mat4 light_matrix;
};
layout(set=1, binding=1) uniform sampler2D depth_shadow_map;
layout(set=1, binding=2) uniform sampler2D depth_map;
layout(set=1, binding=3, r8) uniform image2D oclussion_map;

vec3 get_view_position(sampler2D depth_map, uvec2 coords, mat4 projection_matrix) {
	float depth_value = texelFetch(depth_map, ivec2(coords), 0).r;
	vec2 uv = (vec2(coords) + vec2(0.5)) / vec2(textureSize(depth_map, 0).xy);
	vec4 clip_space = vec4(uv * 2.0 - 1.0, depth_value, 1.0);
	vec4 view_space = inverse(projection_matrix) * clip_space;
	view_space /= view_space.w;
	vec4 world_space = inverse(camera.view_matrix) * view_space;
	return world_space.xyz;
}

layout(local_size_x=32, local_size_y=32) in;
void main() {
	if (gl_GlobalInvocationID.x >= imageSize(oclussion_map).x || gl_GlobalInvocationID.y >= imageSize(oclussion_map).y) { return; }

	vec3 surface_world_position = get_view_position(depth_map, uvec2(gl_GlobalInvocationID.xy), camera.projection_matrix);

	vec4 surface_light_clip_position = light_matrix * vec4(surface_world_position, 1.0);

	vec3 surface_light_ndc_position = surface_light_clip_position.xyz / surface_light_clip_position.w;

	vec2 shadow_uv = surface_light_ndc_position.xy * 0.5 + 0.5;

	float z = surface_light_ndc_position.z;

	float shadow_sample_depth = texture(depth_shadow_map, shadow_uv).r;

	float occlusion_factor = z + 0.00001 < shadow_sample_depth ? 0.0 : 1.0;

	imageStore(oclussion_map, ivec2(gl_GlobalInvocationID.xy), vec4(occlusion_factor, 0, 0, 0));
}
"#;

// const SHADOW_RAY_GEN_SHADER: &'static str = "
// #version 460 core
// #pragma shader_stage(raygen)

// #extension GL_EXT_scalar_block_layout: enable
// #extension GL_EXT_buffer_reference: enable
// #extension GL_EXT_buffer_reference2: enable
// #extension GL_EXT_shader_16bit_storage: require
// #extension GL_EXT_ray_tracing: require

// layout(row_major) uniform; layout(row_major) buffer;

// struct Camera {
// 	mat4 view_matrix;
// 	mat4 projection_matrix;
// 	mat4 view_projection;
// };

// layout(set=0,binding=0,scalar) buffer readonly CameraBuffer {
// 	Camera camera;
// };

// layout(set=1,binding=0) 				uniform accelerationStructureEXT top_level_acceleration_structure;
// layout(set=1,binding=1, r32ui) coherent uniform uimage2D shadow_map;
// layout(set=1,binding=2) 				uniform sampler2D depth;

// layout(location = 0) rayPayloadEXT float hit_distance;

// mat3 matrix_from_direction_vector(vec3 d) {
// 	// TODO: check for colinearity
// 	vec3 u = cross(vec3(0.0, 1.0, 0.0), d);
// 	vec3 v = cross(d, u);
// 	return mat3(u, v, d);
// }

// vec3 get_view_position(uvec2 coords) {
// 	float depth_value = texelFetch(depth, ivec2(coords), 0).r;
// 	vec2 uv = (vec2(coords) + vec2(0.5)) / vec2(textureSize(depth, 0).xy);
// 	vec4 clip_space = vec4(uv * 2.0 - 1.0, depth_value, 1.0);
// 	vec4 view_space = inverse(camera.projection_matrix) * clip_space;
// 	view_space /= view_space.w;
// 	return view_space.xyz;
// }

// vec3 get_view_position(vec2 uv) {
// 	// snap to center of pixel
// 	uv *= textureSize(depth, 0).xy;
// 	uv = floor(uv) + vec2(0.5);
// 	uv /= textureSize(depth, 0).xy;
// 	float depth_value = texture(depth, uv).r;
// 	vec4 clip_space = vec4(uv * 2.0 - 1.0, depth_value, 1.0);
// 	vec4 view_space = inverse(camera.projection_matrix) * clip_space;
// 	view_space /= view_space.w;
// 	return view_space.xyz;
// }

// float length_squared(float v) { return v * v; }
// float length_squared(vec2 v) { return dot(v, v); }
// float length_squared(vec3 v) { return dot(v, v); }

// vec3 min_diff(vec3 p, vec3 a, vec3 b) {
//     vec3 ap = a - p;
//     vec3 bp = p - b;
//     return (length_squared(ap) < length_squared(bp)) ? ap : bp;
// }

// void main() {
// 	const vec2 pixel_center = vec2(gl_LaunchIDEXT.xy) + vec2(0.5);
// 	const vec2 uv = pixel_center / vec2(gl_LaunchSizeEXT.xy);
// 	vec2 d = uv * 2.0 - 1.0;

// 	uvec2 coords = uvec2(gl_LaunchIDEXT.xy);

// 	vec3 p = get_view_position(coords + uvec2(0, 0));
// 	vec3 pt = get_view_position(coords + uvec2(0, 1));
// 	vec3 pl = get_view_position(coords + uvec2(-1, 0));
// 	vec3 pr = get_view_position(coords + uvec2(1, 0));
// 	vec3 pb = get_view_position(coords + uvec2(0, -1));

// 	vec3 n = normalize(cross(min_diff(p, pr, pl), min_diff(p, pt, pb)));

// 	vec3 direction = vec3(0, 1, 0); // Overhead light
// 	vec3 position = get_view_position(uv);

// 	vec2 shadow_d = position.xz / vec2(4.0); // Assuming overhead light, map 8 x 8 area to shadow map
// 	vec2 shadow_uv = shadow_d * 0.5 + 0.5;
// 	ivec2 shadow_texel_coord = ivec2(shadow_uv * imageSize(shadow_map));

// 	if (dot(n, direction) <= 0.0) {
// 		imageAtomicMin(shadow_map, shadow_texel_coord, 0);
// 		return;
// 	}

// 	vec3 origin = position + n * 0.001; // Offset origin slightly to avoid self-intersection

// 	const float ray_distance = 10.0; // Maximum distance to check for intersection

// 	uint ray_flags = 0;
// 	uint cull_mask = 0xff;
// 	float t_min = 0.0f;
// 	float t_max = ray_distance;

// 	traceRayEXT(top_level_acceleration_structure, ray_flags, cull_mask, 0, 0, 0, origin, t_min, direction, t_max, 0);

// 	imageAtomicMin(shadow_map, shadow_texel_coord, (1 << 32) - 1);
// }";

// const SHADOW_HIT_SHADER: &'static str = "
// #version 460 core
// #pragma shader_stage(closest)

// #extension GL_EXT_scalar_block_layout: enable
// #extension GL_EXT_buffer_reference: enable
// #extension GL_EXT_buffer_reference2: enable
// #extension GL_EXT_shader_16bit_storage: require
// #extension GL_EXT_ray_tracing: require

// layout(row_major) uniform; layout(row_major) buffer;

// layout(location = 0) rayPayloadInEXT float hit_distance;

// void main() {
// 	hit_distance = gl_HitTEXT / gl_RayTmaxEXT;
// }";

// const SHADOW_MISS_SHADER: &'static str = "
// #version 460 core
// #pragma shader_stage(miss)

// #extension GL_EXT_scalar_block_layout: enable
// #extension GL_EXT_buffer_reference: enable
// #extension GL_EXT_buffer_reference2: enable
// #extension GL_EXT_shader_16bit_storage: require
// #extension GL_EXT_ray_tracing: require

// layout(row_major) uniform; layout(row_major) buffer;

// layout(location = 0) rayPayloadInEXT float hit_distance;

// void main() {
// 	hit_distance = 1.0f;
// }";

// struct ShadowMappingPass {
// 	pipeline_layout: ghi::PipelineLayoutHandle,
// 	pipeline: ghi::PipelineHandle,
// 	descriptor_set_template: ghi::DescriptorSetTemplateHandle,
// 	descriptor_set: ghi::DescriptorSetHandle,

// 	depth_target: ghi::ImageHandle,
// 	shadow_map: ghi::ImageHandle,
// 	occlusion_map: ghi::ImageHandle,
// }

// impl ShadowMappingPass {
// 	fn new(ghi: &mut dyn ghi::GraphicsHardwareInterface, shadow_rendering_pass: &ShadowRenderingPass, occlusion_map: ghi::ImageHandle, parent_descriptor_set_template: ghi::DescriptorSetTemplateHandle, depth_target: ghi::ImageHandle) -> ShadowMappingPass {
// 		let shadow_map_binding_template = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
// 		let depth_binding_template = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
// 		let result_binding_template = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

// 		let descriptor_set_template = ghi.create_descriptor_set_template(Some("Shadow Mapping Pass Set Layout"), &[shadow_map_binding_template.clone(), depth_binding_template.clone(), result_binding_template.clone()]);
// 		let pipeline_layout = ghi.create_pipeline_layout(&[parent_descriptor_set_template, descriptor_set_template], &[]);
// 		let descriptor_set = ghi.create_descriptor_set(Some("Shadow Mapping Pass Descriptor Set"), &descriptor_set_template);

// 		let shader = ghi.create_shader(ghi::ShaderSource::GLSL(BUILD_SHADOW_MAP_SHADER), ghi::ShaderTypes::Compute,);
// 		let pipeline = ghi.create_compute_pipeline(&pipeline_layout, (&shader, ghi::ShaderTypes::Compute, vec![]));

// 		let shadow_map_binding = ghi.create_descriptor_binding(descriptor_set, &shadow_map_binding_template);
// 		let depth_binding = ghi.create_descriptor_binding(descriptor_set, &depth_binding_template);
// 		let result_binding = ghi.create_descriptor_binding(descriptor_set, &result_binding_template);

// 		let sampler = ghi.create_sampler(ghi::FilteringModes::Linear, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32);

// 		ghi.write(&[
// 			ghi::DescriptorWrite {
// 				binding_handle: shadow_map_binding,
// 				array_element: 0,
// 				descriptor: ghi::Descriptor::CombinedImageSampler { image_handle: shadow_rendering_pass.shadow_map, sampler_handle: sampler, layout: ghi::Layouts::Read },
// 			},
// 			ghi::DescriptorWrite {
// 				binding_handle: depth_binding,
// 				array_element: 0,
// 				descriptor: ghi::Descriptor::CombinedImageSampler { image_handle: depth_target, sampler_handle: sampler, layout: ghi::Layouts::Read },
// 			},
// 			ghi::DescriptorWrite {
// 				binding_handle: result_binding,
// 				array_element: 0,
// 				descriptor: ghi::Descriptor::Image{ handle: occlusion_map, layout: ghi::Layouts::General },
// 			},
// 		]);

// 		ShadowMappingPass {
// 			pipeline_layout,
// 			pipeline,
// 			descriptor_set_template,
// 			descriptor_set,

// 			depth_target,
// 			shadow_map: shadow_rendering_pass.shadow_map,
// 			occlusion_map,
// 		}
// 	}

// 	fn render(&self, command_buffer_recording: &mut dyn ghi::CommandBufferRecording) {
// 		command_buffer_recording.consume_resources(&[
// 			ghi::Consumption{
// 				handle: ghi::Handle::Image(self.shadow_map),
// 				stages: ghi::Stages::COMPUTE,
// 				access: ghi::AccessPolicies::READ,
// 				layout: ghi::Layouts::Read,
// 			},
// 			ghi::Consumption{
// 				handle: ghi::Handle::Image(self.depth_target),
// 				stages: ghi::Stages::COMPUTE,
// 				access: ghi::AccessPolicies::READ,
// 				layout: ghi::Layouts::Read,
// 			},
// 			ghi::Consumption{
// 				handle: ghi::Handle::Image(self.occlusion_map),
// 				stages: ghi::Stages::COMPUTE,
// 				access: ghi::AccessPolicies::WRITE,
// 				layout: ghi::Layouts::General,
// 			},
// 		]);

// 		command_buffer_recording.bind_compute_pipeline(&self.pipeline);
// 		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
// 		command_buffer_recording.dispatch(ghi::DispatchExtent::new(Extent::plane(1920, 1080), Extent::square(32)));
// 	}
// }

// const BUILD_SHADOW_MAP_SHADER: &'static str = "
// #version 460 core
// #pragma shader_stage(compute)

// #extension GL_EXT_scalar_block_layout: enable
// #extension GL_EXT_buffer_reference: enable
// #extension GL_EXT_buffer_reference2: enable
// #extension GL_EXT_shader_16bit_storage: require
// #extension GL_EXT_ray_tracing: require

// layout(row_major) uniform; layout(row_major) buffer;

// struct Camera {
// 	mat4 view_matrix;
// 	mat4 projection_matrix;
// 	mat4 view_projection;
// };

// layout(set=0,binding=0,scalar) buffer readonly CameraBuffer {
// 	Camera camera;
// };

// layout(set=1,binding=0)		uniform sampler2D shadow_map;
// layout(set=1,binding=1)		uniform sampler2D depth;
// layout(set=1,binding=2, r8) uniform image2D occlusion_map;

// vec3 get_view_position(vec2 uv) {
// 	// snap to center of pixel
// 	uv *= textureSize(depth, 0).xy;
// 	uv = floor(uv) + vec2(0.5);
// 	uv /= textureSize(depth, 0).xy;
// 	float depth_value = texture(depth, uv).r;
// 	vec4 clip_space = vec4(uv * 2.0 - 1.0, depth_value, 1.0);
// 	vec4 view_space = inverse(camera.projection_matrix) * clip_space;
// 	view_space /= view_space.w;
// 	return view_space.xyz;
// }

// layout(local_size_x=32, local_size_y=32) in;
// void main() {
// 	const vec2 pixel_center = vec2(gl_GlobalInvocationID.xy) + vec2(0.5);
// 	const vec2 uv = pixel_center / vec2(gl_WorkGroupSize.xy * gl_NumWorkGroups.xy);
// 	vec2 d = uv * 2.0 - 1.0;

// 	vec3 position = get_view_position(uv);

// 	vec2 shadow_d = position.xz / vec2(4.0); // Assuming overhead light, map 8 x 8 area to shadow map
// 	vec2 shadow_uv = shadow_d * 0.5 + 0.5;
// 	ivec2 shadow_texel_coord = ivec2(shadow_uv * textureSize(shadow_map, 0));
	
// 	float shadow = texelFetch(shadow_map, shadow_texel_coord, 0).r;

// 	imageStore(occlusion_map, ivec2(gl_GlobalInvocationID.xy), vec4(shadow, shadow, shadow, 1.0));
// }";