use std::collections::HashMap;

use log::error;
use maths_rs::{prelude::MatTranslate, Mat4f};

use crate::{resource_manager::{self, mesh_resource_handler, material_resource_handler::{Shader, Material, Variant}, texture_resource_handler}, rendering::{render_system::{RenderSystem, self}, directional_light::DirectionalLight, point_light::PointLight}, Extent, orchestrator::{Entity, System, self, OrchestratorReference}, Vector3, camera::{self}, math, window_system};

struct VisibilityInfo {
	instance_count: u32,
	triangle_count: u32,
	meshlet_count: u32,
	vertex_count: u32,
}

struct ToneMapPass {
	pipeline_layout: render_system::PipelineLayoutHandle,
	pipeline: render_system::PipelineHandle,
	descriptor_set_layout: render_system::DescriptorSetLayoutHandle,
	descriptor_set: render_system::DescriptorSetHandle,
}

struct MeshData {
	meshlets: Vec<ShaderMeshletData>,
	vertex_count: u32,
	triangle_count: u32,
}

/// This the visibility buffer implementation of the world render domain.
pub struct VisibilityWorldRenderDomain {
	pipeline_layout_handle: render_system::PipelineLayoutHandle,

	vertex_positions_buffer: render_system::BaseBufferHandle,
	vertex_normals_buffer: render_system::BaseBufferHandle,

	indices_buffer: render_system::BaseBufferHandle,

	albedo: render_system::ImageHandle,
	depth_target: render_system::ImageHandle,
	result: render_system::ImageHandle,

	visiblity_info: VisibilityInfo,

	render_finished_synchronizer: render_system::SynchronizerHandle,
	image_ready: render_system::SynchronizerHandle,
	render_command_buffer: render_system::CommandBufferHandle,
	current_frame: usize,

	camera_data_buffer_handle: render_system::BaseBufferHandle,
	materials_data_buffer_handle: render_system::BaseBufferHandle,

	descriptor_set_layout: render_system::DescriptorSetLayoutHandle,
	descriptor_set: render_system::DescriptorSetHandle,

	transfer_synchronizer: render_system::SynchronizerHandle,
	transfer_command_buffer: render_system::CommandBufferHandle,

	meshes_data_buffer: render_system::BaseBufferHandle,
	meshlets_data_buffer: render_system::BaseBufferHandle,

	camera: Option<EntityHandle<crate::camera::Camera>>,

	meshes: HashMap<String, MeshData>,

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

	instance_id: render_system::ImageHandle,
	primitive_index: render_system::ImageHandle,

	material_count: render_system::BaseBufferHandle,
	material_offset: render_system::BaseBufferHandle,
	material_offset_scratch: render_system::BaseBufferHandle,
	material_evaluation_dispatches: render_system::BaseBufferHandle,
	material_xy: render_system::BaseBufferHandle,

	material_evaluation_descriptor_set_layout: render_system::DescriptorSetLayoutHandle,
	material_evaluation_descriptor_set: render_system::DescriptorSetHandle,
	material_evaluation_pipeline_layout: render_system::PipelineLayoutHandle,

	material_evaluation_materials: HashMap<String, (u32, render_system::PipelineHandle)>,

	tone_map_pass: ToneMapPass,
	debug_position: render_system::ImageHandle,
	debug_normal: render_system::ImageHandle,
	light_data_buffer: render_system::BaseBufferHandle,

	pending_texture_loads: Vec<render_system::ImageHandle>,

	top_level_acceleration_structure: render_system::AccelerationStructureHandle,
}

