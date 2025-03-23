//! SSGI Render Pass
//! This module contains the implementation of the Screen Space Global Illumination (SSGI) render pass.

use crate::core::entity::{Entity, EntityBuilder, EntityHandle};
use std::{rc::Rc, sync::Arc};

use ghi::{GraphicsHardwareInterface, CommandBufferRecordable, BoundComputePipelineMode};

use maths_rs::Vec2f;
use resource_management::{asset::material_asset_handler::ProgramGenerator, image::Image, shader_generation::{ShaderGenerationSettings, ShaderGenerator}, ResourceManager};
use utils::{json, sync::RwLock, Extent};

use super::{common_shader_generator::CommonShaderGenerator, render_pass::{BilateralBlurPass, FullScreenRenderPass, RenderPass}, texture_manager::TextureManager};

/// The SSGI render pass.
pub struct SSGIRenderPass {
	trace: TracePass,
	downsample: FullScreenRenderPass,
	quarter_blur: BilateralBlurPass,
	quarter_to_half_upsample: UpsamplePass,
	half_blur: BilateralBlurPass,
	half_to_full_upsample: UpsamplePass,
	full_blur: BilateralBlurPass,
	// apply: ApplyPass,

	trace_map: ghi::ImageHandle,
	downsample_map: ghi::ImageHandle,
	x_quarter_blur_map: ghi::ImageHandle,
	y_quarter_blur_map: ghi::ImageHandle,
	upsample_map: ghi::ImageHandle,
	x_half_blur_map: ghi::ImageHandle,
	y_half_blur_map: ghi::ImageHandle,
	x_full_blur_map: ghi::ImageHandle,
	y_full_blur_map: ghi::ImageHandle,
}

pub const DEPTH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
pub const DIFFUSE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
pub const TRACE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

impl SSGIRenderPass {
	pub async fn new<'c>(ghi_lock: Rc<RwLock<ghi::GHI>>, resource_manager: EntityHandle<ResourceManager>, texture_manager: Arc<utils::r#async::RwLock<TextureManager>>, parent_descriptor_set_layout: ghi::DescriptorSetTemplateHandle, (depth_image, depth_sampler): (ghi::ImageHandle, ghi::SamplerHandle), diffuse_image: ghi::ImageHandle,) -> Self {
		let mut ghi = ghi_lock.write();
		let trace_map = ghi.build_image(ghi::image::Builder::new(Extent::square(0), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::BlitDestination).name("Trace").use_case(ghi::UseCases::DYNAMIC));

		let downsample_map = ghi.build_image(ghi::image::Builder::new(Extent::square(0), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image).name("Downsample").use_case(ghi::UseCases::DYNAMIC));
		
		let x_quarter_blur_map = ghi.build_image(ghi::image::Builder::new(Extent::square(0), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image).name("X Quarter SSGI Blur").use_case(ghi::UseCases::DYNAMIC));
		let y_quarter_blur_map = ghi.build_image(ghi::image::Builder::new(Extent::square(0), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferSource).name("Y Quarter SSGI Blur").use_case(ghi::UseCases::DYNAMIC));
		
		let x_half_blur_map = ghi.build_image(ghi::image::Builder::new(Extent::square(0), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image).name("X Half SSGI Blur").use_case(ghi::UseCases::DYNAMIC));
		let y_half_blur_map = ghi.build_image(ghi::image::Builder::new(Extent::square(0), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferSource).name("Y Half SSGI Blur").use_case(ghi::UseCases::DYNAMIC));

		let x_full_blur_map = ghi.build_image(ghi::image::Builder::new(Extent::square(0), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image).name("X Full SSGI Blur").use_case(ghi::UseCases::DYNAMIC));
		let y_full_blur_map = ghi.build_image(ghi::image::Builder::new(Extent::square(0), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferSource).name("Y Full SSGI Blur").use_case(ghi::UseCases::DYNAMIC));

		let upsample_map = ghi.build_image(ghi::image::Builder::new(Extent::square(0), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferDestination).name("Upsample").use_case(ghi::UseCases::DYNAMIC));
		
		let sampler = ghi.build_sampler(ghi::sampler::Builder::new().addressing_mode(ghi::SamplerAddressingModes::Mirror));
		let downsample = FullScreenRenderPass::new(&mut ghi, &DOWNSAMPLE_SHADER, &[DOWNSAMPLE_SOURCE_BINDING, DOWNSAMPLE_DESTINATION_BINDING], &(trace_map, sampler), downsample_map);

