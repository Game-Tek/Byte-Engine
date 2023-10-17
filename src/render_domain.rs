use std::{collections::HashMap, hash::Hash};

use log::{error, trace};
use maths_rs::{prelude::MatTranslate, Mat4f};

use crate::{resource_manager::{self, mesh_resource_handler, material_resource_handler::{Shader, Material, Variant}, texture_resource_handler}, rendering::{render_system::{RenderSystem, self}, directional_light::DirectionalLight, point_light::PointLight}, Extent, orchestrator::{Entity, System, self, OrchestratorReference}, Vector3, camera::{self, Camera}, math, window_system};

struct ToneMapPass {
	pipeline_layout: render_system::PipelineLayoutHandle,
	pipeline: render_system::PipelineHandle,
	descriptor_set_layout: render_system::DescriptorSetLayoutHandle,
	descriptor_set: render_system::DescriptorSetHandle,
}

/// This the visibility buffer implementation of the world render domain.
pub struct VisibilityWorldRenderDomain {
	pipeline_layout_handle: render_system::PipelineLayoutHandle,

	vertex_positions_buffer: render_system::BufferHandle,
	vertex_normals_buffer: render_system::BufferHandle,

	indices_buffer: render_system::BufferHandle,

	albedo: render_system::TextureHandle,
	depth_target: render_system::TextureHandle,
	result: render_system::TextureHandle,

	index_count: u32,
	instance_count: u32,
	render_finished_synchronizer: render_system::SynchronizerHandle,
	image_ready: render_system::SynchronizerHandle,
	render_command_buffer: render_system::CommandBufferHandle,
	camera_data_buffer_handle: render_system::BufferHandle,
	current_frame: usize,

	descriptor_set_layout: render_system::DescriptorSetLayoutHandle,
	descriptor_set: render_system::DescriptorSetHandle,

	transfer_synchronizer: render_system::SynchronizerHandle,
	transfer_command_buffer: render_system::CommandBufferHandle,

	meshes_data_buffer: render_system::BufferHandle,

	camera: Option<EntityHandle<crate::camera::Camera>>,

	meshes: HashMap<EntityHandle<Mesh>, u32>,

	mesh_resources: HashMap<&'static str, u32>,

	VERTEX_LAYOUT: [render_system::VertexElement; 2],

	/// Maps resource ids to shaders
	/// The hash and the shader handle are stored to determine if the shader has changed
	shaders: std::collections::HashMap<u64, (u64, render_system::ShaderHandle, render_system::ShaderTypes)>,

	
	swapchain_handles: Vec<render_system::SwapchainHandle>,
	
	visibility_pass_pipeline_layout: render_system::PipelineLayoutHandle,
	visibility_passes_descriptor_set: render_system::DescriptorSetHandle,
	visibility_pass_pipeline: render_system::PipelineHandle,

	material_count_pipeline: render_system::PipelineHandle,
	material_offset_pipeline: render_system::PipelineHandle,
	pixel_mapping_pipeline: render_system::PipelineHandle,

	material_id: render_system::TextureHandle,
	instance_id: render_system::TextureHandle,
	primitive_index: render_system::TextureHandle,

	material_count: render_system::BufferHandle,
	material_offset: render_system::BufferHandle,
	material_offset_scratch: render_system::BufferHandle,
	material_evaluation_dispatches: render_system::BufferHandle,
	material_xy: render_system::BufferHandle,

	material_evaluation_descriptor_set_layout: render_system::DescriptorSetLayoutHandle,
	material_evaluation_descriptor_set: render_system::DescriptorSetHandle,
	material_evaluation_pipeline_layout: render_system::PipelineLayoutHandle,

	material_evaluation_materials: HashMap<String, (u32, render_system::PipelineHandle)>,

	tone_map_pass: ToneMapPass,
	debug_position: render_system::TextureHandle,
	debug_normal: render_system::TextureHandle,
	light_data_buffer: render_system::BufferHandle,

	pending_texture_loads: Vec<render_system::TextureHandle>,
}

