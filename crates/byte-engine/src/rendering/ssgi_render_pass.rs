//! SSGI Render Pass
//! This module contains the implementation of the Screen Space Global Illumination (SSGI) render pass.

use core::{entity::EntityBuilder, Entity, EntityHandle};
use std::rc::Rc;

use ghi::{GraphicsHardwareInterface, CommandBufferRecording, BoundComputePipelineMode};

use resource_management::{asset::material_asset_handler::ProgramGenerator, image::Image, shader_generation::{ShaderGenerationSettings, ShaderGenerator}, ResourceManager};
use utils::{json, sync::RwLock, Extent};

use super::{common_shader_generator::CommonShaderGenerator, texture_manager::TextureManager};

/// The SSGI render pass.
pub struct SSGIRenderPass {
	/// The result of the ray marching.
	trace: ghi::ImageHandle,

	ray_march: ghi::PipelineHandle,
}

pub const DEPTH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
pub const DIFFUSE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
pub const TRACE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

impl SSGIRenderPass {
	pub async fn new<'c>(ghi_lock: Rc<RwLock<ghi::GHI>>, resource_manager: EntityHandle<ResourceManager>, texture_manager: &'c mut TextureManager, parent_descriptor_set_layout: ghi::DescriptorSetTemplateHandle, (depth_image, depth_sampler): (ghi::ImageHandle, ghi::SamplerHandle), diffuse_image: ghi::ImageHandle) -> Self {
		let resource_manager = resource_manager.read_sync();

		let mut blue_noise = resource_manager.request::<Image>("stbn_unitvec3_2Dx1D_128x128x64_0.png").await.unwrap();
		let (_, noise_texture, noise_sampler) = texture_manager.load(&mut blue_noise, ghi_lock.clone()).await.unwrap();

		let mut ghi = ghi_lock.write();

		let trace = ghi.create_image(Some("Trace"), Extent::rectangle(1920, 1080), ghi::Formats::RGB16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Image, ghi::DeviceAccesses::GpuRead | ghi::DeviceAccesses::GpuWrite, ghi::UseCases::DYNAMIC, 1);

		let descriptor_set_template = ghi.create_descriptor_set_template(Some("SSGI"), &[DEPTH_BINDING, DIFFUSE_BINDING, TRACE_BINDING]);

		let pipeline_layout = ghi.create_pipeline_layout(&[parent_descriptor_set_layout, descriptor_set_template], &[]);

		let descriptor_set = ghi.create_descriptor_set(Some("SSGi"), &descriptor_set_template);

		let sampler = ghi.create_sampler(ghi::FilteringModes::Closest, ghi::SamplingReductionModes::Max, ghi::FilteringModes::Closest, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32);

		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DEPTH_BINDING, depth_image, depth_sampler, ghi::Layouts::Read));
		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DIFFUSE_BINDING, diffuse_image, sampler, ghi::Layouts::Read).frame(-1));
		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&TRACE_BINDING, trace, ghi::Layouts::General));

		let ray_march_shader = ghi.create_shader(Some("SSGI Ray March"), ghi::ShaderSource::GLSL(Self::make_ray_march_normals_shader()), ghi::ShaderTypes::Compute, &vec![TRACE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE)]).expect("Failed to create the ray march shader.");
		let ray_march = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&ray_march_shader, ghi::ShaderTypes::Compute));

		SSGIRenderPass {
			trace,
			ray_march,
		}
	}

	fn make_ray_march_normals_shader() -> String {
		let shader_generator = {
			let common_shader_generator = CommonShaderGenerator::new_with_params(false, false, false, false, false, true, false, true);
			common_shader_generator
		};

		use besl::parser::Node;

		let ray_march = Node::function("ray_march", vec![besl::parser::Node::parameter("depth_buffer", "Texture2D"), besl::parser::Node::parameter("projection_matrix", "mat4f"), besl::parser::Node::parameter("direction", "vec3f"), besl::parser::Node::parameter("step_count", "u32"), besl::parser::Node::parameter("uv", "vec2f"), besl::parser::Node::parameter("step_size", "f32")], "vec4f", vec![Node::glsl("
			float start_depth = texture(depth_buffer, uv).r;

			for (uint i = 0; i < step_count; i++) {
				uv += step_size * (vec4(direction, 1.0f) * projection_matrix).xy;
				
				float depth = texture(depth_buffer, uv).r;

				/* Reversed depth */
				if (depth > start_depth) {
					return vec4(uv, 1.0, 1.0);
				}
			}

			return vec4(0.0);
		", &["make_normal_from_positions", "depth"], vec![])]);

		const CODE: &str = "ivec2 coord = ivec2(gl_GlobalInvocationID.xy);
		uvec2 extent = uvec2(1920, 1080);
		vec2 uv = make_uv(coord, extent);
		Camera camera = camera.camera;
		float noise = interleaved_gradient_noise(coord.x, coord.y, 0);
		vec3 normal = make_cosine_hemisphere_sample(noise, noise, make_normal_from_depth_map(depth, coord, extent, camera.inverse_projection_matrix, camera.inverse_view_matrix));
		vec2 jitter = vec2(0.0);
		vec3 direction = normalize(vec4(normal, 0.0) * camera.view).xyz;
		jitter += vec2(0.5);
		uint step_count = 10;
		float step_size = 1.0f / float(step_count);
		step_size = step_size * ((jitter.x + jitter.y) + 1.0f);
		vec4 ray_trace = ray_march(depth, camera.projection_matrix, direction, step_count, uv, step_size);
		float ray_mask = ray_trace.w;
		vec2 hit_uv = ray_trace.xy;
		vec4 result = vec4(texture(diffuse, hit_uv).xyz, 1.0);
		imageStore(trace, coord, result);";

		let main = Node::function("main", Vec::new(), "void", vec![Node::glsl(CODE, &["make_uv", "camera", "interleaved_gradient_noise", "make_cosine_hemisphere_sample", "make_normal_from_depth_map", "get_world_space_position_from_depth", "get_view_space_position_from_depth", "ray_march", "depth", "diffuse", "trace"], vec![])]);

		let mut root = shader_generator.transform(Node::root(), &json::object!{});

		root.add(vec![Node::binding("depth", Node::combined_image_sampler(), 1, DEPTH_BINDING.binding(), true, false), Node::binding("trace", Node::image("rgb16"), 1, TRACE_BINDING.binding(), false, true), Node::binding("diffuse", Node::combined_image_sampler(), 1, DIFFUSE_BINDING.binding(), true, false), ray_march, main]);

		let root = besl::lex(root).unwrap();

		let main = root.borrow().get_main().unwrap();

		let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&ShaderGenerationSettings::compute(Extent::square(32)), &main);

		glsl
	}

	fn render(&self, command_buffer: &mut impl ghi::CommandBufferRecording) {
		command_buffer.region("SSGI", |command_buffer| {
			command_buffer.region("Ray March", |command_buffer| {
				let command_buffer = command_buffer.bind_compute_pipeline(&self.ray_march);
				// command_buffer.bind_descriptor_sets(&self.pipeline_layout, &[]);
				command_buffer.dispatch(ghi::DispatchExtent::new(Extent::rectangle(1920, 1080), Extent::square(32)));
			})
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_make_ray_march_normals_shader() {
		let shader = SSGIRenderPass::make_ray_march_normals_shader();

		if let Err(e) = ghi::compile_glsl("SSGI Trace Shader", &shader) {
			panic!("Failed to compile the shader\n{}", e);
		}
	}
}