const VERTEX_COUNT: u32 = 64;
const TRIANGLE_COUNT: u32 = 126;

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
					stages: render_system::Stages::MESH | render_system::Stages::FRAGMENT | render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MeshData",
					binding: 1,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::MESH | render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "vertex positions",
					binding: 2,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::MESH | render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "vertex normals",
					binding: 3,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::MESH | render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "indices",
					binding: 4,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::MESH | render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "textures",
					binding: 5,
					descriptor_type: render_system::DescriptorType::CombinedImageSampler,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "meshlet data",
					binding: 6,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::MESH | render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
			];

			let descriptor_set_layout = render_system.create_descriptor_set_layout(Some("Base Set Layout"), &bindings);

			let descriptor_set = render_system.create_descriptor_set(Some("Base Descriptor Set"), &descriptor_set_layout, &bindings);

			let pipeline_layout_handle = render_system.create_pipeline_layout(&[descriptor_set_layout], &[]);
			
			let vertex_positions_buffer_handle = render_system.create_buffer(Some("Visibility Vertex Positions Buffer"), 1024 * 1024 * 16, render_system::Uses::Vertex | render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let vertex_normals_buffer_handle = render_system.create_buffer(Some("Visibility Vertex Normals Buffer"), 1024 * 1024 * 16, render_system::Uses::Vertex | render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			let indices_buffer_handle = render_system.create_buffer(Some("Visibility Index Buffer"), 1024 * 1024 * 16, render_system::Uses::Index | render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			let debug_position = render_system.create_image(Some("debug position"), Extent::new(1920, 1080, 1), render_system::Formats::RGBAu16, None, render_system::Uses::RenderTarget | render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
			let debug_normals = render_system.create_image(Some("debug normal"), Extent::new(1920, 1080, 1), render_system::Formats::RGBAu16, None, render_system::Uses::RenderTarget | render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let albedo = render_system.create_image(Some("albedo"), Extent::new(1920, 1080, 1), render_system::Formats::RGBAu16, None, render_system::Uses::RenderTarget | render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
			let depth_target = render_system.create_image(Some("depth_target"), Extent::new(1920, 1080, 1), render_system::Formats::Depth32, None, render_system::Uses::DepthStencil, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let render_finished_synchronizer = render_system.create_synchronizer(true);
			let image_ready = render_system.create_synchronizer(false);

			let transfer_synchronizer = render_system.create_synchronizer(false);

			let render_command_buffer = render_system.create_command_buffer();
			let transfer_command_buffer = render_system.create_command_buffer();

			let camera_data_buffer_handle = render_system.create_buffer(Some("Visibility Camera Data"), 16 * 4 * 4, render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let meshes_data_buffer = render_system.create_buffer(Some("Visibility Meshes Data"), std::mem::size_of::<ShaderInstanceData>() * 1024, render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let meshlets_data_buffer = render_system.create_buffer(Some("Visibility Meshlets Data"), std::mem::size_of::<ShaderMeshletData>() * 1024, render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			render_system.write(&[
				render_system::DescriptorWrite {
					descriptor_set,
					binding: 0,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: camera_data_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite {
					descriptor_set,
					binding: 1,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: meshes_data_buffer, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite {
					descriptor_set,
					binding: 2,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: vertex_positions_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite {
					descriptor_set,
					binding: 3,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: vertex_normals_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite {
					descriptor_set,
					binding: 4,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: indices_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite {
					descriptor_set,
					binding: 6,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer { handle: meshlets_data_buffer, size: render_system::Ranges::Whole },
				},
			]);

			let visibility_pass_mesh_source = format!("
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

layout(location=0) perprimitiveEXT out uint out_instance_index[{TRIANGLE_COUNT}];
layout(location=1) perprimitiveEXT out uint out_primitive_index[{TRIANGLE_COUNT}];

struct Camera {{
	mat4 view_matrix;
	mat4 projection_matrix;
	mat4 view_projection;
}};

struct Mesh {{
	mat4 model;
	uint material_id;
}};

struct Meshlet {{
	uint32_t instance_index;
	uint16_t vertex_offset;
	uint16_t triangle_offset;
	uint8_t vertex_count;
	uint8_t triangle_count;
	uint8_t padding[6];
}};

layout(set=0,binding=0,scalar) buffer readonly CameraBuffer {{
	Camera camera;
}};

layout(set=0,binding=1,scalar) buffer readonly MeshesBuffer {{
	Mesh meshes[];
}};

layout(set=0,binding=2,scalar) buffer readonly MeshVertexPositions {{
	vec3 vertex_positions[];
}};

layout(set=0,binding=4,scalar) buffer readonly MeshIndices {{
	uint16_t indices[];
}};

layout(set=0,binding=6,scalar) buffer readonly MeshletsBuffer {{
	Meshlet meshlets[];
}};

layout(triangles, max_vertices={VERTEX_COUNT}, max_primitives={TRIANGLE_COUNT}) out;
layout(local_size_x=128) in;
void main() {{
	uint meshlet_index = gl_WorkGroupID.x;

	Meshlet meshlet = meshlets[meshlet_index];

	uint instance_index = meshlet.instance_index;

	SetMeshOutputsEXT(meshlet.vertex_count, meshlet.triangle_count);

	if (gl_LocalInvocationID.x < uint(meshlet.vertex_count) && gl_LocalInvocationID.x < {VERTEX_COUNT}) {{
		uint vertex_index = uint(meshlet.vertex_offset) + gl_LocalInvocationID.x;
		gl_MeshVerticesEXT[gl_LocalInvocationID.x].gl_Position = camera.view_projection * meshes[instance_index].model * vec4(vertex_positions[vertex_index], 1.0);
		// gl_MeshVerticesEXT[gl_LocalInvocationID.x].gl_Position = vec4(vertex_positions[vertex_index], 1.0);
	}}
	
	if (gl_LocalInvocationID.x < uint(meshlet.triangle_count) && gl_LocalInvocationID.x < {TRIANGLE_COUNT}) {{
		uint triangle_index = uint(meshlet.triangle_offset) + gl_LocalInvocationID.x;
		uint triangle_indices[3] = uint[](indices[triangle_index * 3 + 0], indices[triangle_index * 3 + 1], indices[triangle_index * 3 + 2]);
		gl_PrimitiveTriangleIndicesEXT[gl_LocalInvocationID.x] = uvec3(triangle_indices[0], triangle_indices[1], triangle_indices[2]);
		out_instance_index[gl_LocalInvocationID.x] = instance_index;
		out_primitive_index[gl_LocalInvocationID.x] = (meshlet_index << 8) | (gl_LocalInvocationID.x & 0xFF);
	}}
}}", TRIANGLE_COUNT=TRIANGLE_COUNT, VERTEX_COUNT=VERTEX_COUNT);

			let visibility_pass_fragment_source = r#"
				#version 450
				#pragma shader_stage(fragment)

				#extension GL_EXT_scalar_block_layout: enable
				#extension GL_EXT_buffer_reference: enable
				#extension GL_EXT_buffer_reference2: enable
				#extension GL_EXT_mesh_shader: require

				layout(location=0) perprimitiveEXT flat in uint in_instance_index;
				layout(location=1) perprimitiveEXT flat in uint in_primitive_index;

				layout(location=0) out uint out_primitive_index;
				layout(location=1) out uint out_instance_id;

				void main() {
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

			let primitive_index = render_system.create_image(Some("primitive index"), crate::Extent::new(1920, 1080, 1), render_system::Formats::U32, None, render_system::Uses::RenderTarget | render_system::Uses::Storage, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
			let instance_id = render_system.create_image(Some("instance_id"), crate::Extent::new(1920, 1080, 1), render_system::Formats::U32, None, render_system::Uses::RenderTarget | render_system::Uses::Storage, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let attachments = [
				render_system::AttachmentInformation {
					image: primitive_index,
					layout: render_system::Layouts::RenderTarget,
					format: render_system::Formats::U32,
					clear: render_system::ClearValue::Integer(!0u32, 0, 0, 0),
					load: false,
					store: true,
				},
				render_system::AttachmentInformation {
					image: instance_id,
					layout: render_system::Layouts::RenderTarget,
					format: render_system::Formats::U32,
					clear: render_system::ClearValue::Integer(!0u32, 0, 0, 0),
					load: false,
					store: true,
				},
				render_system::AttachmentInformation {
					image: depth_target,
					layout: render_system::Layouts::RenderTarget,
					format: render_system::Formats::Depth32,
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

				struct Mesh {
					mat4 model;
					uint material_index;
				};

				layout(set=0,binding=1,scalar) buffer MeshesBuffer {
					Mesh meshes[];
				};

				layout(set=1,binding=0,scalar) buffer MaterialCount {
					uint material_count[];
				};

				layout(set=1, binding=7, r32ui) uniform readonly uimage2D instance_index;

				layout(local_size_x=32, local_size_y=32) in;
				void main() {
					// If thread is out of bound respect to the material_id texture, return
					if (gl_GlobalInvocationID.x >= imageSize(instance_index).x || gl_GlobalInvocationID.y >= imageSize(instance_index).y) { return; }

					uint pixel_instance_index = imageLoad(instance_index, ivec2(gl_GlobalInvocationID.xy)).r;

					if (pixel_instance_index == 0xFFFFFFFF) { return; }

					uint material_index = meshes[pixel_instance_index].material_index;

					atomicAdd(material_count[material_index], 1);
				}
			"#;

			let material_offset_source = r#"
				#version 450
				#pragma shader_stage(compute)

				#extension GL_EXT_scalar_block_layout: enable
				#extension GL_EXT_buffer_reference2: enable

				layout(set=1,binding=0,scalar) buffer MaterialCount {
					uint material_count[];
				};

				layout(set=1,binding=1,scalar) buffer MaterialOffset {
					uint material_offset[];
				};

				layout(set=1,binding=2,scalar) buffer MaterialOffsetScratch {
					uint material_offset_scratch[];
				};

				layout(set=1,binding=3,scalar) buffer MaterialEvaluationDispatches {
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

				struct Mesh {
					mat4 model;
					uint material_index;
				};

				layout(set=0,binding=1,scalar) buffer MeshesBuffer {
					Mesh meshes[];
				};

				layout(set=1,binding=1,scalar) buffer MaterialOffset {
					uint material_offset[];
				};

				layout(set=1,binding=2,scalar) buffer MaterialOffsetScratch {
					uint material_offset_scratch[];
				};

				layout(set=1,binding=4,scalar) buffer PixelMapping {
					u16vec2 pixel_mapping[];
				};

				layout(set=1, binding=7, r32ui) uniform readonly uimage2D instance_index;

				layout(local_size_x=32, local_size_y=32) in;
				void main() {
					// If thread is out of bound respect to the material_id texture, return
					if (gl_GlobalInvocationID.x >= imageSize(instance_index).x || gl_GlobalInvocationID.y >= imageSize(instance_index).y) { return; }

					uint pixel_instance_index = imageLoad(instance_index, ivec2(gl_GlobalInvocationID.xy)).r;

					if (pixel_instance_index == 0xFFFFFFFF) { return; }

					uint material_index = meshes[pixel_instance_index].material_index;

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
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialOffset",
					binding: 1,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialOffsetScratch",
					binding: 2,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialEvaluationDispatches",
					binding: 3,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialXY",
					binding: 4,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialId",
					binding: 5,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "VertexId",
					binding: 6,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "InstanceId",
					binding: 7,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
			];

			let visibility_descriptor_set_layout = render_system.create_descriptor_set_layout(Some("Visibility Set Layout"), &bindings);
			let visibility_pass_pipeline_layout = render_system.create_pipeline_layout(&[descriptor_set_layout, visibility_descriptor_set_layout], &[]);
			let visibility_passes_descriptor_set = render_system.create_descriptor_set(Some("Visibility Descriptor Set"), &visibility_descriptor_set_layout, &bindings);

			render_system.write(&[
				render_system::DescriptorWrite { // MaterialCount
					descriptor_set: visibility_passes_descriptor_set,
					binding: 0,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_count, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // MaterialOffset
					descriptor_set: visibility_passes_descriptor_set,
					binding: 1,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_offset, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // MaterialOffsetScratch
					descriptor_set: visibility_passes_descriptor_set,
					binding: 2,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_offset_scratch, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // MaterialEvaluationDispatches
					descriptor_set: visibility_passes_descriptor_set,
					binding: 3,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_evaluation_dispatches, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // MaterialXY
					descriptor_set: visibility_passes_descriptor_set,
					binding: 4,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_xy, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // Primitive Index
					descriptor_set: visibility_passes_descriptor_set,
					binding: 6,
					array_element: 0,
					descriptor: render_system::Descriptor::Image{ handle: primitive_index, layout: render_system::Layouts::General },
				},
				render_system::DescriptorWrite { // InstanceId
					descriptor_set: visibility_passes_descriptor_set,
					binding: 7,
					array_element: 0,
					descriptor: render_system::Descriptor::Image{ handle: instance_id, layout: render_system::Layouts::General },
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
			
			let materials_data_buffer_handle = render_system.create_buffer(Some("Materials Data"), 1024 * 4 * 4, render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let bindings = [
				render_system::DescriptorSetLayoutBinding {
					name: "albedo",
					binding: 0,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MeshBuffer",
					binding: 1,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "Positions",
					binding: 2,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "Normals",
					binding: 3,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "Indeces",
					binding: 4,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "CameraData",
					binding: 5,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MeshData",
					binding: 6,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "debug_position",
					binding: 7,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "debug_normals",
					binding: 8,
					descriptor_type: render_system::DescriptorType::StorageImage,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "LightData",
					binding: 9,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
				render_system::DescriptorSetLayoutBinding {
					name: "MaterialsData",
					binding: 10,
					descriptor_type: render_system::DescriptorType::StorageBuffer,
					descriptor_count: 1,
					stages: render_system::Stages::COMPUTE,
					immutable_samplers: None,
				},
			];	

			let material_evaluation_descriptor_set_layout = render_system.create_descriptor_set_layout(Some("Material Evaluation Set Layout"), &bindings);
			let material_evaluation_descriptor_set = render_system.create_descriptor_set(Some("Material Evaluation Descriptor Set"), &material_evaluation_descriptor_set_layout, &bindings);

			render_system.write(&[
				render_system::DescriptorWrite { // albedo
					descriptor_set: material_evaluation_descriptor_set,
					binding: 0,
					array_element: 0,
					descriptor: render_system::Descriptor::Image{ handle: albedo, layout: render_system::Layouts::General },
				},
				render_system::DescriptorWrite { // MeshBuffer
					descriptor_set: material_evaluation_descriptor_set,
					binding: 1,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: meshes_data_buffer, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // Positions
					descriptor_set: material_evaluation_descriptor_set,
					binding: 2,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: vertex_positions_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // Normals
					descriptor_set: material_evaluation_descriptor_set,
					binding: 3,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: vertex_normals_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // Indeces
					descriptor_set: material_evaluation_descriptor_set,
					binding: 4,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: indices_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // CameraData
					descriptor_set: material_evaluation_descriptor_set,
					binding: 5,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: camera_data_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // MeshData
					descriptor_set: material_evaluation_descriptor_set,
					binding: 6,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: meshes_data_buffer, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // debug_position
					descriptor_set: material_evaluation_descriptor_set,
					binding: 7,
					array_element: 0,
					descriptor: render_system::Descriptor::Image{ handle: debug_position, layout: render_system::Layouts::General }
				},
				render_system::DescriptorWrite { // debug_normals
					descriptor_set: material_evaluation_descriptor_set,
					binding: 8,
					array_element: 0,
					descriptor: render_system::Descriptor::Image{ handle: debug_normals, layout: render_system::Layouts::General }
				},
				render_system::DescriptorWrite { // LightData
					descriptor_set: material_evaluation_descriptor_set,
					binding: 9,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: light_data_buffer, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // MaterialsData
					descriptor_set: material_evaluation_descriptor_set,
					binding: 10,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: materials_data_buffer_handle, size: render_system::Ranges::Whole },
				},
			]);

			let material_evaluation_pipeline_layout = render_system.create_pipeline_layout(&[descriptor_set_layout, visibility_descriptor_set_layout, material_evaluation_descriptor_set_layout], &[render_system::PushConstantRange{ offset: 0, size: 4 }]);

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

			let result = render_system.create_image(Some("result"), Extent::new(1920, 1080, 1), render_system::Formats::RGBAu8, None, render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let tone_map_pass = {
				let descriptor_set_layout = render_system.create_descriptor_set_layout(Some("Tonemap Pass Set Layout"), &[
					render_system::DescriptorSetLayoutBinding {
						name: "source",
						binding: 0,
						descriptor_type: render_system::DescriptorType::StorageImage,
						descriptor_count: 1,
						stages: render_system::Stages::COMPUTE,
						immutable_samplers: None,
					},
					render_system::DescriptorSetLayoutBinding {
						name: "result",
						binding: 1,
						descriptor_type: render_system::DescriptorType::StorageImage,
						descriptor_count: 1,
						stages: render_system::Stages::COMPUTE,
						immutable_samplers: None,
					},
				]);

				let pipeline_layout = render_system.create_pipeline_layout(&[descriptor_set_layout], &[]);

				let descriptor_set = render_system.create_descriptor_set(Some("Tonemap Pass Descriptor Set"), &descriptor_set_layout, &[
					render_system::DescriptorSetLayoutBinding {
						name: "source",
						binding: 0,
						descriptor_type: render_system::DescriptorType::StorageImage,
						descriptor_count: 1,
						stages: render_system::Stages::COMPUTE,
						immutable_samplers: None,
					},
					render_system::DescriptorSetLayoutBinding {
						name: "result",
						binding: 1,
						descriptor_type: render_system::DescriptorType::StorageImage,
						descriptor_count: 1,
						stages: render_system::Stages::COMPUTE,
						immutable_samplers: None,
					},
				]);

				render_system.write(&[
					render_system::DescriptorWrite {
						descriptor_set,
						binding: 0,
						array_element: 0,
						descriptor: render_system::Descriptor::Image{ handle: albedo, layout: render_system::Layouts::General },
					},
					render_system::DescriptorWrite {
						descriptor_set,
						binding: 1,
						array_element: 0,
						descriptor: render_system::Descriptor::Image{ handle: result, layout: render_system::Layouts::General },
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

			let _instance_buffer = render_system.create_acceleration_structure_instance_buffer(Some("Scene Instance Buffer"), 16);
			
			let buffer = render_system.create_buffer(None, 65565, render_system::Uses::AccelerationStructure, render_system::DeviceAccesses::GpuWrite, render_system::UseCases::STATIC);
			let top_level_acceleration_structure = render_system.create_acceleration_structure(Some("Top Level Acceleration Structure"), render_system::AccelerationStructureTypes::TopLevel{ instance_count: 16 }, render_system::BufferDescriptor { buffer: buffer, offset: 0, range: 4096, slot: 0 });

			let rt_pass_descriptor_set_layout = render_system.create_descriptor_set_layout(Some("RT Pass Set Layout"), &[
				render_system::DescriptorSetLayoutBinding {
					name: "top level acc str",
					binding: 0,
					descriptor_type: render_system::DescriptorType::AccelerationStructure,
					descriptor_count: 1,
					stages: render_system::Stages::ACCELERATION_STRUCTURE,
					immutable_samplers: None,
				},
			]);

			let _rt_pass_pipeline_layout = render_system.create_pipeline_layout(&[descriptor_set_layout, rt_pass_descriptor_set_layout], &[]);

			let rt_pass_descriptor_set = render_system.create_descriptor_set(Some("RT Pass Descriptor Set"), &rt_pass_descriptor_set_layout, &[
				render_system::DescriptorSetLayoutBinding {
					name: "top level acc str",
					binding: 0,
					descriptor_type: render_system::DescriptorType::AccelerationStructure,
					descriptor_count: 1,
					stages: render_system::Stages::ACCELERATION_STRUCTURE,
					immutable_samplers: None,
				},
			]);

			render_system.write(&[
				render_system::DescriptorWrite {
					descriptor_set: rt_pass_descriptor_set,
					binding: 0,
					array_element: 0,
					descriptor: render_system::Descriptor::AccelerationStructure{ handle: top_level_acceleration_structure },
				},
			]);

			const _SHADOW_RAY_GEN_SHADER: &'static str = "
#version 450
#pragma shader_stage(ray_gen)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(row_major) uniform; layout(row_major) buffer;			

void main() {
	const vec2 pixelCenter = vec2(gl_LaunchIDEXT.xy) + vec2(0.5);
	const vec2 inUV = pixelCenter/vec2(gl_LaunchSizeEXT.xy);
	vec2 d = inUV * 2.0 - 1.0;
}";

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

				visiblity_info:  VisibilityInfo{ triangle_count: 0, instance_count: 0, meshlet_count:0, vertex_count:0, },

				current_frame: 0,

				render_finished_synchronizer,
				image_ready,
				render_command_buffer,

				camera_data_buffer_handle,

				transfer_synchronizer,
				transfer_command_buffer,

				meshes_data_buffer,
				meshlets_data_buffer,

				shaders: HashMap::new(),

				camera: None,

				meshes: HashMap::new(),

				light_data_buffer,
				materials_data_buffer_handle,

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

				primitive_index,
				instance_id,

				debug_position,
				debug_normal: debug_normals,

				material_count,
				material_offset,
				material_offset_scratch,
				material_evaluation_dispatches,
				material_xy,

				material_evaluation_materials: HashMap::new(),

				tone_map_pass,

				pending_texture_loads: Vec::new(),

				top_level_acceleration_structure,
			}
		})
			// .add_post_creation_function(Box::new(Self::load_needed_assets))
			.add_listener::<camera::Camera>()
			.add_listener::<Mesh>()
			.add_listener::<window_system::Window>()
			.add_listener::<DirectionalLight>()
			.add_listener::<PointLight>()
	}

	fn load_material(&mut self, resource_manager: &mut resource_manager::resource_manager::ResourceManager, render_system: &mut render_system::RenderSystemImplementation, asset_url: &str) {
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

					let new_texture = render_system.create_image(Some(&resource_document.url), texture.extent, render_system::Formats::RGBAu8, compression, render_system::Uses::Image | render_system::Uses::TransferDestination, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

					render_system.get_texture_slice_mut(new_texture).copy_from_slice(&buffer[resource_document.offset as usize..(resource_document.offset + resource_document.size) as usize]);
					
					let sampler = render_system.create_sampler(); // TODO: use actual sampler

					render_system.write(&[
						render_system::DescriptorWrite {
							descriptor_set: self.descriptor_set,
							binding: 5,
							array_element: 0,
							descriptor: render_system::Descriptor::CombinedImageSampler { image_handle: new_texture, sampler_handle: sampler, layout: render_system::Layouts::Read },
						},
					]);

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
								image: self.albedo,
								layout: render_system::Layouts::RenderTarget,
								format: render_system::Formats::RGBAu8,
								clear: render_system::ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
								load: false,
								store: true,
							},
							render_system::AttachmentInformation {
								image: self.depth_target,
								layout: render_system::Layouts::RenderTarget,
								format: render_system::Formats::Depth32,
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
						
						let materials_buffer_slice = render_system.get_mut_buffer_slice(self.materials_data_buffer_handle);

						let material_data = materials_buffer_slice.as_mut_ptr() as *mut MaterialData;

						let material_data = unsafe { material_data.as_mut().unwrap() };

						material_data.textures[0] = 0; // TODO: make dynamic based on supplied textures

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

			let consumption = self.pending_texture_loads.iter().map(|handle|{
				render_system::Consumption{
					handle: render_system::Handle::Image(*handle),
					stages: render_system::Stages::COMPUTE,
					access: render_system::AccessPolicies::READ,
					layout: render_system::Layouts::Read,
				}
			}).collect::<Vec<_>>();

			command_buffer_recording.consume_resources(&consumption);

			self.pending_texture_loads.clear();

			command_buffer_recording.execute(&[], &[self.transfer_synchronizer], self.transfer_synchronizer);
		}

		render_system.wait(self.render_finished_synchronizer);

		render_system.start_frame_capture();

		let camera_data_buffer = render_system.get_mut_buffer_slice(self.camera_data_buffer_handle);

		let camera_position = orchestrator.get_property(camera_handle, camera::Camera::position);
		let camera_orientation = orchestrator.get_property(camera_handle, camera::Camera::orientation);

		let view_matrix = maths_rs::Mat4f::from_translation(-camera_position) * math::look_at(camera_orientation);

		let projection_matrix = math::projection_matrix(45f32, 16f32 / 9f32, 0.1f32, 100f32);

		let view_projection_matrix = projection_matrix * view_matrix;

		struct ShaderCameraData {
			view_matrix: maths_rs::Mat4f,
			projection_matrix: maths_rs::Mat4f,
			view_projection_matrix: maths_rs::Mat4f,
		}

		let camera_data_reference = unsafe { (camera_data_buffer.as_mut_ptr() as *mut ShaderCameraData).as_mut().unwrap() };

		camera_data_reference.view_matrix = view_matrix;
		camera_data_reference.projection_matrix = projection_matrix;
		camera_data_reference.view_projection_matrix = view_projection_matrix;

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
				image: self.primitive_index,
				layout: render_system::Layouts::RenderTarget,
				format: render_system::Formats::U32,
				clear: render_system::ClearValue::Integer(!0u32, 0, 0, 0),
				load: false,
				store: true,
			},
			render_system::AttachmentInformation {
				image: self.instance_id,
				layout: render_system::Layouts::RenderTarget,
				format: render_system::Formats::U32,
				clear: render_system::ClearValue::Integer(!0u32, 0, 0, 0),
				load: false,
				store: true,
			},
			render_system::AttachmentInformation {
				image: self.depth_target,
				layout: render_system::Layouts::RenderTarget,
				format: render_system::Formats::Depth32,
				clear: render_system::ClearValue::Depth(1f32),
				load: false,
				store: true,
			},
		];

		command_buffer_recording.start_region("Visibility Pass");

		command_buffer_recording.consume_resources(&[
			render_system::Consumption {
				handle: render_system::Handle::Buffer(self.camera_data_buffer_handle),
				stages: render_system::Stages::MESH | render_system::Stages::FRAGMENT,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption {
				handle: render_system::Handle::Buffer(self.vertex_positions_buffer),
				stages: render_system::Stages::MESH,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption {
				handle: render_system::Handle::Buffer(self.vertex_normals_buffer),
				stages: render_system::Stages::MESH,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption {
				handle: render_system::Handle::Buffer(self.indices_buffer),
				stages: render_system::Stages::MESH,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption {
				handle: render_system::Handle::Buffer(self.meshes_data_buffer),
				stages: render_system::Stages::MESH | render_system::Stages::FRAGMENT,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption {
				handle: render_system::Handle::Buffer(self.meshlets_data_buffer),
				stages: render_system::Stages::MESH | render_system::Stages::FRAGMENT,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
		]);

		command_buffer_recording.start_render_pass(Extent::new(1920, 1080, 1), &attachments);

		// Visibility pass

		command_buffer_recording.bind_raster_pipeline(&self.visibility_pass_pipeline);

		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout_handle, &[(self.descriptor_set, 0)]);

		command_buffer_recording.dispatch_meshes(self.visiblity_info.meshlet_count, 1, 1);

		command_buffer_recording.end_render_pass();

		command_buffer_recording.end_region();

		command_buffer_recording.clear_buffers(&[self.material_count, self.material_offset, self.material_offset_scratch, self.material_evaluation_dispatches, self.material_xy]);

		// Material count pass

		command_buffer_recording.start_region("Material Count Pass");

		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Buffer(self.material_count),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ | render_system::AccessPolicies::WRITE, // Atomic operations are read/write
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Image(self.instance_id),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
		]);

		command_buffer_recording.bind_compute_pipeline(&self.material_count_pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.visibility_pass_pipeline_layout, &[(self.descriptor_set, 0), (self.visibility_passes_descriptor_set, 1)]);
		command_buffer_recording.dispatch(render_system::DispatchExtent { workgroup_extent: Extent { width: 32, height: 32, depth: 1 }, dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });

		command_buffer_recording.end_region();

		// Material offset pass

		command_buffer_recording.start_region("Material Offset Pass");

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
		command_buffer_recording.bind_descriptor_sets(&self.visibility_pass_pipeline_layout, &[(self.descriptor_set, 0), (self.visibility_passes_descriptor_set, 1)]);
		command_buffer_recording.dispatch(render_system::DispatchExtent { workgroup_extent: Extent { width: 1, height: 1, depth: 1 }, dispatch_extent: Extent { width: 1, height: 1, depth: 1 } });

		command_buffer_recording.end_region();

		// Pixel mapping pass

		command_buffer_recording.start_region("Pixel Mapping Pass");

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
		command_buffer_recording.bind_descriptor_sets(&self.visibility_pass_pipeline_layout, &[(self.descriptor_set, 0), (self.visibility_passes_descriptor_set, 1)]);
		command_buffer_recording.dispatch(render_system::DispatchExtent { workgroup_extent: Extent { width: 32, height: 32, depth: 1 }, dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });

		command_buffer_recording.end_region();

		// Material evaluation pass

		command_buffer_recording.clear_textures(&[(self.albedo, render_system::ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 })), (self.result, render_system::ClearValue::Color(crate::RGBA { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }))]);

		command_buffer_recording.start_region("Material Evaluation Pass");

		command_buffer_recording.consume_resources(&[
			render_system::Consumption {
				handle: render_system::Handle::Image(self.albedo),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption {
				handle: render_system::Handle::Image(self.primitive_index),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption {
				handle: render_system::Handle::Image(self.instance_id),
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
				handle: render_system::Handle::Image(self.debug_position),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Image(self.debug_normal),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);

		command_buffer_recording.bind_descriptor_sets(&self.material_evaluation_pipeline_layout, &[(self.descriptor_set, 0), (self.visibility_passes_descriptor_set, 1), (self.material_evaluation_descriptor_set, 2)]);

		for (_, (i, pipeline)) in self.material_evaluation_materials.iter() {
			// No need for sync here, as each thread across all invocations will write to a different pixel
			command_buffer_recording.bind_compute_pipeline(pipeline);
			command_buffer_recording.write_to_push_constant(&self.material_evaluation_pipeline_layout, 0, unsafe {
				std::slice::from_raw_parts(&(*i as u32) as *const u32 as *const u8, std::mem::size_of::<u32>())
			});
			command_buffer_recording.indirect_dispatch(&render_system::BufferDescriptor { buffer: self.material_evaluation_dispatches, offset: (*i as u64 * 12), range: 12, slot: 0 });
		}

		command_buffer_recording.end_region();

		// Tone mapping pass

		command_buffer_recording.start_region("Tone Mapping Pass");

		command_buffer_recording.consume_resources(&[
			render_system::Consumption{
				handle: render_system::Handle::Image(self.albedo),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::General,
			},
			render_system::Consumption{
				handle: render_system::Handle::Image(self.result),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::WRITE,
				layout: render_system::Layouts::General,
			},
		]);

		command_buffer_recording.bind_compute_pipeline(&self.tone_map_pass.pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.tone_map_pass.pipeline_layout, &[(self.tone_map_pass.descriptor_set, 0)]);
		command_buffer_recording.dispatch(render_system::DispatchExtent { workgroup_extent: Extent { width: 32, height: 32, depth: 1 }, dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });

		command_buffer_recording.end_region();

		// Copy to swapchain

		command_buffer_recording.copy_to_swapchain(self.result, image_index, swapchain_handle);

		command_buffer_recording.execute(&[self.transfer_synchronizer, self.image_ready], &[self.render_finished_synchronizer], self.render_finished_synchronizer);

		render_system.end_frame_capture();

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

#[derive(Copy, Clone)]
#[repr(C)]
struct ShaderMeshletData {
	instance_index: u32,
	vertex_offset: u16,
	triangle_offset: u16,
	vertex_count: u8,
	triangle_count: u8,
	pad: [u8; 6],
}

#[repr(C)]
struct ShaderInstanceData {
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

#[repr(C)]
struct MaterialData {
	textures: [u32; 16],
}

impl orchestrator::EntitySubscriber<Mesh> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<Mesh>, mesh: &Mesh) {
		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		orchestrator.tie_self(Self::transform, &handle, Mesh::transform);

		{
			let resource_manager = orchestrator.get_by_class::<resource_manager::resource_manager::ResourceManager>();
			let mut resource_manager = resource_manager.get_mut();
			let resource_manager: &mut resource_manager::resource_manager::ResourceManager = resource_manager.downcast_mut().unwrap();

			self.load_material(resource_manager, render_system, mesh.material_id);
		}

		if !self.mesh_resources.contains_key(mesh.resource_id) { // Load only if not already loaded
			let resource_manager = orchestrator.get_by_class::<resource_manager::resource_manager::ResourceManager>();
			let mut resource_manager = resource_manager.get_mut();
			let resource_manager: &mut resource_manager::resource_manager::ResourceManager = resource_manager.downcast_mut().unwrap();

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
								resource_manager::Buffer{ buffer: &mut vertex_positions_buffer[(self.visiblity_info.vertex_count as usize * std::mem::size_of::<Vector3>())..], tag: "Vertex.Position".to_string() },
								resource_manager::Buffer{ buffer: &mut vertex_normals_buffer[(self.visiblity_info.vertex_count as usize * std::mem::size_of::<Vector3>())..], tag: "Vertex.Normal".to_string() },
								resource_manager::Buffer{ buffer: &mut index_buffer[(self.visiblity_info.triangle_count as usize * 3 * std::mem::size_of::<u16>())..], tag: "MeshletIndices".to_string() }
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
						self.mesh_resources.insert(mesh.resource_id, self.visiblity_info.triangle_count);

						let mesh: &mesh_resource_handler::Mesh = resource.resource.downcast_ref().unwrap();

						{
							let vertex_offset = self.visiblity_info.vertex_count;
							let triangle_offset = self.visiblity_info.triangle_count;

							let meshlet_count = (mesh.vertex_count).div_ceil(63);

							let mut mesh_vertex_count = 0;
							let mut mesh_triangle_count = 0;

							let mut meshlets = Vec::with_capacity(meshlet_count as usize);

							let meshlet_index_stream = mesh.index_streams.iter().find(|is| is.stream_type == mesh_resource_handler::IndexStreamTypes::Meshlets).unwrap();

							assert_eq!(meshlet_index_stream.data_type, mesh_resource_handler::IntegralTypes::U16, "Meshlet index stream is not u16");

							for _ in 0..meshlet_count {
								let meshlet_vertex_count = (mesh.vertex_count - mesh_vertex_count).min(63) as u8;
								let meshlet_triangle_count = (meshlet_index_stream.count / 3 - mesh_triangle_count).min(21) as u8;

								let meshlet_data = ShaderMeshletData {
									instance_index: self.visiblity_info.instance_count,
									vertex_offset: vertex_offset as u16 + mesh_vertex_count as u16,
									triangle_offset:triangle_offset as u16 + mesh_triangle_count as u16,
									vertex_count: meshlet_vertex_count,
									triangle_count: meshlet_triangle_count,
									pad: [0u8; 6],
								};
								
								meshlets.push(meshlet_data);

								mesh_vertex_count += meshlet_vertex_count as u32;
								mesh_triangle_count += meshlet_triangle_count as u32;
							}

							self.meshes.insert(resource.url.clone(), MeshData{ meshlets, vertex_count: mesh_vertex_count, triangle_count: mesh_triangle_count, });
						}
					}
					_ => {}
				}
			}
		}

		let meshes_data_slice = render_system.get_mut_buffer_slice(self.meshes_data_buffer);

		let mesh_data = ShaderInstanceData {
			model: mesh.transform,
			material_id: self.material_evaluation_materials.get(mesh.material_id).unwrap().0,
		};

		let meshes_data_slice = unsafe { std::slice::from_raw_parts_mut(meshes_data_slice.as_mut_ptr() as *mut ShaderInstanceData, 16) };

		meshes_data_slice[self.visiblity_info.instance_count as usize] = mesh_data;

		let meshlets_data_slice = render_system.get_mut_buffer_slice(self.meshlets_data_buffer);

		let meshlets_data_slice = unsafe { std::slice::from_raw_parts_mut(meshlets_data_slice.as_mut_ptr() as *mut ShaderMeshletData, 256) };

		let mesh = self.meshes.get(mesh.resource_id).expect("Mesh not loaded");

		for (i, meshlet) in mesh.meshlets.iter().enumerate() {
			let meshlet = ShaderMeshletData { instance_index: self.visiblity_info.instance_count, ..(*meshlet) };
			meshlets_data_slice[self.visiblity_info.meshlet_count as usize + i] = meshlet;
		}

		self.visiblity_info.meshlet_count += mesh.meshlets.len() as u32;
		self.visiblity_info.vertex_count += mesh.vertex_count;
		self.visiblity_info.triangle_count += mesh.triangle_count;
		self.visiblity_info.instance_count += 1;
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

		lighting_data.lights[light_index].position = crate::Vec3f::new(0.0, 2.0, 0.0);
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