impl VisibilityWorldRenderDomain {
	pub fn new() -> orchestrator::EntityReturn<Self> {
		orchestrator::EntityReturn::new_from_function(move |orchestrator| {
			let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
			let mut render_system = render_system.get_mut();
			let render_system: &mut render_system::RenderSystemImplementation = render_system.downcast_mut().unwrap();

			let bindings = [
				render_system::DescriptorSetLayoutBinding {
					name: "CameraData",
					binding: 0,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::VERTEX,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MeshData",
					binding: 1,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::VERTEX,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "vertex positions",
					binding: 2,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::MESH,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "vertex normals",
					binding: 3,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::MESH,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "indices",
					binding: 4,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::MESH,
					immutable_samplers: None,
				},
			];

			let descriptor_set_layout = render_system.create_descriptor_set_layout(&bindings);

			let descriptor_set = render_system.create_descriptor_set(&descriptor_set_layout, &bindings);

			let pipeline_layout_handle = render_system.create_pipeline_layout(&[descriptor_set_layout], &[render_system::PushConstantRange{ offset: 0, size: 16 }]);
			
			let vertex_positions_buffer_handle = render_system.create_buffer(Some("Visibility Vertex Positions Buffer"), 1024 * 1024 * 16, render_system::Uses::Vertex | render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let vertex_normals_buffer_handle = render_system.create_buffer(Some("Visibility Vertex Normals Buffer"), 1024 * 1024 * 16, render_system::Uses::Vertex | render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			let indices_buffer_handle = render_system.create_buffer(Some("Visibility Index Buffer"), 1024 * 1024 * 16, render_system::Uses::Index | render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			let debug_position = render_system.create_texture(Some("debug position"), Extent::new(1920, 1080, 1), render_system::TextureFormats::RGBAu16, None, render_system::Uses::RenderTarget | render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
			let debug_normal = render_system.create_texture(Some("debug normal"), Extent::new(1920, 1080, 1), render_system::TextureFormats::RGBAu16, None, render_system::Uses::RenderTarget | render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let albedo = render_system.create_texture(Some("albedo"), Extent::new(1920, 1080, 1), render_system::TextureFormats::RGBAu16, None, render_system::Uses::RenderTarget | render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
			let depth_target = render_system.create_texture(Some("depth_target"), Extent::new(1920, 1080, 1), render_system::TextureFormats::Depth32, None, render_system::Uses::DepthStencil, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let render_finished_synchronizer = render_system.create_synchronizer(true);
			let image_ready = render_system.create_synchronizer(false);

			let transfer_synchronizer = render_system.create_synchronizer(false);

			let render_command_buffer = render_system.create_command_buffer();
			let transfer_command_buffer = render_system.create_command_buffer();

			let camera_data_buffer_handle = render_system.create_buffer(Some("Visibility Camera Data"), 16 * 4 * 4, render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let meshes_data_buffer = render_system.create_buffer(Some("Visibility Meshes Data"), 16 * 4 * 4 * 16, render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			render_system.write(&[
				render_system::DescriptorWrite {
					descriptor_set,
					binding: 0,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: camera_data_buffer_handle, size: 64 },
				},
				render_system::DescriptorWrite {
					descriptor_set,
					binding: 1,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: meshes_data_buffer, size: 64 },
				},
				render_system::DescriptorWrite {
					descriptor_set,
					binding: 2,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: vertex_positions_buffer_handle, size: 512 },
				},
				render_system::DescriptorWrite {
					descriptor_set,
					binding: 3,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: vertex_normals_buffer_handle, size: 512 },
				},
				render_system::DescriptorWrite {
					descriptor_set,
					binding: 4,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: indices_buffer_handle, size: 512 },
				},
			]);

			let visibility_pass_vertex_source = r#"
				#version 450
				#pragma shader_stage(vertex)

				#extension GL_EXT_scalar_block_layout: enable
				#extension GL_EXT_buffer_reference: enable
				#extension GL_EXT_buffer_reference2: enable

				layout(row_major) uniform; layout(row_major) buffer;

				layout(location=0) in vec3 in_position;
				layout(location=1) in vec3 in_normal;

				layout(location=0) out uint out_instance_index;
				layout(location=1) out uint out_vertex_id;

				layout(scalar, buffer_reference) buffer CameraData {
					mat4 view_matrix;
					mat4 projection_matrix;
					mat4 view_projection;
				};

				layout(scalar, buffer_reference, buffer_reference_align=1) buffer MeshData {
					mat4 model;
					uint material_id;
				};

				layout(push_constant) uniform PushConstant {
					CameraData camera;
					MeshData meshes;
				} pc;

				void main() {
					gl_Position = pc.camera.view_projection * pc.meshes[gl_InstanceIndex].model * vec4(in_position, 1.0);
					out_instance_index = gl_InstanceIndex;
					out_vertex_id = gl_VertexIndex;
				}
			"#;

			let visibility_pass_mesh_source = r#"
				#version 450
				#pragma shader_stage(mesh)

				#extension GL_EXT_scalar_block_layout: enable
				#extension GL_EXT_buffer_reference: enable
				#extension GL_EXT_buffer_reference2: enable
				#extension GL_EXT_shader_16bit_storage: require
				#extension GL_EXT_mesh_shader: require

				layout(row_major) uniform; layout(row_major) buffer;

				layout(location=0) perprimitiveEXT out uint out_instance_index[12];
				layout(location=1) perprimitiveEXT out uint out_primitive_index[12];

				layout(scalar, buffer_reference) buffer CameraData {
					mat4 view_matrix;
					mat4 projection_matrix;
					mat4 view_projection;
				};

				layout(scalar, buffer_reference, buffer_reference_align=1) buffer MeshData {
					mat4 model;
					uint material_id;
				};

				layout(set=0,binding=2,scalar) buffer readonly MeshVertexPositions {
					vec3 vertex_positions[];
				};

				layout(set=0,binding=4,scalar) buffer readonly MeshIndices {
					uint16_t indices[];
				};

				layout(push_constant) uniform PushConstant {
					CameraData camera;
					MeshData meshes;
				} pc;

				layout(triangles, max_vertices=24, max_primitives=12) out;
				
				layout(local_size_x=32) in;
				void main() {
					SetMeshOutputsEXT(24, 12);

					if (gl_LocalInvocationID.x < 24) {
						gl_MeshVerticesEXT[gl_LocalInvocationID.x].gl_Position = pc.camera.view_projection * pc.meshes[0].model * vec4(vertex_positions[gl_LocalInvocationID.x], 1.0);
					}
					
					if (gl_LocalInvocationID.x < 12) {
						uint triangle_indices[3] = uint[](uint(indices[gl_LocalInvocationID.x * 3 + 0]), uint(indices[gl_LocalInvocationID.x * 3 + 1]), uint(indices[gl_LocalInvocationID.x * 3 + 2]));
						gl_PrimitiveTriangleIndicesEXT[gl_LocalInvocationID.x] = uvec3(triangle_indices[0], triangle_indices[1], triangle_indices[2]);
						out_instance_index[gl_LocalInvocationID.x] = 0;
						out_primitive_index[gl_LocalInvocationID.x] = gl_LocalInvocationID.x * 3;
					}
				}
			"#;

			let visibility_pass_fragment_source = r#"
				#version 450
				#pragma shader_stage(fragment)

				#extension GL_EXT_scalar_block_layout: enable
				#extension GL_EXT_buffer_reference: enable
				#extension GL_EXT_buffer_reference2: enable
				#extension GL_EXT_mesh_shader: require

				layout(location=0) perprimitiveEXT flat in uint in_instance_index;
				layout(location=1) perprimitiveEXT flat in uint in_primitive_index;
				
				layout(scalar, buffer_reference) buffer CameraData {
					mat4 view_matrix;
					mat4 projection_matrix;
					mat4 view_projection;
				};

				layout(scalar, buffer_reference, buffer_reference_align=1) buffer MeshData {
					mat4 model;
					uint material_id;
				};

				layout(push_constant) uniform PushConstant {
					CameraData camera;
					MeshData meshes;
				} pc;

				layout(location=0) out uint out_material_id;
				layout(location=1) out uint out_primitive_index;
				layout(location=2) out uint out_instance_id;

				void main() {
					out_material_id = pc.meshes[in_instance_index].material_id;
					out_primitive_index = in_primitive_index;
					out_instance_id = in_instance_index;
				}
			"#;

			// let visibility_pass_vertex_shader = render_system.create_shader(render_system::ShaderSourceType::GLSL, render_system::ShaderTypes::Vertex, visibility_pass_vertex_source.as_bytes());
			let visibility_pass_mesh_shader = render_system.create_shader(render_system::ShaderSourceType::GLSL, render_system::ShaderTypes::Mesh, visibility_pass_mesh_source.as_bytes());
			let visibility_pass_fragment_shader = render_system.create_shader(render_system::ShaderSourceType::GLSL, render_system::ShaderTypes::Fragment, visibility_pass_fragment_source.as_bytes());

			let visibility_pass_shaders = [
				(&visibility_pass_mesh_shader, render_system::ShaderTypes::Mesh, vec![]),
				(&visibility_pass_fragment_shader, render_system::ShaderTypes::Fragment, vec![]),
			];

			let material_id = render_system.create_texture(Some("material_id"), crate::Extent::new(1920, 1080, 1), render_system::TextureFormats::U32, None, render_system::Uses::RenderTarget | render_system::Uses::Storage, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
			let primitive_index = render_system.create_texture(Some("primitive index"), crate::Extent::new(1920, 1080, 1), render_system::TextureFormats::U32, None, render_system::Uses::RenderTarget | render_system::Uses::Storage, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
			let instance_id = render_system.create_texture(Some("instance_id"), crate::Extent::new(1920, 1080, 1), render_system::TextureFormats::U32, None, render_system::Uses::RenderTarget | render_system::Uses::Storage, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let attachments = [
				render_system::AttachmentInformation {
					texture: material_id,
					layout: render_system::Layouts::RenderTarget,
					format: render_system::TextureFormats::U32,
					clear: render_system::ClearValue::Integer(!0u32, 0, 0, 0),
					load: false,
					store: true,
				},
				render_system::AttachmentInformation {
					texture: primitive_index,
					layout: render_system::Layouts::RenderTarget,
					format: render_system::TextureFormats::U32,
					clear: render_system::ClearValue::Integer(!0u32, 0, 0, 0),
					load: false,
					store: true,
				},
				render_system::AttachmentInformation {
					texture: instance_id,
					layout: render_system::Layouts::RenderTarget,
					format: render_system::TextureFormats::U32,
					clear: render_system::ClearValue::Integer(!0u32, 0, 0, 0),
					load: false,
					store: true,
				},
				render_system::AttachmentInformation {
					texture: depth_target,
					layout: render_system::Layouts::RenderTarget,
					format: render_system::TextureFormats::Depth32,
					clear: render_system::ClearValue::Depth(0f32),
					load: false,
					store: true,
				},
			];

			let VERTEX_LAYOUT = [
				render_system::VertexElement{ name: "POSITION".to_string(), format: render_system::DataTypes::Float3, binding: 0 },
				render_system::VertexElement{ name: "NORMAL".to_string(), format: render_system::DataTypes::Float3, binding: 1 },
			];

			let visibility_pass_pipeline = render_system.create_raster_pipeline(&[
				render_system::PipelineConfigurationBlocks::Layout { layout: &pipeline_layout_handle },
				render_system::PipelineConfigurationBlocks::Shaders { shaders: &visibility_pass_shaders },
				// render_system::PipelineConfigurationBlocks::VertexInput { vertex_elements: &VERTEX_LAYOUT },
				render_system::PipelineConfigurationBlocks::RenderTargets { targets: &attachments },
			]);

			let material_count = render_system.create_buffer(Some("Material Count"), 1024 /* max materials */ * 4 /* u32 size */, render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let material_offset = render_system.create_buffer(Some("Material Offset"), 1024 /* max materials */ * 4 /* u32 size */, render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let material_offset_scratch = render_system.create_buffer(Some("Material Offset Scratch"), 1024 /* max materials */ * 4 /* u32 size */, render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let material_evaluation_dispatches = render_system.create_buffer(Some("Material Evaluation Dipatches"), 1024 /* max materials */ * 12 /* uvec3 size */, render_system::Uses::Storage | render_system::Uses::TransferDestination | render_system::Uses::Indirect, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			let material_xy = render_system.create_buffer(Some("Material XY"), 1920 * 1080 * 2 * 2, render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			let material_count_source = r#"
				#version 450
				#pragma shader_stage(compute)

				#extension GL_EXT_scalar_block_layout: enable
				#extension GL_EXT_buffer_reference2: enable

				layout(scalar, set=0, binding=0) buffer MaterialCount {
					uint material_count[];
				};

				layout(set=0, binding=5, r8ui) uniform readonly uimage2D material_id;

				layout(local_size_x=32, local_size_y=32) in;
				void main() {
					// If thread is out of bound respect to the material_id texture, return
					if (gl_GlobalInvocationID.x >= imageSize(material_id).x || gl_GlobalInvocationID.y >= imageSize(material_id).y) { return; }

					uint material_index = imageLoad(material_id, ivec2(gl_GlobalInvocationID.xy)).r;

					if (material_index != 0xFFFFFFFF) {
						atomicAdd(material_count[material_index], 1);
					}
				}
			"#;

			let material_offset_source = r#"
				#version 450
				#pragma shader_stage(compute)

				#extension GL_EXT_scalar_block_layout: enable
				#extension GL_EXT_buffer_reference2: enable

				layout(scalar, binding=0) buffer MaterialCount {
					uint material_count[];
				};

				layout(scalar, binding=1) buffer MaterialOffset {
					uint material_offset[];
				};

				layout(scalar, binding=2) buffer MaterialOffsetScratch {
					uint material_offset_scratch[];
				};

				layout(scalar, binding=3) buffer MaterialEvaluationDispatches {
					uvec3 material_evaluation_dispatches[];
				};

				layout(local_size_x=1) in;
				void main() {
					uint sum = 0;

					for (uint i = 0; i < 4; i++) {
						material_offset[i] = sum;
						material_offset_scratch[i] = sum;
						material_evaluation_dispatches[i] = uvec3((material_count[i] + 31) / 32, 1, 1);
						sum += material_count[i];
					}
				}
			"#;

			let pixel_mapping_source = r#"
				#version 450
				#pragma shader_stage(compute)

				#extension GL_EXT_scalar_block_layout: enable
				#extension GL_EXT_buffer_reference2: enable
				#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable

				layout(scalar, binding=1) buffer MaterialOffset {
					uint material_offset[];
				};

				layout(scalar, binding=2) buffer MaterialOffsetScratch {
					uint material_offset_scratch[];
				};

				layout(scalar, binding=4) buffer PixelMapping {
					u16vec2 pixel_mapping[];
				};

				layout(set=0, binding=5, r8ui) uniform readonly uimage2D material_id;

				layout(local_size_x=32, local_size_y=32) in;
				void main() {
					// If thread is out of bound respect to the material_id texture, return
					if (gl_GlobalInvocationID.x >= imageSize(material_id).x || gl_GlobalInvocationID.y >= imageSize(material_id).y) { return; }

					uint material_index = imageLoad(material_id, ivec2(gl_GlobalInvocationID.xy)).r;

					if (material_index == 0xFFFFFFFF) { return; }

					uint offset = atomicAdd(material_offset_scratch[material_index], 1);

					pixel_mapping[offset] = u16vec2(gl_GlobalInvocationID.xy);
				}
			"#;

			let bindings = [
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialCount",
					binding: 0,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialOffset",
					binding: 1,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialOffsetScratch",
					binding: 2,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialEvaluationDispatches",
					binding: 3,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialXY",
					binding: 4,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialId",
					binding: 5,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "VertexId",
					binding: 6,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "InstanceId",
					binding: 7,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
			];

			let visibility_descriptor_set_layout = render_system.create_descriptor_set_layout(&bindings);
			let visibility_pass_pipeline_layout = render_system.create_pipeline_layout(&[visibility_descriptor_set_layout], &[render_system::PushConstantRange{ offset: 0, size: 16 }]);
			let visibility_passes_descriptor_set = render_system.create_descriptor_set(&visibility_descriptor_set_layout, &bindings);

			render_system.write(&[
				render_system::DescriptorWrite { // MaterialCount
					descriptor_set: visibility_passes_descriptor_set,
					binding: 0,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_count, size: 1024 * 4 },
				},
				render_system::DescriptorWrite { // MaterialOffset
					descriptor_set: visibility_passes_descriptor_set,
					binding: 1,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_offset, size: 1024 * 4 },
				},
				render_system::DescriptorWrite { // MaterialOffsetScratch
					descriptor_set: visibility_passes_descriptor_set,
					binding: 2,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_offset_scratch, size: 1024 * 4 },
				},
				render_system::DescriptorWrite { // MaterialEvaluationDispatches
					descriptor_set: visibility_passes_descriptor_set,
					binding: 3,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_evaluation_dispatches, size: 1024 * 12 },
				},
				render_system::DescriptorWrite { // MaterialXY
					descriptor_set: visibility_passes_descriptor_set,
					binding: 4,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_xy, size: 1920 * 1080 * 2 * 2 },
				},
				render_system::DescriptorWrite { // MaterialId
					descriptor_set: visibility_passes_descriptor_set,
					binding: 5,
					array_element: 0,
					descriptor: render_system::Descriptor::Texture(material_id),
				},
				render_system::DescriptorWrite { // MaterialId
					descriptor_set: visibility_passes_descriptor_set,
					binding: 6,
					array_element: 0,
					descriptor: render_system::Descriptor::Texture(primitive_index),
				},
				render_system::DescriptorWrite { // InstanceId
					descriptor_set: visibility_passes_descriptor_set,
					binding: 7,
					array_element: 0,
					descriptor: render_system::Descriptor::Texture(instance_id),
				},
			]);

			let material_count_shader = render_system.create_shader(render_system::ShaderSourceType::GLSL, render_system::ShaderTypes::Compute, material_count_source.as_bytes());
			let material_count_pipeline = render_system.create_compute_pipeline(&visibility_pass_pipeline_layout, (&material_count_shader, render_system::ShaderTypes::Compute, vec![]));

			let material_offset_shader = render_system.create_shader(render_system::ShaderSourceType::GLSL, render_system::ShaderTypes::Compute, material_offset_source.as_bytes());
			let material_offset_pipeline = render_system.create_compute_pipeline(&visibility_pass_pipeline_layout, (&material_offset_shader, render_system::ShaderTypes::Compute, vec![]));

			let pixel_mapping_shader = render_system.create_shader(render_system::ShaderSourceType::GLSL, render_system::ShaderTypes::Compute, pixel_mapping_source.as_bytes());
			let pixel_mapping_pipeline = render_system.create_compute_pipeline(&visibility_pass_pipeline_layout, (&pixel_mapping_shader, render_system::ShaderTypes::Compute, vec![]));

			let light_data_buffer = render_system.create_buffer(Some("Light Data"), 1024 * 4, render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let lighting_data = unsafe { (render_system.get_mut_buffer_slice(light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };

			lighting_data.count = 0; // Initially, no lights

			let bindings = [
				render_system::DescriptorSetLayoutBinding {
					name: "albedo",
					binding: 0,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MeshBuffer",
					binding: 1,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "Positions",
					binding: 2,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "Normals",
					binding: 3,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "Indeces",
					binding: 4,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "CameraData",
					binding: 5,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MeshData",
					binding: 6,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "debug_position",
					binding: 7,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "debug_normals",
					binding: 8,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "LightData",
					binding: 9,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stage_flags: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
			];	

			let material_evaluation_descriptor_set_layout = render_system.create_descriptor_set_layout(&bindings);
			let material_evaluation_descriptor_set = render_system.create_descriptor_set(&material_evaluation_descriptor_set_layout, &bindings);

			render_system.write(&[
				render_system::DescriptorWrite { // albedo
					descriptor_set: material_evaluation_descriptor_set,
					binding: 0,
					array_element: 0,
					descriptor: render_system::Descriptor::Texture(albedo),
				},
				render_system::DescriptorWrite { // MeshBuffer
					descriptor_set: material_evaluation_descriptor_set,
					binding: 1,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: meshes_data_buffer, size: 1024 * 4 },
				},
				render_system::DescriptorWrite { // Positions
					descriptor_set: material_evaluation_descriptor_set,
					binding: 2,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: vertex_positions_buffer_handle, size: 1024 * 4 },
				},
				render_system::DescriptorWrite { // Normals
					descriptor_set: material_evaluation_descriptor_set,
					binding: 3,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: vertex_normals_buffer_handle, size: 1024 * 4 },
				},
				render_system::DescriptorWrite { // Indeces
					descriptor_set: material_evaluation_descriptor_set,
					binding: 4,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: indices_buffer_handle, size: 1024 * 4 },
				},
				render_system::DescriptorWrite { // CameraData
					descriptor_set: material_evaluation_descriptor_set,
					binding: 5,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: camera_data_buffer_handle, size: 256 },
				},
				render_system::DescriptorWrite { // MeshData
					descriptor_set: material_evaluation_descriptor_set,
					binding: 6,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: meshes_data_buffer, size: 256 },
				},
				render_system::DescriptorWrite { // debug_position
					descriptor_set: material_evaluation_descriptor_set,
					binding: 7,
					array_element: 0,
					descriptor: render_system::Descriptor::Texture(debug_position),
				},
				render_system::DescriptorWrite { // debug_normals
					descriptor_set: material_evaluation_descriptor_set,
					binding: 8,
					array_element: 0,
					descriptor: render_system::Descriptor::Texture(debug_normal),
				},
				render_system::DescriptorWrite { // LightData
					descriptor_set: material_evaluation_descriptor_set,
					binding: 9,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: light_data_buffer, size: 1024 * 4 },
				},
			]);

			let material_evaluation_pipeline_layout = render_system.create_pipeline_layout(&[visibility_descriptor_set_layout, material_evaluation_descriptor_set_layout], &[render_system::PushConstantRange{ offset: 0, size: 28 }]);

			let tone_mapping_shader = r#"
			#version 450
			#pragma shader_stage(compute)

			#extension GL_EXT_scalar_block_layout: enable
			#extension GL_EXT_buffer_reference2: enable
			#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable

			layout(set=0, binding=0, rgba16) uniform readonly image2D source;
			layout(set=0, binding=1, rgba8) uniform image2D result;

			vec3 ACESNarkowicz(vec3 x) {
				const float a = 2.51;
				const float b = 0.03;
				const float c = 2.43;
				const float d = 0.59;
				const float e = 0.14;
				return clamp((x*(a*x+b))/(x*(c*x+d)+e), 0.0, 1.0);
			}

			const mat3 ACES_INPUT_MAT = mat3(
				vec3( 0.59719,  0.35458,  0.04823),
				vec3( 0.07600,  0.90834,  0.01566),
				vec3( 0.02840,  0.13383,  0.83777)
			);

			const mat3 ACES_OUTPUT_MAT = mat3(
				vec3( 1.60475, -0.53108, -0.07367),
				vec3(-0.10208,  1.10813, -0.00605),
				vec3(-0.00327, -0.07276,  1.07602)
			);

			vec3 RRTAndODTFit(vec3 v) {
				vec3 a = v * (v + 0.0245786) - 0.000090537;
				vec3 b = v * (0.983729 * v + 0.4329510) + 0.238081;
				return a / b;
			}

			vec3 ACESFitted(vec3 x) {
				return clamp(ACES_OUTPUT_MAT * RRTAndODTFit(ACES_INPUT_MAT * x), 0.0, 1.0);
			}

			layout(local_size_x=32, local_size_y=32) in;
			void main() {
				if (gl_GlobalInvocationID.x >= imageSize(source).x || gl_GlobalInvocationID.y >= imageSize(source).y) { return; }

				vec4 source_color = imageLoad(source, ivec2(gl_GlobalInvocationID.xy));

				vec3 result_color = ACESNarkowicz(source_color.rgb);

				result_color = pow(result_color, vec3(1.0 / 2.2));

				imageStore(result, ivec2(gl_GlobalInvocationID.xy), vec4(result_color, 1.0));
			}
			"#;

			let result = render_system.create_texture(Some("result"), Extent::new(1920, 1080, 1), render_system::TextureFormats::RGBAu8, None, render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let tone_map_pass = {
				let descriptor_set_layout = render_system.create_descriptor_set_layout(&[
					render_system::DescriptorSetLayoutBinding {
						name: "source",
						binding: 0,
						descriptor_type: render_system::DescriptorType::StorageImage,
						descriptor_count: 1,
						stage_flags: render_system::Stages::COMPUTE,
						immutable_samplers: None,
					},
					render_system::DescriptorSetLayoutBinding {
						name: "result",
						binding: 1,
						descriptor_type: render_system::DescriptorType::StorageImage,
						descriptor_count: 1,
						stage_flags: render_system::Stages::COMPUTE,
						immutable_samplers: None,
					},
				]);

				let pipeline_layout = render_system.create_pipeline_layout(&[descriptor_set_layout], &[]);

				let descriptor_set = render_system.create_descriptor_set(&descriptor_set_layout, &[
					render_system::DescriptorSetLayoutBinding {
						name: "source",
						binding: 0,
						descriptor_type: render_system::DescriptorType::StorageImage,
						descriptor_count: 1,
						stage_flags: render_system::Stages::COMPUTE,
						immutable_samplers: None,
					},
					render_system::DescriptorSetLayoutBinding {
						name: "result",
						binding: 1,
						descriptor_type: render_system::DescriptorType::StorageImage,
						descriptor_count: 1,
						stage_flags: render_system::Stages::COMPUTE,
						immutable_samplers: None,
					},
				]);

				render_system.write(&[
					render_system::DescriptorWrite {
						descriptor_set,
						binding: 0,
						array_element: 0,
						descriptor: render_system::Descriptor::Texture(albedo),
					},
					render_system::DescriptorWrite {
						descriptor_set,
						binding: 1,
						array_element: 0,
						descriptor: render_system::Descriptor::Texture(result),
					},
				]);

				let tone_mapping_shader = render_system.create_shader(render_system::ShaderSourceType::GLSL, render_system::ShaderTypes::Compute, tone_mapping_shader.as_bytes());
				let tone_mapping_pipeline = render_system.create_compute_pipeline(&pipeline_layout, (&tone_mapping_shader, render_system::ShaderTypes::Compute, vec![]));

				ToneMapPass {
					descriptor_set_layout,
					pipeline_layout,
					descriptor_set,
					pipeline: tone_mapping_pipeline,
				}
			};

			Self {
				pipeline_layout_handle,

				vertex_positions_buffer: vertex_positions_buffer_handle,
				vertex_normals_buffer: vertex_normals_buffer_handle,
				indices_buffer: indices_buffer_handle,

				descriptor_set_layout,
				descriptor_set,

				albedo,
				depth_target,
				result,

				index_count: 0,
				instance_count: 0,
				current_frame: 0,

				render_finished_synchronizer,
				image_ready,
				render_command_buffer,

				camera_data_buffer_handle,

				transfer_synchronizer,
				transfer_command_buffer,

				meshes_data_buffer,

				shaders: HashMap::new(),

				camera: None,

				meshes: HashMap::new(),
				light_data_buffer,

				mesh_resources: HashMap::new(),

				VERTEX_LAYOUT,

				swapchain_handles: Vec::new(),

				visibility_pass_pipeline_layout,
				visibility_passes_descriptor_set,
				visibility_pass_pipeline,

				material_count_pipeline,
				material_offset_pipeline,
				pixel_mapping_pipeline,

				material_evaluation_descriptor_set_layout,
				material_evaluation_descriptor_set,
				material_evaluation_pipeline_layout,

				material_id,
				primitive_index,
				instance_id,

				debug_position,
				debug_normal,

				material_count,
				material_offset,
				material_offset_scratch,
				material_evaluation_dispatches,
				material_xy,

				material_evaluation_materials: HashMap::new(),

				tone_map_pass,

				pending_texture_loads: Vec::new(),
			}
		})
			// .add_post_creation_function(Box::new(Self::load_needed_assets))
			.add_listener::<camera::Camera>()
			.add_listener::<Mesh>()
			.add_listener::<window_system::Window>()
			.add_listener::<DirectionalLight>()
			.add_listener::<PointLight>()
	}

	fn load_material(&mut self, resource_manager: &mut resource_manager::ResourceManager, render_system: &mut render_system::RenderSystemImplementation, asset_url: &str) {
		let (response, buffer) = resource_manager.get(asset_url).unwrap();

		for resource_document in &response.resources {
			match resource_document.class.as_str() {
				"Texture" => {
					let texture: &texture_resource_handler::Texture = resource_document.resource.downcast_ref().unwrap();

					let compression = if let Some(compression) = &texture.compression {
						match compression {
							texture_resource_handler::CompressionSchemes::BC7 => Some(render_system::CompressionSchemes::BC7)
						}
					} else {
						None
					};

					let new_texture = render_system.create_texture(Some(&resource_document.url), texture.extent, render_system::TextureFormats::RGBAu8, compression, render_system::Uses::Texture | render_system::Uses::TransferDestination, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

					render_system.get_texture_slice_mut(new_texture).copy_from_slice(&buffer[resource_document.offset as usize..(resource_document.offset + resource_document.size) as usize]);

					self.pending_texture_loads.push(new_texture);
				}
				"Shader" => {
					let shader: &Shader = resource_document.resource.downcast_ref().unwrap();

					let hash = resource_document.hash; let resource_id = resource_document.id;

					if let Some((old_hash, _old_shader, _)) = self.shaders.get(&resource_id) {
						if *old_hash == hash { continue; }
					}

					let offset = resource_document.offset as usize;
					let size = resource_document.size as usize;

					let new_shader = render_system.create_shader(render_system::ShaderSourceType::SPIRV, shader.stage, &buffer[offset..(offset + size)]);

					self.shaders.insert(resource_id, (hash, new_shader, shader.stage));
				}
				"Variant" => {
					if !self.material_evaluation_materials.contains_key(&resource_document.url) {
						let variant: &Variant = resource_document.resource.downcast_ref().unwrap();

						let material_resource_document = response.resources.iter().find(|r| &r.url == &variant.parent).unwrap();

						let shaders = material_resource_document.required_resources.iter().map(|f| response.resources.iter().find(|r| &r.url == f).unwrap().id).collect::<Vec<_>>();

						let shaders = shaders.iter().map(|shader| {
							let (_hash, shader, shader_type) = self.shaders.get(shader).unwrap();

							(shader, *shader_type)
						}).collect::<Vec<_>>();

						let targets = [
							render_system::AttachmentInformation {
								texture: self.albedo,
								layout: render_system::Layouts::RenderTarget,
								format: render_system::TextureFormats::RGBAu8,
								clear: render_system::ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
								load: false,
								store: true,
							},
							render_system::AttachmentInformation {
								texture: self.depth_target,
								layout: render_system::Layouts::RenderTarget,
								format: render_system::TextureFormats::Depth32,
								clear: render_system::ClearValue::Depth(0f32),
								load: false,
								store: true,
							},
						];

						let mut specialization_constants: Vec<Box<dyn render_system::SpecializationMapEntry>> = vec![];

						for (i, variable) in variant.variables.iter().enumerate() {
							// TODO: use actual variable type

							match variable.value.as_str() {
								"White" => {
									specialization_constants.push(
										Box::new(render_system::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [1f32, 1f32, 1f32, 1f32] })
									);
								}
								"Red" => {
									specialization_constants.push(
										Box::new(render_system::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [1f32, 0f32, 0f32, 1f32] })
									);
								}
								"Green" => {
									specialization_constants.push(
										Box::new(render_system::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [0f32, 1f32, 0f32, 1f32] })
									);
								}
								"Blue" => {
									specialization_constants.push(
										Box::new(render_system::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [0f32, 0f32, 1f32, 1f32] })
									);
								}
								"Purple" => {
									specialization_constants.push(
										Box::new(render_system::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [1f32, 0f32, 1f32, 1f32] })
									);
								}
								"Yellow" => {
									specialization_constants.push(
										Box::new(render_system::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [1f32, 1f32, 0f32, 1f32] })
									);
								}
								"Black" => {
									specialization_constants.push(
										Box::new(render_system::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [0f32, 0f32, 0f32, 1f32] })
									);
								}
								_ => {
									error!("Unknown variant value: {}", variable.value);
								}
							}

						}

						let material = self.material_evaluation_materials.get(&variant.parent).unwrap();

						let pipeline = render_system.create_compute_pipeline(&self.material_evaluation_pipeline_layout, (&shaders[0].0, render_system::ShaderTypes::Compute, specialization_constants));
						
						self.material_evaluation_materials.insert(resource_document.url.clone(), (self.material_evaluation_materials.len() as u32, pipeline));
					}
				}
				"Material" => {
					if !self.material_evaluation_materials.contains_key(&resource_document.url) {
						let material: &Material = resource_document.resource.downcast_ref().unwrap();
						
						let shaders = resource_document.required_resources.iter().map(|f| response.resources.iter().find(|r| &r.url == f).unwrap().id).collect::<Vec<_>>();
	
						let shaders = shaders.iter().map(|shader| {
							let (_hash, shader, shader_type) = self.shaders.get(shader).unwrap();
	
							(shader, *shader_type)
						}).collect::<Vec<_>>();
	
						let targets = [
							render_system::AttachmentInformation {
								texture: self.albedo,
								layout: render_system::Layouts::RenderTarget,
								format: render_system::TextureFormats::RGBAu8,
								clear: render_system::ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
								load: false,
								store: true,
							},
							render_system::AttachmentInformation {
								texture: self.depth_target,
								layout: render_system::Layouts::RenderTarget,
								format: render_system::TextureFormats::Depth32,
								clear: render_system::ClearValue::Depth(0f32),
								load: false,
								store: true,
							},
						];
						
						match material.model.name.as_str() {
							"Visibility" => {
								match material.model.pass.as_str() {
									"MaterialEvaluation" => {
										let pipeline = render_system.create_compute_pipeline(&self.material_evaluation_pipeline_layout, (&shaders[0].0, render_system::ShaderTypes::Compute, vec![Box::new(render_system::GenericSpecializationMapEntry{ constant_id: 0, r#type: "vec4f".to_string(), value: [0f32, 1f32, 0f32, 1f32] })]));
										
										self.material_evaluation_materials.insert(resource_document.url.clone(), (self.material_evaluation_materials.len() as u32, pipeline));
									}
									_ => {
										error!("Unknown material pass: {}", material.model.pass)
									}
								}
							}
							_ => {
								error!("Unknown material model: {}", material.model.name);
							}
						}
					}

				}
				_ => {}
			}
		}
	}

	fn get_transform(&self) -> Mat4f { return Mat4f::identity(); }
	fn set_transform(&mut self, orchestrator: OrchestratorReference, value: Mat4f) {
		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<&mut render_system::RenderSystemImplementation>().unwrap();

		// let closed_frame_index = self.current_frame % 2;

		let meshes_data_slice = render_system.get_mut_buffer_slice(self.meshes_data_buffer);

		let meshes_data = [
			value,
		];

		let meshes_data_bytes = unsafe { std::slice::from_raw_parts(meshes_data.as_ptr() as *const u8, std::mem::size_of_val(&meshes_data)) };

		unsafe {
			std::ptr::copy_nonoverlapping(meshes_data_bytes.as_ptr(), meshes_data_slice.as_mut_ptr().add(0 as usize * std::mem::size_of::<maths_rs::Mat4f>()), meshes_data_bytes.len());
		}
	}

	/// Return the property for the transform of a mesh
	pub const fn transform() -> orchestrator::Property<(), Self, Mat4f> { orchestrator::Property::Component { getter: Self::get_transform, setter: Self::set_transform } }

	pub fn render(&mut self, orchestrator: OrchestratorReference) {
		if self.swapchain_handles.len() == 0 { return; }

		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut binding = render_system.get_mut();
  		let render_system = binding.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		let camera_handle = if let Some(camera_handle) = &self.camera { camera_handle } else { return; };

		{
			let mut command_buffer_recording = render_system.create_command_buffer_recording(self.transfer_command_buffer, None);

			command_buffer_recording.transfer_textures(&self.pending_texture_loads);

			self.pending_texture_loads.clear();

			command_buffer_recording.execute(&[], &[self.transfer_synchronizer], self.transfer_synchronizer);
		}

		render_system.wait(self.render_finished_synchronizer);

		//render_system.start_frame_capture();

		let camera_data_buffer = render_system.get_mut_buffer_slice(self.camera_data_buffer_handle);

		let camera_position = orchestrator.get_property(camera_handle, camera::Camera::position);
		let camera_orientation = orchestrator.get_property(camera_handle, camera::Camera::orientation);

		let view_matrix = maths_rs::Mat4f::from_translation(-camera_position) * math::look_at(camera_orientation);

		let projection_matrix = math::projection_matrix(45f32, 16f32 / 9f32, 0.1f32, 100f32);

		let view_projection_matrix = projection_matrix * view_matrix;

		let camera_data = [
			view_matrix,
			projection_matrix,
			view_projection_matrix,
		];

		let camera_data_bytes = unsafe { std::slice::from_raw_parts(camera_data.as_ptr() as *const u8, std::mem::size_of_val(&camera_data)) };

		unsafe {
			std::ptr::copy_nonoverlapping(camera_data_bytes.as_ptr(), camera_data_buffer.as_mut_ptr(), camera_data_bytes.len());
		}

		let swapchain_handle = self.swapchain_handles[0];

		let image_index = render_system.acquire_swapchain_image(swapchain_handle, self.image_ready);

		let mut command_buffer_recording = render_system.create_command_buffer_recording(self.render_command_buffer, Some(self.current_frame as u32));

		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.vertex_positions_buffer),
				stages: render_system::Stages::MESH,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.vertex_normals_buffer),
				stages: render_system::Stages::MESH,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},render_system::Consumption{
				handle: render_system::Handle::Buffer(self.indices_buffer),
				stages: render_system::Stages::MESH,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
		]);

		let attachments = [
			render_system::AttachmentInformation {
				texture: self.material_id,
				layout: render_system::Layouts::RenderTarget,
				format: render_system::TextureFormats::U32,
				clear: render_system::ClearValue::Integer(!0u32, 0, 0, 0),
				load: false,
				store: true,
			},
			render_system::AttachmentInformation {
				texture: self.primitive_index,
				layout: render_system::Layouts::RenderTarget,
				format: render_system::TextureFormats::U32,
				clear: render_system::ClearValue::Integer(!0u32, 0, 0, 0),
				load: false,
				store: true,
			},
			render_system::AttachmentInformation {
				texture: self.instance_id,
				layout: render_system::Layouts::RenderTarget,
				format: render_system::TextureFormats::U32,
				clear: render_system::ClearValue::Integer(!0u32, 0, 0, 0),
				load: false,
				store: true,
			},
			render_system::AttachmentInformation {
				texture: self.depth_target,
				layout: render_system::Layouts::RenderTarget,
				format: render_system::TextureFormats::Depth32,
				clear: render_system::ClearValue::Depth(1f32),
				load: false,
				store: true,
			},
		];

		command_buffer_recording.start_render_pass(Extent::new(1920, 1080, 1), &attachments);

		// Visibility pass

		command_buffer_recording.bind_pipeline(&self.visibility_pass_pipeline);

		// let vertex_buffer_descriptors = [
		// 	render_system::BufferDescriptor {
		// 		buffer: self.vertex_positions_buffer,
		// 		offset: 0,
		// 		range: (24 * std::mem::size_of::<Vector3>() as u32) as u64,
		// 		slot: 0,
		// 	},
		// 	render_system::BufferDescriptor {
		// 		buffer: self.vertex_normals_buffer,
		// 		offset: 0,
		// 		range: (24 * std::mem::size_of::<Vector3>() as u32) as u64,
		// 		slot: 1,
		// 	},
		// ];

		// command_buffer_recording.bind_vertex_buffers(&vertex_buffer_descriptors);

		// let index_buffer_index_descriptor = render_system::BufferDescriptor {
		// 	buffer: self.indices_buffer,
		// 	offset: 0,
		// 	range: (self.index_count * std::mem::size_of::<u16>() as u32) as u64,
		// 	slot: 0,
		// };

		// command_buffer_recording.bind_index_buffer(&index_buffer_index_descriptor);

		let camera_data_buffer_address = render_system.get_buffer_address(self.camera_data_buffer_handle);
		let meshes_data_buffer_address = render_system.get_buffer_address(self.meshes_data_buffer);

		let data = [
			camera_data_buffer_address,
			meshes_data_buffer_address,
		];

		command_buffer_recording.bind_descriptor_set(&self.pipeline_layout_handle, 0, &self.descriptor_set);

		command_buffer_recording.write_to_push_constant(&self.pipeline_layout_handle, 0, unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(&data)) });

		// command_buffer_recording.draw_indexed(self.index_count, self.instance_count, 0, 0, 0);
		// command_buffer_recording.dispatch_meshes(self.index_count / 3, 1, 1);
		command_buffer_recording.dispatch_meshes(1, 1, 1);

		command_buffer_recording.end_render_pass();

		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_count),
				stages: render_system::Stages::TRANSFER,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::Transfer,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_offset),
				stages: render_system::Stages::TRANSFER,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::Transfer,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_offset_scratch),
				stages: render_system::Stages::TRANSFER,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::Transfer,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_evaluation_dispatches),
				stages: render_system::Stages::TRANSFER,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::Transfer,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_xy),
				stages: render_system::Stages::TRANSFER,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::Transfer,
			},
		]);

		command_buffer_recording.clear_buffer(self.material_count);
		command_buffer_recording.clear_buffer(self.material_offset);
		command_buffer_recording.clear_buffer(self.material_offset_scratch);
		command_buffer_recording.clear_buffer(self.material_evaluation_dispatches);
		command_buffer_recording.clear_buffer(self.material_xy);

		// Material count pass

		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Texture(self.material_id),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_count),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ | render_system::AccessPolicies::WRITE, // Atomic operations are read/write
				layout: render_system::Layouts::General,
			},
		]);

		command_buffer_recording.bind_compute_pipeline(&self.material_count_pipeline);
		command_buffer_recording.bind_descriptor_set(&self.visibility_pass_pipeline_layout, 0, &self.visibility_passes_descriptor_set);
		command_buffer_recording.dispatch(1920u32.div_ceil(32), 1080u32.div_ceil(32), 1);

		// Material offset pass

		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_count),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_offset),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_offset_scratch),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);
		command_buffer_recording.bind_compute_pipeline(&self.material_offset_pipeline);
		command_buffer_recording.bind_descriptor_set(&self.visibility_pass_pipeline_layout, 0, &self.visibility_passes_descriptor_set);
		command_buffer_recording.dispatch(1, 1, 1);

		// Pixel mapping pass

		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_offset),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_offset_scratch),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ | render_system::AccessPolicies::WRITE, // Atomic operations are read/write
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_xy),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);
		command_buffer_recording.bind_compute_pipeline(&self.pixel_mapping_pipeline);
		command_buffer_recording.bind_descriptor_set(&self.visibility_pass_pipeline_layout, 0, &self.visibility_passes_descriptor_set);
		command_buffer_recording.dispatch(1920u32.div_ceil(32), 1080u32.div_ceil(32), 1);

		// Material evaluation pass
		
		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Texture(self.albedo),
				stages: render_system::Stages::TRANSFER,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::Transfer,
			},
			render_system::Consumption{
				handle: render_system::Handle::Texture(self.result),
				stages: render_system::Stages::TRANSFER,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::Transfer,
			},
		]);

		command_buffer_recording.clear_texture(self.albedo, render_system::ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }));
		command_buffer_recording.clear_texture(self.result, render_system::ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }));

		command_buffer_recording.consume_resources(&[
			render_system::Consumption {
				handle: render_system::Handle::Texture(self.albedo),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption {
				handle: render_system::Handle::Texture(self.primitive_index),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption {
				handle: render_system::Handle::Texture(self.instance_id),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_evaluation_dispatches),
				stages: render_system::Stages::INDIRECT,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_xy),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Texture(self.debug_position),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Texture(self.debug_normal),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);

		command_buffer_recording.bind_descriptor_set(&self.material_evaluation_pipeline_layout, 0, &self.visibility_passes_descriptor_set);
		command_buffer_recording.bind_descriptor_set(&self.material_evaluation_pipeline_layout, 1, &self.material_evaluation_descriptor_set);

		for (_, (i, pipeline)) in self.material_evaluation_materials.iter() {
			// No need for sync here, as each thread across all invocations will write to a different pixel
			command_buffer_recording.bind_compute_pipeline(pipeline);
			command_buffer_recording.write_to_push_constant(&self.material_evaluation_pipeline_layout, 0, unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(&data)) });
			command_buffer_recording.write_to_push_constant(&self.material_evaluation_pipeline_layout, 16, unsafe {
				std::slice::from_raw_parts(&(*i as u32) as *const u32 as *const u8, std::mem::size_of::<u32>())
			});
			command_buffer_recording.write_to_push_constant(&self.material_evaluation_pipeline_layout, 20, unsafe {
				std::slice::from_raw_parts(&(*i as u64) as *const u64 as *const u8, std::mem::size_of::<u64>())
			});
			command_buffer_recording.indirect_dispatch(&render_system::BufferDescriptor { buffer: self.material_evaluation_dispatches, offset: (*i as u64 * 12), range: 12, slot: 0 });
		}

		// Tone mapping pass

		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Texture(self.albedo),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Texture(self.result),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);

		command_buffer_recording.bind_compute_pipeline(&self.tone_map_pass.pipeline);
		command_buffer_recording.bind_descriptor_set(&self.tone_map_pass.pipeline_layout, 0, &self.tone_map_pass.descriptor_set);
		command_buffer_recording.dispatch(1920u32.div_ceil(32), 1080u32.div_ceil(32), 1);

		// Copy to swapchain

		command_buffer_recording.copy_to_swapchain(self.result, image_index, swapchain_handle);

		command_buffer_recording.execute(&[self.transfer_synchronizer, self.image_ready], &[self.render_finished_synchronizer], self.render_finished_synchronizer);

		//render_system.end_frame_capture();

		render_system.present(image_index, &[swapchain_handle], self.render_finished_synchronizer);

		render_system.wait(self.transfer_synchronizer); // Wait for buffers to be copied over to the GPU, or else we might overwrite them on the CPU before they are copied over

		self.current_frame += 1;
	}
}

