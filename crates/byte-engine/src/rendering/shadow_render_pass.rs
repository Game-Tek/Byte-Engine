use core::EntityHandle;
use std::{io::Write, mem::transmute};

use maths_rs::{mat::{MatInverse, MatProjection, MatRotate3D, MatTranslate}, Mat4f};
use resource_management::{asset::material_asset_handler::ProgramGenerator, shader_generation::{ShaderGenerationSettings, ShaderGenerator}};
use utils::{json, Extent, RGBA};

use ghi::{GraphicsHardwareInterface, CommandBufferRecording, BoundRasterizationPipelineMode, RasterizationRenderPassMode};

use crate::{core::Entity, ghi, math, Vector3};

use super::{common_shader_generator::CommonShaderGenerator, csm, directional_light::DirectionalLight, view::View, visibility_model::render_domain::{Instance, LightData, LightingData, MeshData, ShaderViewData, MESHLET_DATA_BINDING, MESH_DATA_BINDING, PRIMITIVE_INDICES_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEWS_DATA_BINDING}, visibility_shader_generator::VisibilityShaderGenerator, world_render_domain::WorldRenderDomain};

pub struct ShadowRenderingPass {
	pipeline: ghi::PipelineHandle,
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	shadow_map: ghi::ImageHandle,
}