		let quarter_blur = BilateralBlurPass::new(&mut ghi, (depth_image, depth_sampler), downsample_map, x_quarter_blur_map, y_quarter_blur_map).await;
		let quarter_to_half_upsample = UpsamplePass::new(y_quarter_blur_map, upsample_map);
		let half_blur = BilateralBlurPass::new(&mut ghi, (depth_image, depth_sampler), upsample_map, x_half_blur_map, y_half_blur_map).await;
		let half_to_full_upsample = UpsamplePass::new(y_half_blur_map, trace_map);
		let full_blur = BilateralBlurPass::new(&mut ghi, (depth_image, depth_sampler), trace_map, x_full_blur_map, y_full_blur_map).await;
		drop(ghi);

		let trace = TracePass::new(ghi_lock.clone(), resource_manager.clone(), texture_manager.clone(), parent_descriptor_set_layout, (depth_image, depth_sampler), diffuse_image, trace_map).await;
		// let apply = ApplyPass::new(ghi_lock.clone(), resource_manager.clone(), texture_manager.clone(), diffuse_image, y_full_blur_map, result_map).await;

		SSGIRenderPass {
			trace,
			downsample,
			quarter_blur,
			quarter_to_half_upsample,
			half_blur,
			half_to_full_upsample,
			full_blur,
			// apply,

			trace_map,
			downsample_map,
			x_quarter_blur_map,
			y_quarter_blur_map,
			upsample_map,
			x_half_blur_map,
			y_half_blur_map,
			x_full_blur_map,
			y_full_blur_map,
		}
	}

	pub fn get_gi_map(&self) -> ghi::ImageHandle {
		self.y_full_blur_map
	}
}

impl RenderPass for SSGIRenderPass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {
		unimplemented!()
	}
	
	fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {}
	
	fn record(&self, command_buffer_recording: &mut ghi::CommandBufferRecording, extent: Extent) {
		command_buffer_recording.region("SSGI", |command_buffer| {
			self.trace.record(command_buffer, extent);
			self.downsample.record(command_buffer, extent / 4);
			self.quarter_blur.record(command_buffer, extent / 4);
			self.quarter_to_half_upsample.record(command_buffer, extent);
			self.half_blur.record(command_buffer, extent / 2);
			self.half_to_full_upsample.record(command_buffer, extent);
			self.full_blur.record(command_buffer, extent);
			// self.apply.record(command_buffer, extent);
		});
	}

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {
		ghi.resize_image(self.trace_map, extent);

		ghi.resize_image(self.downsample_map, extent / 4);
		ghi.resize_image(self.x_quarter_blur_map, extent / 4);
		ghi.resize_image(self.y_quarter_blur_map, extent / 4);
		ghi.resize_image(self.upsample_map, extent / 2);
		ghi.resize_image(self.x_half_blur_map, extent / 2);
		ghi.resize_image(self.y_half_blur_map, extent / 2);
		ghi.resize_image(self.x_full_blur_map, extent);
		ghi.resize_image(self.y_full_blur_map, extent);

		self.trace.resize(ghi, extent);
		self.downsample.resize(ghi, extent);
		self.quarter_blur.resize(ghi, extent / 4);
		self.quarter_to_half_upsample.resize(ghi, extent / 2);
		self.half_blur.resize(ghi, extent / 2);
		self.half_to_full_upsample.resize(ghi, extent);
		self.full_blur.resize(ghi, extent);
		// self.apply.resize(ghi, extent);
	}
}

/// This render pass traces ray against the scene to calculate the global illumination.
struct TracePass {
	ray_march: ghi::PipelineHandle,
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
}

impl TracePass {
	pub async fn new<'c>(ghi_lock: Rc<RwLock<ghi::GHI>>, resource_manager: EntityHandle<ResourceManager>, texture_manager: Arc<utils::r#async::RwLock<TextureManager>>, parent_descriptor_set_layout: ghi::DescriptorSetTemplateHandle, (depth_image, depth_sampler): (ghi::ImageHandle, ghi::SamplerHandle), diffuse_image: ghi::ImageHandle, trace_map: ghi::ImageHandle) -> Self {
		let resource_manager = resource_manager.read_sync();

		let mut blue_noise = resource_manager.request::<Image>("stbn_unitvec3_2Dx1D_128x128x64_0.png").await.unwrap();
		let (_, noise_texture, noise_sampler) = texture_manager.write().await.load(&mut blue_noise, ghi_lock.clone()).await.unwrap();

