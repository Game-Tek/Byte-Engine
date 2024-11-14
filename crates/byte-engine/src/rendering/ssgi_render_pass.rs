//! SSGI Render Pass
//! This module contains the implementation of the Screen Space Global Illumination (SSGI) render pass.

use core::{entity::EntityBuilder, Entity, EntityHandle};
use std::{rc::Rc, sync::Arc};

use ghi::{GraphicsHardwareInterface, CommandBufferRecordable, BoundComputePipelineMode};

use resource_management::{asset::material_asset_handler::ProgramGenerator, image::Image, shader_generation::{ShaderGenerationSettings, ShaderGenerator}, ResourceManager};
use utils::{json, sync::RwLock, Extent};

use super::{common_shader_generator::CommonShaderGenerator, render_pass::RenderPass, texture_manager::TextureManager};

/// The SSGI render pass.
pub struct SSGIRenderPass {
	/// The result of the ray marching.
	trace: ghi::ImageHandle,

	ray_march: ghi::PipelineHandle,
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
}

pub const DEPTH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
pub const DIFFUSE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
pub const TRACE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

impl SSGIRenderPass {
	pub async fn new<'c>(ghi_lock: Rc<RwLock<ghi::GHI>>, resource_manager: EntityHandle<ResourceManager>, texture_manager: Arc<utils::r#async::RwLock<TextureManager>>, parent_descriptor_set_layout: ghi::DescriptorSetTemplateHandle, (depth_image, depth_sampler): (ghi::ImageHandle, ghi::SamplerHandle), diffuse_image: ghi::ImageHandle) -> Self {
		let resource_manager = resource_manager.read_sync();

		let mut blue_noise = resource_manager.request::<Image>("stbn_unitvec3_2Dx1D_128x128x64_0.png").await.unwrap();
		let (_, noise_texture, noise_sampler) = texture_manager.write().await.load(&mut blue_noise, ghi_lock.clone()).await.unwrap();

		let mut ghi = ghi_lock.write();

		let trace = ghi.create_image(Some("Trace"), Extent::square(0), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage, ghi::DeviceAccesses::GpuRead | ghi::DeviceAccesses::GpuWrite, ghi::UseCases::DYNAMIC, 1);

		let descriptor_set_template = ghi.create_descriptor_set_template(Some("SSGI"), &[DEPTH_BINDING, DIFFUSE_BINDING, TRACE_BINDING]);

		let pipeline_layout = ghi.create_pipeline_layout(&[parent_descriptor_set_layout, descriptor_set_template], &[]);

		let descriptor_set = ghi.create_descriptor_set(Some("SSGi"), &descriptor_set_template);

		let sampler = ghi.create_sampler(ghi::FilteringModes::Linear, ghi::SamplingReductionModes::WeightedAverage, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32);

		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DEPTH_BINDING, depth_image, depth_sampler, ghi::Layouts::Read));
		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DIFFUSE_BINDING, diffuse_image, sampler, ghi::Layouts::Read));
		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&TRACE_BINDING, trace, ghi::Layouts::General));

		let ray_march_shader = ghi.create_shader(Some("SSGI Ray March"), ghi::ShaderSource::GLSL(Self::make_ray_march_normals_shader()), ghi::ShaderTypes::Compute, &vec![DEPTH_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ), DIFFUSE_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ), TRACE_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE)]).expect("Failed to create the ray march shader.");
		let ray_march = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&ray_march_shader, ghi::ShaderTypes::Compute));

		SSGIRenderPass {
			trace,
			pipeline_layout,
			descriptor_set,
			ray_march,
		}
	}

	fn make_ray_march_normals_shader() -> String {
		let shader_generator = {
			let common_shader_generator = CommonShaderGenerator::new_with_params(false, false, false, false, false, true, false, true);
			common_shader_generator
		};

		use besl::parser::Node;

		let ray_march = Node::function("ray_march", vec![besl::parser::Node::parameter("depth_buffer", "Texture2D"), besl::parser::Node::parameter("direction", "vec2f"), besl::parser::Node::parameter("step_count", "u32"), besl::parser::Node::parameter("uv", "vec2f"), besl::parser::Node::parameter("step_size", "f32")], "vec4f", vec![Node::glsl("
			float start_depth = texture(depth_buffer, uv).r;

			for (uint i = 0; i < step_count; i++) {
				uv += step_size * direction;
				
				float depth = texture(depth_buffer, uv).r;

				/* Reversed depth */
				if (depth > start_depth) {
					return vec4(uv, 1.0, 1.0);
				}
			}

			return vec4(0.0);
		", &[], vec![])]);

		const CODE: &str = "ivec2 coord = ivec2(gl_GlobalInvocationID.xy);
		uvec2 extent = uvec2(imageSize(trace));
		vec2 uv = make_uv(coord, extent);
		View view = views.views[0];
		float noise = interleaved_gradient_noise(coord.x, coord.y, 0);
		vec3 normal = make_normal_from_depth_map(depth, coord, extent, view.inverse_projection, view.inverse_view);

		if (normal.x == 0.0 && normal.y == 0.0 && normal.z == 0.0) {
			imageStore(trace, coord, vec4(0.0));
			return;
		}

		vec3 hem_dir = make_cosine_hemisphere_sample(noise, noise, normal);
		vec2 jitter = vec2(coord.y, coord.x);
		vec4 direction = normalize(vec4(hem_dir, 0.0) * view.view);
		jitter += vec2(0.5);
		uint step_count = 10;
		float step_size = 1.0f / float(step_count);
		vec4 ray_trace = ray_march(depth, (view.projection * direction).xy, step_count, uv, step_size);
		float ray_mask = ray_trace.w;
		vec2 hit_uv = ray_trace.xy;
		vec4 result = vec4(texture(diffuse, hit_uv).xyz, 1.0);
		imageStore(trace, coord, vec4(result * ray_trace.w));
		";

		let main = Node::function("main", Vec::new(), "void", vec![Node::glsl(CODE, &["make_uv", "views", "interleaved_gradient_noise", "make_cosine_hemisphere_sample", "make_normal_from_depth_map", "get_world_space_position_from_depth", "get_view_space_position_from_depth", "ray_march", "depth", "diffuse", "trace"], vec![])]);

		let mut root = shader_generator.transform(Node::root(), &json::object!{});

		root.add(vec![Node::binding("depth", Node::combined_image_sampler(), 1, DEPTH_BINDING.binding(), true, false), Node::binding("trace", Node::image("rgba16"), 1, TRACE_BINDING.binding(), false, true), Node::binding("diffuse", Node::combined_image_sampler(), 1, DIFFUSE_BINDING.binding(), true, false), ray_march, main]);

		let root = besl::lex(root).unwrap();

		let main = root.borrow().get_main().unwrap();

		let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&ShaderGenerationSettings::compute(Extent::square(32)), &main);

		glsl
	}
}

impl RenderPass for SSGIRenderPass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {
		unimplemented!()
	}
	
	fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {}
	
	fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: Extent) {
		command_buffer_recording.region("SSGI", |command_buffer| {
			command_buffer.region("Ray March", |command_buffer| {
				let command_buffer = command_buffer.bind_compute_pipeline(&self.ray_march);
				command_buffer.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
				command_buffer.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));
			});

			command_buffer.region("Denoise", |command_buffer| {
			});

			command_buffer.region("Apply", |command_buffer| {
			});
		});
	}

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {
		ghi.resize_image(self.trace, extent);
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