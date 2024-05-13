//! SSGI Render Pass
//! This module contains the implementation of the Screen Space Global Illumination (SSGI) render pass.

use core::{entity::EntityBuilder, Entity};

use ghi::{GraphicsHardwareInterface, CommandBufferRecording, BoundComputePipelineMode};

use utils::Extent;

use crate::shader_generator;
use super::shader_strings;

/// The SSGI render pass.
pub struct SSGIRenderPass {
	/// The generated stochastic normal for the ray marching.
	normals: ghi::ImageHandle,
	/// The result of the ray marching.
	trace: ghi::ImageHandle,

	stochastic_normals: ghi::PipelineHandle,
	ray_march: ghi::PipelineHandle,
}

impl Entity for SSGIRenderPass {}

impl SSGIRenderPass {
	pub fn new(ghi: &mut ghi::GHI) -> EntityBuilder<'static, Self> {
		let normals = ghi.create_image(Some("Normals"), Extent::rectangle(1920, 1080), ghi::Formats::RGBA8(ghi::Encodings::SignedNormalized), ghi::Uses::Image, ghi::DeviceAccesses::GpuRead | ghi::DeviceAccesses::GpuWrite, ghi::UseCases::DYNAMIC);
		let trace = ghi.create_image(Some("Trace"), Extent::rectangle(1920, 1080), ghi::Formats::RGB16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Image, ghi::DeviceAccesses::GpuRead | ghi::DeviceAccesses::GpuWrite, ghi::UseCases::DYNAMIC);

		let depth_binding = ghi::ShaderBindingDescriptor::new(1, 0, ghi::AccessPolicies::READ);
		let normals_binding = ghi::ShaderBindingDescriptor::new(2, 0, ghi::AccessPolicies::WRITE);
		let trace_binding = ghi::ShaderBindingDescriptor::new(3, 0, ghi::AccessPolicies::WRITE);

		let pipeline_layout = ghi.create_pipeline_layout(&[], &[]);

		let stochastic_normals_shader = ghi.create_shader(Some("SSGI Stochastic Normals"), ghi::ShaderSource::GLSL(Self::make_stochastic_normals_shader()), ghi::ShaderTypes::Compute, &vec![depth_binding.clone(), normals_binding.clone()]).expect("Failed to create the stochastic normals shader.");
		let stochastic_normals = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&stochastic_normals_shader, ghi::ShaderTypes::Compute));

		let ray_march_shader = ghi.create_shader(Some("SSGI Ray March"), ghi::ShaderSource::GLSL(Self::make_ray_march_normals_shader()), ghi::ShaderTypes::Compute, &vec![normals_binding.clone(), trace_binding.clone()]).expect("Failed to create the ray march shader.");
		let ray_march = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&ray_march_shader, ghi::ShaderTypes::Compute));

		EntityBuilder::new(SSGIRenderPass {
			normals,
			trace,
			stochastic_normals,
			ray_march,
		})
	}

	fn make_stochastic_normals_shader() -> String {
		let mut string = shader_generator::generate_glsl_header_block(&shader_generator::ShaderGenerationSettings::new("Compute"));

		string.push_str(shader_strings::LENGTH_SQUARED);
		string.push_str(shader_strings::MIN_DIFF);
		string.push_str(shader_strings::ANIMATED_INTERLEAVED_GRADIENT_NOISE);
		string.push_str(shader_strings::GET_WORLD_SPACE_POSITION_FROM_DEPTH);
		string.push_str(shader_strings::MAKE_NORMAL_FROM_NEIGHBOURING_DEPTH_SAMPLES);
		string.push_str(shader_strings::MAKE_NORMAL_FROM_DEPTH);
		string.push_str(shader_strings::GET_COSINE_HEMISPHERE_SAMPLE);

		string.push_str(&shader_generator::generate_uniform_block(1, 0, ghi::AccessPolicies::READ, "sampler2D", "depth"));
		string.push_str(&shader_generator::generate_uniform_block(1, 1, ghi::AccessPolicies::WRITE, "image2D", "normals"));

		string.push_str("void main() {\n");
		string.push_str("ivec2 coord = ivec2(gl_GlobalInvocationID.xy);\n");
		string.push_str("Camera camera = get_camera();\n");
		string.push_str("vec3 normal = make_normal_from_depth(depth, uvec2(coord), camera.inverse_projection_matrix, camera.inverse_view_matrix);\n");
		string.push_str("float noise = interleaved_gradient_noise(uint32_t(coord.x), uint32_t(coord.y), 0);\n");
		string.push_str("vec3 stochastic_normal = normalize(get_cosine_hemisphere_sample(noise, noise, normal));\n");
		string.push_str("imageStore(normals, coord, vec4(stochastic_normal, 1.0));\n");
		string.push_str("}\n");

		string
	}

	fn make_ray_march_normals_shader() -> String {
		let mut string = shader_generator::generate_glsl_header_block(&shader_generator::ShaderGenerationSettings::new("Compute"));

		string.push_str(shader_strings::LENGTH_SQUARED);
		string.push_str(shader_strings::MAKE_UV);
		string.push_str(shader_strings::GET_WORLD_SPACE_POSITION_FROM_DEPTH);
		string.push_str(shader_strings::GET_VIEW_SPACE_POSITION_FROM_DEPTH);

		string.push_str(&shader_generator::generate_uniform_block(1, 0, ghi::AccessPolicies::READ, "sampler2D", "depth"));
		string.push_str(&shader_generator::generate_uniform_block(1, 2, ghi::AccessPolicies::WRITE, "image2D", "trace"));
		string.push_str(&shader_generator::generate_uniform_block(1, 3, ghi::AccessPolicies::READ, "sampler2D", "diffuse"));

		string.push_str("void main() {\n");
		string.push_str("ivec2 coord = ivec2(gl_GlobalInvocationID.xy);\n");
		string.push_str("vec2 uv = make_uv(coord, ivec2(1920, 1080));\n");

		string.push_str("Camera camera = get_camera();\n");

		string.push_str("vec3 position = get_world_space_position_from_depth(uv, depth, camera.inverse_projection_matrix, camera.inverse_view_matrix);\n");
		string.push_str("vec3 normal = texture(normals, uv).xyz;\n");

		string.push_str("vec3 view_position = get_view_space_position_from_depth(uv, depth, camera.inverse_projection_matrix);\n");

		string.push_str("vec2 jitter = vec2(0.0);\n");

		string.push_str("vec3 direction = normalize(normal * camera.view_matrix);\n");

		string.push_str("jitter += vec2(0.5);\n");

		string.push_str("float step_count = 10f;\n");
		string.push_str("float step_size = 1.0f / step_count;\n");
		string.push_str("step_size = step_size * ((jitter.x + jitter.y) + 1.0f);\n");

		string.push_str("vec4 ray_trace = ray_march(depth, camera.projection_matrix, direction, step_count, view_position, screen_position, uv, step_size, 1.0f);\n");

		string.push_str("float ray_mask = ray_trace.w;\n");
		string.push_str("vec2 hit_uv = ray_trace.xy;\n");

		string.push_str("vec4 result = vec4(vec3(0.0), 1.0);\n");
		string.push_str("result.xyz = texture(diffuse, hit_uv).xyz;\n");

		string.push_str("imageStore(trace, coord, result);\n");

		string
	}
}