		let mut ghi = ghi_lock.write();

		let descriptor_set_template = ghi.create_descriptor_set_template(Some("SSGI"), &[DEPTH_BINDING, DIFFUSE_BINDING, TRACE_BINDING]);

		let pipeline_layout = ghi.create_pipeline_layout(&[parent_descriptor_set_layout, descriptor_set_template], &[]);

		let descriptor_set = ghi.create_descriptor_set(Some("SSGi"), &descriptor_set_template);

		let sampler = ghi.create_sampler(ghi::FilteringModes::Linear, ghi::SamplingReductionModes::WeightedAverage, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32);

		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DEPTH_BINDING, depth_image, depth_sampler, ghi::Layouts::Read));
		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&DIFFUSE_BINDING, diffuse_image, sampler, ghi::Layouts::Read));
		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&TRACE_BINDING, trace_map, ghi::Layouts::General));

		let ray_march_shader = ghi.create_shader(Some("SSGI Ray March"), ghi::ShaderSource::GLSL(Self::make_ray_march_normals_shader()), ghi::ShaderTypes::Compute, &vec![DEPTH_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ), DIFFUSE_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ), TRACE_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE)]).expect("Failed to create the ray march shader.");
		let ray_march = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&ray_march_shader, ghi::ShaderTypes::Compute));

		TracePass {
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

impl RenderPass for TracePass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {
		unimplemented!()
	}
	
	fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {}

	fn record(&self, command_buffer: &mut ghi::CommandBufferRecording, extent: Extent) {
		command_buffer.region("Ray March", |command_buffer| {
			let command_buffer = command_buffer.bind_compute_pipeline(&self.ray_march);
			command_buffer.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
			command_buffer.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));
		});
	}

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {
	}
}

pub const DOWNSAMPLE_SOURCE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
pub const DOWNSAMPLE_DESTINATION_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

const DOWNSAMPLE_SHADER: &'static str = r#"
#version 460 core
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_shader_explicit_arithmetic_types: enable

layout(row_major) uniform; layout(row_major) buffer;

layout(set=0, binding=0) uniform sampler2D source;
layout(set=0, binding=1) uniform writeonly image2D result;

vec2 make_uv(ivec2 coord, uvec2 extent) {
	return vec2(coord) / vec2(extent);
}

layout(local_size_x=16, local_size_y=16) in;
void main() {
	if (gl_GlobalInvocationID.x >= imageSize(result).x || gl_GlobalInvocationID.y >= imageSize(result).y) { return; }

	uvec2 texel = uvec2(gl_GlobalInvocationID.xy);

	vec4 lu_quad = texture(source, make_uv(ivec2(texel) * 4 + ivec2(-1, 1), textureSize(source, 0)));
	vec4 ru_quad = texture(source, make_uv(ivec2(texel) * 4 + ivec2(1, 1), textureSize(source, 0)));
	vec4 ld_quad = texture(source, make_uv(ivec2(texel) * 4 + ivec2(-1, -1), textureSize(source, 0)));
	vec4 rd_quad = texture(source, make_uv(ivec2(texel) * 4 + ivec2(1, -1), textureSize(source, 0)));

	vec4 value = (lu_quad + ru_quad + ld_quad + rd_quad) / 4.0;

	imageStore(result, ivec2(gl_GlobalInvocationID.xy), value);
}"#;

struct UpsamplePass {
	source: ghi::ImageHandle,
	destination: ghi::ImageHandle,
}

impl UpsamplePass {
	pub fn new(source_image: ghi::ImageHandle, destination_image: ghi::ImageHandle) -> Self {
		UpsamplePass {
			source: source_image,
			destination: destination_image,
		}
	}
}

impl RenderPass for UpsamplePass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {
		unimplemented!()
	}
	
	fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {}

	fn record(&self, command_buffer: &mut ghi::CommandBufferRecording, extent: Extent) {
		command_buffer.region("Upsample", |command_buffer| {
			command_buffer.blit_image(self.source, ghi::Layouts::Transfer, self.destination, ghi::Layouts::Transfer);
		});
	}

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {}
}

pub const APPLY_DIFFUSE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
pub const APPLY_TRACE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
pub const APPLY_RESULT_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

struct ApplyPass {
	// source: ghi::ImageHandle,
	// destination: ghi::ImageHandle,