impl ShadowRenderingPass {
	pub fn new(ghi: &mut ghi::GHI, visibility_descriptor_set_template: &ghi::DescriptorSetTemplateHandle, view_depth_image: &ghi::ImageHandle) -> ShadowRenderingPass {
		let light_depth_map = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);
		let view_depth_map = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE);

		let bindings = [
			light_depth_map.clone(),
			view_depth_map.clone(),
		];

		let descriptor_set_template = ghi.create_descriptor_set_template(Some("Shadow Rendering Set Layout"), &bindings);

		// Push constant:
		// 0: view index
		// 1: instance index
		let pipeline_layout = ghi.create_pipeline_layout(&[visibility_descriptor_set_template.clone(), descriptor_set_template], &[ghi::PushConstantRange::new(0, 8)]);

		let descriptor_set = ghi.create_descriptor_set(Some("Shadow Rendering Descriptor Set"), &descriptor_set_template);
		
		let colored_shadow: bool = false;
		
		let shadow_map_resolution = Extent::square(4096);
		
		let shadow_map = ghi.build_image(ghi::image::ImageBuilder::new(shadow_map_resolution, ghi::Formats::Depth32, ghi::Uses::Image | ghi::Uses::Clear).name("Shadow Map").use_case(ghi::UseCases::DYNAMIC).array_layers(4));
		let sampler = ghi.build_sampler(ghi::sampler::Builder::new().addressing_mode(ghi::SamplerAddressingModes::Border {}));
		let lighting_data_buffer = ghi.create_buffer(Some("Lighting Data"), 1024, ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
		
		let shadow_map_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&light_depth_map, shadow_map, sampler, ghi::Layouts::Read));
		let view_depth_map_binding = ghi.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::combined_image_sampler(&view_depth_map, view_depth_image.clone(), sampler, ghi::Layouts::Read));

		let source_code = {
			let shader_generator = {
				let common_shader_generator = CommonShaderGenerator::new();
				common_shader_generator
			};

			let main_code = r#"
			View view = views.views[push_constant.view_index];
			
			process_meshlet(push_constant.instance_index, view.view_projection);
			"#;

			let lighting_data = besl::parser::Node::binding("lighting_data", besl::parser::Node::buffer("LightingData", vec![besl::parser::Node::member("light_count", "u32"), besl::parser::Node::member("lights", "Light[16]")]), 1, 2, true, false);
			let main = besl::parser::Node::function("main", Vec::new(), "void", vec![besl::parser::Node::glsl(main_code, &["push_constant", "process_meshlet", "views",], Vec::new())]);

			let root_node = besl::parser::Node::root();

			let mut root = shader_generator.transform(root_node, &json::object!{});

			let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("view_index", "u32"), besl::parser::Node::member("instance_index", "u32")]);

			root.add(vec![lighting_data, push_constant, main]);

			let root_node = besl::lex(root).unwrap();

			let main_node = root_node.borrow().get_main().unwrap();

			let glsl = ShaderGenerator::new().minified(!cfg!(debug_assertions)).compilation().generate_glsl_shader(&ShaderGenerationSettings::mesh(64, 126, Extent::line(128)), &main_node);

			glsl
		};

		let mesh_shader = ghi.create_shader(Some("Shadow Pass Mesh Shader"), ghi::ShaderSource::GLSL(source_code), ghi::ShaderTypes::Mesh, &[
			VIEWS_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			VERTEX_POSITIONS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			VERTEX_NORMALS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			VERTEX_UV_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			VERTEX_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			PRIMITIVE_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			MESHLET_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
		]).expect("Failed to create mesh shader");

		let pipeline = ghi.create_raster_pipeline(&[
			ghi::PipelineConfigurationBlocks::Layout { layout: &pipeline_layout },
			ghi::PipelineConfigurationBlocks::Shaders { shaders: &[ghi::ShaderParameter::new(&mesh_shader, ghi::ShaderTypes::Mesh)], },
			ghi::PipelineConfigurationBlocks::RenderTargets { targets: &[ghi::AttachmentInformation::new(shadow_map, ghi::Formats::Depth32, ghi::Layouts::RenderTarget, ghi::ClearValue::Depth(0.0f32), false, true)] },
		]);

		ShadowRenderingPass { pipeline, pipeline_layout, descriptor_set, shadow_map, }
	}

	pub fn render(&self, command_buffer_recording: &mut impl ghi::CommandBufferRecording, render_domain: &impl WorldRenderDomain, instances: &[Instance]) {
		command_buffer_recording.start_region("Shadow Rendering");

		let visibility_info = render_domain.get_visibility_info();

		for view in 0..4 {
			command_buffer_recording.start_region(&format!("Cascade {}", view));
			
			let binding = [ghi::AttachmentInformation::new(self.shadow_map, ghi::Formats::Depth32, ghi::Layouts::RenderTarget, ghi::ClearValue::Depth(0.0f32), false, true).layer(view)];
			let render_pass = command_buffer_recording.start_render_pass(Extent::square(4096), &binding);
			render_pass.bind_descriptor_sets(&self.pipeline_layout, &[render_domain.get_descriptor_set(), self.descriptor_set]);

			let pipeline_bind = render_pass.bind_raster_pipeline(&self.pipeline);

			for (i, instance) in instances.iter().enumerate() {
				pipeline_bind.write_push_constant(&self.pipeline_layout, 0, 1 + view as u32); // Write view index
				pipeline_bind.write_push_constant(&self.pipeline_layout, 4, i as u32); // Write instance index
				pipeline_bind.dispatch_meshes(instance.meshlet_count, 1, 1);
			}

			render_pass.end_render_pass();

			command_buffer_recording.end_region();
		}

		command_buffer_recording.end_region();
	}

	pub fn prepare(&self, ghi: &mut ghi::GHI, lights: &[EntityHandle<DirectionalLight>], views_data: &mut [ShaderViewData], lighting_data: &mut LightingData, primary_view: &View) {
		for (i, light) in lights.iter().enumerate() {
			let light = light.read_sync();
			
			let views = csm::make_csm_views(*primary_view, light.direction, 4);

			lighting_data.lights[i] = LightData {
				light_type: 'D' as u8,
				position: light.direction,
				color: light.color,
				cascades: [1, 2, 3, 4, 0, 0, 0, 0], // Hardcoded for now
			};

			for (j, view) in views.iter().enumerate() {
				views_data[1 + j] = ShaderViewData {
					fov: view.fov(),
					view: view.view(),
					projection: view.projection(),
					view_projection: view.projection_view(),
					inverse_view: view.view().inverse(),
					inverse_projection: view.projection().inverse(),
					inverse_view_projection: view.projection_view().inverse(),
					near: view.near(), far: view.far(),
				};
			}
		}

		lighting_data.count = lights.len() as u32;

		assert!(lighting_data.count < 16 as u32, "Light count exceeded");
	}

	pub fn get_shadow_map_image(&self) -> ghi::ImageHandle { self.shadow_map }
}

impl Entity for ShadowRenderingPass {}

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