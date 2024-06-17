//! SSGI Render Pass
//! This module contains the implementation of the Screen Space Global Illumination (SSGI) render pass.

use core::{entity::EntityBuilder, Entity};

use ghi::{GraphicsHardwareInterface, CommandBufferRecording, BoundComputePipelineMode};

use json::object;
use resource_management::{asset::material_asset_handler::ProgramGenerator, shader_generation::{ShaderGenerationSettings, ShaderGenerator}};
use utils::Extent;

use super::common_shader_generator::CommonShaderGenerator;

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

pub const DEPTH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
pub const NORMALS_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const TRACE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

impl SSGIRenderPass {
	pub fn new(ghi: &mut ghi::GHI) -> EntityBuilder<'static, Self> {
		let normals = ghi.create_image(Some("Normals"), Extent::rectangle(1920, 1080), ghi::Formats::RGBA8(ghi::Encodings::SignedNormalized), ghi::Uses::Image, ghi::DeviceAccesses::GpuRead | ghi::DeviceAccesses::GpuWrite, ghi::UseCases::DYNAMIC);
		let trace = ghi.create_image(Some("Trace"), Extent::rectangle(1920, 1080), ghi::Formats::RGB16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Image, ghi::DeviceAccesses::GpuRead | ghi::DeviceAccesses::GpuWrite, ghi::UseCases::DYNAMIC);

		let pipeline_layout = ghi.create_pipeline_layout(&[], &[]);

		let stochastic_normals_shader = ghi.create_shader(Some("SSGI Stochastic Normals"), ghi::ShaderSource::GLSL(Self::make_stochastic_normals_shader()), ghi::ShaderTypes::Compute, &vec![DEPTH_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ), NORMALS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE)]).expect("Failed to create the stochastic normals shader.");
		let stochastic_normals = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&stochastic_normals_shader, ghi::ShaderTypes::Compute));

		let ray_march_shader = ghi.create_shader(Some("SSGI Ray March"), ghi::ShaderSource::GLSL(Self::make_ray_march_normals_shader()), ghi::ShaderTypes::Compute, &vec![NORMALS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ), TRACE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE)]).expect("Failed to create the ray march shader.");
		let ray_march = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&ray_march_shader, ghi::ShaderTypes::Compute));

		EntityBuilder::new(SSGIRenderPass {
			normals,
			trace,
			stochastic_normals,
			ray_march,
		})
	}

	fn make_stochastic_normals_shader() -> String {
		let shader_generator = {
			let common_shader_generator = CommonShaderGenerator::new_with_params(false, false, false, false, false, true, false, true);
			common_shader_generator
		};

		use besl::parser::Node;

		let _ = Node::binding("depth", Node::combined_image_sampler(), 1, 0, true, false);
		let _ = Node::binding("normals", Node::image("rgba8"), 1, 1, false, true);

		const CODE: &str = "uvec2 coord = uvec2(gl_GlobalInvocationID.xy);
		Camera camera = camera.camera;
		vec3 normal = make_normal_from_depth(depth, coord, camera.inverse_projection_matrix, camera.inverse_view_matrix);
		float noise = interleaved_gradient_noise(coord.x, coord.y, 0);
		vec3 stochastic_normal = get_cosine_hemisphere_sample(noise, noise, normal);
		imageStore(normals, ivec2(coord), vec4(stochastic_normal, 1.0));";

		let main = Node::function("main", Vec::new(), "void", vec![Node::glsl(CODE, &["make_normal_from_depth", "camera", "interleaved_gradient_noise", "get_cosine_hemisphere_sample"], vec![])]);

		let mut root = shader_generator.transform(Node::root(), &object!{});

		root.add(vec![main]);

		let root = besl::lex(root).unwrap();

		let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&ShaderGenerationSettings::compute(Extent::square(32)), &root);

		glsl
	}

	fn make_ray_march_normals_shader() -> String {
		let shader_generator = {
			let common_shader_generator = CommonShaderGenerator::new_with_params(false, false, false, false, false, true, false, true);
			common_shader_generator
		};

		use besl::parser::Node;

		let _ = Node::binding("depth", Node::combined_image_sampler(), 1, 0, true, false);
		let _ = Node::binding("trace", Node::image("rgb16"), 1, 2, false, true);
		let _ = Node::binding("diffuse", Node::combined_image_sampler(), 1, 3, true, false);

		const CODE: &str = "ivec2 coord = ivec2(gl_GlobalInvocationID.xy);
		vec2 uv = make_uv(coord, ivec2(1920, 1080));
		Camera camera = camera.camera;
		vec3 position = get_world_space_position_from_depth(uv, depth, camera.inverse_projection_matrix, camera.inverse_view_matrix);
		vec3 normal = texture(normals, uv).xyz;
		vec3 view_position = get_view_space_position_from_depth(uv, depth, camera.inverse_projection_matrix);
		vec2 jitter = vec2(0.0);
		vec3 direction = normalize(normal * camera.view_matrix);
		jitter += vec2(0.5);
		float step_count = 10f;
		float step_size = 1.0f / step_count;
		step_size = step_size * ((jitter.x + jitter.y) + 1.0f);
		vec4 ray_trace = ray_march(depth, camera.projection_matrix, direction, step_count, view_position, screen_position, uv, step_size, 1.0f);
		float ray_mask = ray_trace.w;
		vec2 hit_uv = ray_trace.xy;
		vec4 result = vec4(vec3(0.0), 1.0);
		result.xyz = texture(diffuse, hit_uv).xyz;
		imageStore(trace, coord, result);";

		let main = Node::function("main", Vec::new(), "void", vec![Node::glsl(CODE, &["make_uv", "camera", "get_world_space_position_from_depth", "get_view_space_position_from_depth", "ray_march"], vec![])]);

		let mut root = shader_generator.transform(Node::root(), &object!{});

		root.add(vec![main]);

		let root = besl::lex(root).unwrap();

		let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&ShaderGenerationSettings::compute(Extent::square(32)), &root);

		glsl
	}
}