	pipeline: ghi::PipelineHandle,
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
}

impl ApplyPass {
	pub async fn new<'c>(ghi_lock: Rc<RwLock<ghi::GHI>>, resource_manager: EntityHandle<ResourceManager>, texture_manager: Arc<utils::r#async::RwLock<TextureManager>>, diffuse_map: ghi::ImageHandle, trace_map: ghi::ImageHandle, result_image: ghi::ImageHandle) -> Self {
		let mut ghi = ghi_lock.write();

		let descriptor_set_template = ghi.create_descriptor_set_template(Some("SSGI Apply"), &[APPLY_DIFFUSE_BINDING, APPLY_TRACE_BINDING, APPLY_RESULT_BINDING]);

		let pipeline_layout = ghi.create_pipeline_layout(&[descriptor_set_template], &[]);

		let descriptor_set = ghi.create_descriptor_set(Some("SSGI Apply"), &descriptor_set_template);

		let sampler = ghi.build_sampler(ghi::sampler::Builder::new());

		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&APPLY_DIFFUSE_BINDING, diffuse_map, sampler, ghi::Layouts::Read));
		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&APPLY_TRACE_BINDING, trace_map, sampler, ghi::Layouts::Read));
		ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&APPLY_RESULT_BINDING, result_image, ghi::Layouts::General));

		let apply_shader = ghi.create_shader(Some("SSGI Apply"), ghi::ShaderSource::GLSL(Self::make_apply_shader()), ghi::ShaderTypes::Compute, &vec![APPLY_DIFFUSE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ), APPLY_TRACE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ), APPLY_RESULT_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE)]).expect("Failed to create the apply shader.");
		let pipeline = ghi.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&apply_shader, ghi::ShaderTypes::Compute));

		ApplyPass {
			pipeline,
			pipeline_layout,
			descriptor_set,
		}
	}

	fn make_apply_shader() -> String {
		let shader_generator = {
			let common_shader_generator = CommonShaderGenerator::new_with_params(false, false, false, false, false, true, false, true);
			common_shader_generator
		};

		use besl::parser::Node;

		const CODE: &str = "ivec2 coord = ivec2(gl_GlobalInvocationID.xy);
		uvec2 extent = uvec2(imageSize(result_map));
		vec2 uv = make_uv(coord, extent);
		vec4 trace = texture(trace_map, uv);
		vec4 diffuse = texture(diffuse_map, uv);
		vec4 result = diffuse + trace;
		imageStore(result_map, coord, vec4(result));
		";

		let main = Node::function("main", Vec::new(), "void", vec![Node::glsl(CODE, &["make_uv", "trace_map", "diffuse_map", "result_map"], vec![])]);

		let mut root = shader_generator.transform(Node::root(), &json::object!{});

		root.add(vec![
			Node::binding("trace_map", Node::combined_image_sampler(), 0, APPLY_TRACE_BINDING.binding(), true, false),
			Node::binding("diffuse_map", Node::combined_image_sampler(), 0, APPLY_DIFFUSE_BINDING.binding(), true, false),
			Node::binding("result_map", Node::image("rgba16"), 0, APPLY_RESULT_BINDING.binding(), false, true),
			main,
		]);

		let root = besl::lex(root).unwrap();

		let main = root.borrow().get_main().unwrap();

		let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&ShaderGenerationSettings::compute(Extent::square(32)), &main);

		glsl
	}
}

impl RenderPass for ApplyPass {
	fn add_render_pass(&mut self, render_pass: EntityHandle<dyn RenderPass>) {
		unimplemented!()
	}
	
	fn prepare(&self, ghi: &mut ghi::GHI, extent: Extent) {}

	fn record(&self, command_buffer: &mut ghi::CommandBufferRecording, extent: Extent) {
		command_buffer.region("Apply", |command_buffer| {
			let command_buffer = command_buffer.bind_compute_pipeline(&self.pipeline);
			command_buffer.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
			command_buffer.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));
		});
	}

	fn resize(&self, ghi: &mut ghi::GHI, extent: Extent) {
		// ghi.resize_image(self.source, extent);
		// ghi.resize_image(self.destination, extent);
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_make_ray_march_normals_shader() {
		let shader = TracePass::make_ray_march_normals_shader();

		if let Err(e) = ghi::compile_glsl("SSGI Trace Shader", &shader) {
			panic!("Failed to compile the shader\n{}", e);
		}
	}
}