impl orchestrator::EntitySubscriber<camera::Camera> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<camera::Camera>, camera: &camera::Camera) {
		self.camera = Some(handle);
	}
}

#[repr(C)]
struct ShaderMeshData {
	model: Mat4f,
	material_id: u32,
}
#[repr(C)]
struct LightingData {
	count: u32,
	lights: [LightData; 16],
}

#[repr(C)]
struct LightData {
	position: Vector3,
	color: Vector3,
}

impl orchestrator::EntitySubscriber<Mesh> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<Mesh>, mesh: &Mesh) {
		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		orchestrator.tie_self(Self::transform, &handle, Mesh::transform);

		{
			let resource_manager = orchestrator.get_by_class::<resource_manager::ResourceManager>();
			let mut resource_manager = resource_manager.get_mut();
			let resource_manager: &mut resource_manager::ResourceManager = resource_manager.downcast_mut().unwrap();

			self.load_material(resource_manager, render_system, mesh.material_id);
		}

		if !self.mesh_resources.contains_key(mesh.resource_id) { // Load only if not already loaded
			let resource_manager = orchestrator.get_by_class::<resource_manager::ResourceManager>();
			let mut resource_manager = resource_manager.get_mut();
			let resource_manager: &mut resource_manager::ResourceManager = resource_manager.downcast_mut().unwrap();

			self.load_material(resource_manager, render_system, mesh.material_id);

			let resource_request = resource_manager.request_resource(mesh.resource_id);

			let resource_request = if let Some(resource_info) = resource_request { resource_info } else { return; };

			let mut options = resource_manager::Options { resources: Vec::new(), };

			for resource in &resource_request.resources {
				match resource.class.as_str() {
					"Mesh" => {
						let vertex_positions_buffer = render_system.get_mut_buffer_slice(self.vertex_positions_buffer);
						let vertex_normals_buffer = render_system.get_mut_buffer_slice(self.vertex_normals_buffer);
						let index_buffer = render_system.get_mut_buffer_slice(self.indices_buffer);

						options.resources.push(resource_manager::OptionResource {
							url: resource.url.clone(),
							buffers: vec![
								resource_manager::Buffer{ buffer: vertex_positions_buffer, tag: "Vertex.Position".to_string() },
								resource_manager::Buffer{ buffer: vertex_normals_buffer, tag: "Vertex.Normal".to_string() },
								resource_manager::Buffer{ buffer: index_buffer, tag: "Index".to_string() }
							],
						});
					}
					_ => {}
				}
			}

			let resource = if let Ok(a) = resource_manager.load_resource(resource_request, Some(options), None) { a } else { return; };

			let (response, _buffer) = (resource.0, resource.1.unwrap());

			for resource in &response.resources {
				match resource.class.as_str() {
					"Mesh" => {
						self.mesh_resources.insert(mesh.resource_id, self.index_count);

						let mesh: &mesh_resource_handler::Mesh = resource.resource.downcast_ref().unwrap();

						self.index_count += mesh.index_count;
					}
					_ => {}
				}
			}
		}

		let meshes_data_slice = render_system.get_mut_buffer_slice(self.meshes_data_buffer);

		let mesh_data = ShaderMeshData {
			model: mesh.transform,
			material_id: self.material_evaluation_materials.get(mesh.material_id).unwrap().0,
		};

		let meshes_data_slice = unsafe { std::slice::from_raw_parts_mut(meshes_data_slice.as_mut_ptr() as *mut ShaderMeshData, 16) };

		meshes_data_slice[self.instance_count as usize] = mesh_data;

		self.meshes.insert(handle, self.instance_count);

		self.instance_count += 1;
	}
}

impl orchestrator::EntitySubscriber<window_system::Window> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<window_system::Window>, window: &window_system::Window) {
		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		let window_system = orchestrator.get_by_class::<window_system::WindowSystem>();
		let mut window_system = window_system.get_mut();
		let window_system = window_system.downcast_mut::<window_system::WindowSystem>().unwrap();

		let swapchain_handle = render_system.bind_to_window(&window_system.get_os_handles(&handle));

		self.swapchain_handles.push(swapchain_handle);
	}
}

impl orchestrator::EntitySubscriber<DirectionalLight> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<DirectionalLight>, light: &DirectionalLight) {
		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		let lighting_data = unsafe { (render_system.get_mut_buffer_slice(self.light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };

		let light_index = lighting_data.count as usize;

		lighting_data.lights[light_index].position = crate::Vec3f::new(0.0, 2.0, -1.5);
		lighting_data.lights[light_index].color = light.color;
		
		lighting_data.count += 1;
	}
}

impl orchestrator::EntitySubscriber<PointLight> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<PointLight>, light: &PointLight) {
		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		let lighting_data = unsafe { (render_system.get_mut_buffer_slice(self.light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };

		let light_index = lighting_data.count as usize;

		lighting_data.lights[light_index].position = light.position;
		lighting_data.lights[light_index].color = light.color;
		
		lighting_data.count += 1;
	}
}

impl Entity for VisibilityWorldRenderDomain {}
impl System for VisibilityWorldRenderDomain {}

use crate::orchestrator::{Component, EntityHandle};

#[derive(component_derive::Component)]
pub struct Mesh{
	pub resource_id: &'static str,
	pub material_id: &'static str,
	#[field] pub transform: maths_rs::Mat4f,
}

pub struct MeshParameters {
	pub resource_id: &'static str,
	pub transform: maths_rs::Mat4f,
}

impl Entity for Mesh {}

impl Mesh {
	fn set_transform(&mut self, _orchestrator: orchestrator::OrchestratorReference, value: maths_rs::Mat4f) { self.transform = value; }

	fn get_transform(&self) -> maths_rs::Mat4f { self.transform }

	pub const fn transform() -> orchestrator::Property<(), Self, maths_rs::Mat4f> { orchestrator::Property::Component { getter: Mesh::get_transform, setter: Mesh::set_transform } }
}

impl Component for Mesh {
	// type Parameters<'a> = MeshParameters;
}