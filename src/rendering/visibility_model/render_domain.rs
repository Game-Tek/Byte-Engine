use std::cmp::min;
use std::collections::HashMap;

use log::error;
use maths_rs::{prelude::MatTranslate, Mat4f};

use crate::orchestrator::EntityHandle;
use crate::rendering::mesh;
use crate::rendering::world_render_domain::WorldRenderDomain;
use crate::{resource_manager::{self, mesh_resource_handler, material_resource_handler::{Shader, Material, Variant}, texture_resource_handler}, rendering::{render_system::{RenderSystem, self}, directional_light::DirectionalLight, point_light::PointLight}, Extent, orchestrator::{Entity, System, self, OrchestratorReference}, Vector3, camera::{self}, math, window_system};
use crate::rendering::render_system::{BindingTables, BottomLevelAccelerationStructure, BottomLevelAccelerationStructureBuild, BottomLevelAccelerationStructureBuildDescriptions, BottomLevelAccelerationStructureDescriptions, BufferDescriptor, BufferStridedRange, DataTypes, DeviceAccesses, Encodings, ShaderTypes, TopLevelAccelerationStructureBuild, TopLevelAccelerationStructureBuildDescriptions, UseCases, Uses, CommandBufferRecording};

struct VisibilityInfo {
	instance_count: u32,
	triangle_count: u32,
	meshlet_count: u32,
	vertex_count: u32,
}

struct MeshData {
	meshlets: Vec<ShaderMeshletData>,
	vertex_count: u32,
	triangle_count: u32,
	/// The base index of the vertex buffer
	vertex_offset: u32,
	/// The base index into the triangle indices buffer
	triangle_offset: u32,
	acceleration_structure: Option<render_system::BottomLevelAccelerationStructureHandle>,
}

enum MeshState {
	Build {
		mesh_handle: String,
	},
	Update {},
}

struct RayTracing {
	top_level_acceleration_structure: render_system::TopLevelAccelerationStructureHandle,
	descriptor_set_template: render_system::DescriptorSetTemplateHandle,
	descriptor_set: render_system::DescriptorSetHandle,
	pipeline_layout: render_system::PipelineLayoutHandle,
	pipeline: render_system::PipelineHandle,

	ray_gen_sbt_buffer: render_system::BaseBufferHandle,
	miss_sbt_buffer: render_system::BaseBufferHandle,
	hit_sbt_buffer: render_system::BaseBufferHandle,

	shadow_map_resolution: Extent,
	shadow_map: render_system::ImageHandle,

	instances_buffer: render_system::BaseBufferHandle,
	scratch_buffer: render_system::BaseBufferHandle,

	pending_meshes: Vec<MeshState>,
}

/// This the visibility buffer implementation of the world render domain.
pub struct VisibilityWorldRenderDomain {
	visibility_info: VisibilityInfo,

	camera: Option<EntityHandle<crate::camera::Camera>>,

	meshes: HashMap<String, MeshData>,

	mesh_resources: HashMap<&'static str, u32>,

	/// Maps resource ids to shaders
	/// The hash and the shader handle are stored to determine if the shader has changed
	shaders: std::collections::HashMap<u64, (u64, render_system::ShaderHandle, render_system::ShaderTypes)>,

	material_evaluation_materials: HashMap<String, (u32, render_system::PipelineHandle)>,

	pending_texture_loads: Vec<render_system::ImageHandle>,
	
	occlusion_map: render_system::ImageHandle,

	transfer_synchronizer: render_system::SynchronizerHandle,
	transfer_command_buffer: render_system::CommandBufferHandle,

	// Visibility

	pipeline_layout_handle: render_system::PipelineLayoutHandle,

	vertex_positions_buffer: render_system::BaseBufferHandle,
	vertex_normals_buffer: render_system::BaseBufferHandle,

	/// Indices laid out as a triangle list
	triangle_indices_buffer: render_system::BaseBufferHandle,

	/// Indices laid out as indices into the vertex buffers
	vertex_indices_buffer: render_system::BaseBufferHandle,
	/// Indices laid out as indices into the `vertex_indices_buffer`
	primitive_indices_buffer: render_system::BaseBufferHandle,

	albedo: render_system::ImageHandle,
	depth_target: render_system::ImageHandle,

	camera_data_buffer_handle: render_system::BaseBufferHandle,
	materials_data_buffer_handle: render_system::BaseBufferHandle,

	descriptor_set_layout: render_system::DescriptorSetTemplateHandle,
	descriptor_set: render_system::DescriptorSetHandle,

	textures_binding: render_system::DescriptorSetBindingHandle,

	meshes_data_buffer: render_system::BaseBufferHandle,
	meshlets_data_buffer: render_system::BaseBufferHandle,
	vertex_layout: [render_system::VertexElement; 2],

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

	material_evaluation_descriptor_set_layout: render_system::DescriptorSetTemplateHandle,
	material_evaluation_descriptor_set: render_system::DescriptorSetHandle,
	material_evaluation_pipeline_layout: render_system::PipelineLayoutHandle,

	debug_position: render_system::ImageHandle,
	debug_normal: render_system::ImageHandle,
	light_data_buffer: render_system::BaseBufferHandle,
}

impl VisibilityWorldRenderDomain {
	pub fn new<'a>(render_system: &'a mut dyn render_system::RenderSystem) -> orchestrator::EntityReturn<'a, Self> {
		orchestrator::EntityReturn::new_from_function(move |orchestrator| {
			let bindings = [
				render_system::DescriptorSetBindingTemplate::new(0, render_system::DescriptorType::StorageBuffer, render_system::Stages::MESH | render_system::Stages::FRAGMENT | render_system::Stages::RAYGEN | render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(1, render_system::DescriptorType::StorageBuffer, render_system::Stages::MESH | render_system::Stages::FRAGMENT | render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(2, render_system::DescriptorType::StorageBuffer, render_system::Stages::MESH | render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(3, render_system::DescriptorType::StorageBuffer, render_system::Stages::MESH | render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(4, render_system::DescriptorType::StorageBuffer, render_system::Stages::MESH | render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(5, render_system::DescriptorType::StorageBuffer, render_system::Stages::MESH | render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(6, render_system::DescriptorType::StorageBuffer, render_system::Stages::MESH | render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(7, render_system::DescriptorType::CombinedImageSampler, render_system::Stages::COMPUTE),
			];

			let descriptor_set_layout = render_system.create_descriptor_set_template(Some("Base Set Layout"), &bindings);

			let descriptor_set = render_system.create_descriptor_set(Some("Base Descriptor Set"), &descriptor_set_layout);

			let camera_data_binding = render_system.create_descriptor_binding(descriptor_set, &bindings[0]);
			let meshes_data_binding = render_system.create_descriptor_binding(descriptor_set, &bindings[1]);
			let vertex_positions_binding = render_system.create_descriptor_binding(descriptor_set, &bindings[2]);
			let vertex_normals_binding = render_system.create_descriptor_binding(descriptor_set, &bindings[3]);
			let vertex_indices_binding = render_system.create_descriptor_binding(descriptor_set, &bindings[4]);
			let primitive_indices_binding = render_system.create_descriptor_binding(descriptor_set, &bindings[5]);
			let meshlets_data_binding = render_system.create_descriptor_binding(descriptor_set, &bindings[6]);
			let textures_binding = render_system.create_descriptor_binding(descriptor_set, &bindings[7]);

			let pipeline_layout_handle = render_system.create_pipeline_layout(&[descriptor_set_layout], &[]);
			
			let vertex_positions_buffer_handle = render_system.create_buffer(Some("Visibility Vertex Positions Buffer"), std::mem::size_of::<[[f32; 3]; MAX_VERTICES]>(), render_system::Uses::Vertex | Uses::AccelerationStructureBuild | render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let vertex_normals_buffer_handle = render_system.create_buffer(Some("Visibility Vertex Normals Buffer"), std::mem::size_of::<[[f32; 3]; MAX_VERTICES]>(), render_system::Uses::Vertex | Uses::AccelerationStructureBuild | render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let triangle_indices_buffer_handle = render_system.create_buffer(Some("Visibility Triangle Indices Buffer"), std::mem::size_of::<[[u16; 3]; MAX_TRIANGLES]>(), render_system::Uses::Index | Uses::AccelerationStructureBuild | render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let vertex_indices_buffer_handle = render_system.create_buffer(Some("Visibility Index Buffer"), std::mem::size_of::<[[u8; 3]; MAX_TRIANGLES]>(), render_system::Uses::Index | Uses::AccelerationStructureBuild | render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let primitive_indices_buffer_handle = render_system.create_buffer(Some("Visibility Primitive Indices Buffer"), std::mem::size_of::<[[u16; 3]; MAX_PRIMITIVE_TRIANGLES]>(), render_system::Uses::Index | Uses::AccelerationStructureBuild | render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			let debug_position = render_system.create_image(Some("debug position"), Extent::new(1920, 1080, 1), render_system::Formats::RGBAu16, None, render_system::Uses::RenderTarget | render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
			let debug_normals = render_system.create_image(Some("debug normal"), Extent::new(1920, 1080, 1), render_system::Formats::RGBAu16, None, render_system::Uses::RenderTarget | render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let albedo = render_system.create_image(Some("albedo"), Extent::new(1920, 1080, 1), render_system::Formats::RGBAu16, None, render_system::Uses::RenderTarget | render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
			let depth_target = render_system.create_image(Some("depth_target"), Extent::new(1920, 1080, 1), render_system::Formats::Depth32, None, render_system::Uses::DepthStencil | render_system::Uses::Image, render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let camera_data_buffer_handle = render_system.create_buffer(Some("Visibility Camera Data"), 16 * 4 * 4, render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let meshes_data_buffer = render_system.create_buffer(Some("Visibility Meshes Data"), std::mem::size_of::<[ShaderInstanceData; MAX_INSTANCES]>(), render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let meshlets_data_buffer = render_system.create_buffer(Some("Visibility Meshlets Data"), std::mem::size_of::<[ShaderMeshletData; MAX_MESHLETS]>(), render_system::Uses::Storage, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			render_system.write(&[
				render_system::DescriptorWrite {
					binding_handle: camera_data_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: camera_data_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite {
					binding_handle: meshes_data_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: meshes_data_buffer, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite {
					binding_handle: vertex_positions_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: vertex_positions_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite {
					binding_handle: vertex_normals_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: vertex_normals_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite {
					binding_handle: vertex_indices_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: vertex_indices_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite {
					binding_handle: primitive_indices_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: primitive_indices_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite {
					binding_handle: meshlets_data_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer { handle: meshlets_data_buffer, size: render_system::Ranges::Whole },
				},
			]);

			let visibility_pass_mesh_shader = render_system.create_shader(render_system::ShaderSource::GLSL(VISIBILITY_PASS_MESH_SOURCE), render_system::ShaderTypes::Mesh,);
			let visibility_pass_fragment_shader = render_system.create_shader(render_system::ShaderSource::GLSL(VISIBILITY_PASS_FRAGMENT_SOURCE), render_system::ShaderTypes::Fragment,);

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

			let vertex_layout = [
				render_system::VertexElement{ name: "POSITION".to_string(), format: render_system::DataTypes::Float3, binding: 0 },
				render_system::VertexElement{ name: "NORMAL".to_string(), format: render_system::DataTypes::Float3, binding: 1 },
			];

			let visibility_pass_pipeline = render_system.create_raster_pipeline(&[
				render_system::PipelineConfigurationBlocks::Layout { layout: &pipeline_layout_handle },
				render_system::PipelineConfigurationBlocks::Shaders { shaders: &visibility_pass_shaders },
				render_system::PipelineConfigurationBlocks::RenderTargets { targets: &attachments },
			]);

			let material_count = render_system.create_buffer(Some("Material Count"), std::mem::size_of::<[u32; MAX_MATERIALS]>(), render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let material_offset = render_system.create_buffer(Some("Material Offset"), std::mem::size_of::<[u32; MAX_MATERIALS]>(), render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let material_offset_scratch = render_system.create_buffer(Some("Material Offset Scratch"), std::mem::size_of::<[u32; MAX_MATERIALS]>(), render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);
			let material_evaluation_dispatches = render_system.create_buffer(Some("Material Evaluation Dipatches"), std::mem::size_of::<[[u32; 3]; MAX_MATERIALS]>(), render_system::Uses::Storage | render_system::Uses::TransferDestination | render_system::Uses::Indirect, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			let material_xy = render_system.create_buffer(Some("Material XY"), 1920 * 1080 * 2 * 2, render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			let bindings = [
				render_system::DescriptorSetBindingTemplate::new(0, render_system::DescriptorType::StorageBuffer, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(1, render_system::DescriptorType::StorageBuffer, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(2, render_system::DescriptorType::StorageBuffer, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(3, render_system::DescriptorType::StorageBuffer, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(4, render_system::DescriptorType::StorageBuffer, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(5, render_system::DescriptorType::StorageImage, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(6, render_system::DescriptorType::StorageImage, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(7, render_system::DescriptorType::StorageImage, render_system::Stages::COMPUTE),
			];

			let visibility_descriptor_set_layout = render_system.create_descriptor_set_template(Some("Visibility Set Layout"), &bindings);
			let visibility_pass_pipeline_layout = render_system.create_pipeline_layout(&[descriptor_set_layout, visibility_descriptor_set_layout], &[]);
			let visibility_passes_descriptor_set = render_system.create_descriptor_set(Some("Visibility Descriptor Set"), &visibility_descriptor_set_layout);

			let material_count_binding = render_system.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[0]);
			let material_offset_binding = render_system.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[1]);
			let material_offset_scratch_binding = render_system.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[2]);
			let material_evaluation_dispatches_binding = render_system.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[3]);
			let material_xy_binding = render_system.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[4]);
			let _material_id_binding = render_system.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[5]);
			let vertex_id_binding = render_system.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[6]);
			let instance_id_binding = render_system.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[7]);

			render_system.write(&[
				render_system::DescriptorWrite { // MaterialCount
					binding_handle: material_count_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_count, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // MaterialOffset
					binding_handle: material_offset_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_offset, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // MaterialOffsetScratch
					binding_handle: material_offset_scratch_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_offset_scratch, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // MaterialEvaluationDispatches
					binding_handle: material_evaluation_dispatches_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_evaluation_dispatches, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // MaterialXY
					binding_handle: material_xy_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: material_xy, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // Primitive Index
					binding_handle: vertex_id_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Image{ handle: primitive_index, layout: render_system::Layouts::General },
				},
				render_system::DescriptorWrite { // InstanceId
					binding_handle: instance_id_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Image{ handle: instance_id, layout: render_system::Layouts::General },
				},
			]);

			let material_count_shader = render_system.create_shader(render_system::ShaderSource::GLSL(MATERIAL_COUNT_SOURCE), render_system::ShaderTypes::Compute,);
			let material_count_pipeline = render_system.create_compute_pipeline(&visibility_pass_pipeline_layout, (&material_count_shader, render_system::ShaderTypes::Compute, vec![]));

			let material_offset_shader = render_system.create_shader(render_system::ShaderSource::GLSL(MATERIAL_OFFSET_SOURCE), render_system::ShaderTypes::Compute,);
			let material_offset_pipeline = render_system.create_compute_pipeline(&visibility_pass_pipeline_layout, (&material_offset_shader, render_system::ShaderTypes::Compute, vec![]));

			let pixel_mapping_shader = render_system.create_shader(render_system::ShaderSource::GLSL(PIXEL_MAPPING_SOURCE), render_system::ShaderTypes::Compute,);
			let pixel_mapping_pipeline = render_system.create_compute_pipeline(&visibility_pass_pipeline_layout, (&pixel_mapping_shader, render_system::ShaderTypes::Compute, vec![]));

			let light_data_buffer = render_system.create_buffer(Some("Light Data"), std::mem::size_of::<LightingData>(), render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);
			
			let lighting_data = unsafe { (render_system.get_mut_buffer_slice(light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };
			
			lighting_data.count = 0; // Initially, no lights
			
			let materials_data_buffer_handle = render_system.create_buffer(Some("Materials Data"), MAX_MATERIALS * std::mem::size_of::<MaterialData>(), render_system::Uses::Storage | render_system::Uses::TransferDestination, render_system::DeviceAccesses::CpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::DYNAMIC);

			let bindings = [
				render_system::DescriptorSetBindingTemplate::new(0, render_system::DescriptorType::StorageImage, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(1, render_system::DescriptorType::StorageBuffer, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(2, render_system::DescriptorType::StorageImage, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(3, render_system::DescriptorType::StorageImage, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(4, render_system::DescriptorType::StorageBuffer, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(5, render_system::DescriptorType::StorageBuffer, render_system::Stages::COMPUTE),
				render_system::DescriptorSetBindingTemplate::new(10, render_system::DescriptorType::CombinedImageSampler, render_system::Stages::COMPUTE),
			];

			let material_evaluation_descriptor_set_layout = render_system.create_descriptor_set_template(Some("Material Evaluation Set Layout"), &bindings);
			let material_evaluation_descriptor_set = render_system.create_descriptor_set(Some("Material Evaluation Descriptor Set"), &material_evaluation_descriptor_set_layout);

			let albedo_binding = render_system.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[0]);
			let camera_data_binding = render_system.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[1]);
			let debug_position_binding = render_system.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[2]);
			let debug_normals_binding = render_system.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[3]);
			let light_data_binding = render_system.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[4]);
			let materials_data_binding = render_system.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[5]);
			let occlussion_texture_binding = render_system.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[6]);

			let sampler = render_system.create_sampler(render_system::FilteringModes::Linear, render_system::FilteringModes::Linear, render_system::SamplerAddressingModes::Clamp, None, 0f32, 0f32);
			let occlusion_map = render_system.create_image(Some("Occlusion Map"), Extent::new(1920, 1080, 1), render_system::Formats::R8(render_system::Encodings::UnsignedNormalized), None, render_system::Uses::Storage | render_system::Uses::Image, render_system::DeviceAccesses::GpuWrite | render_system::DeviceAccesses::GpuRead, render_system::UseCases::STATIC);

			render_system.write(&[
				render_system::DescriptorWrite { // albedo
					binding_handle: albedo_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Image{ handle: albedo, layout: render_system::Layouts::General },
				},
				render_system::DescriptorWrite { // CameraData
					binding_handle: camera_data_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: camera_data_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // debug_position
					binding_handle: debug_position_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Image{ handle: debug_position, layout: render_system::Layouts::General }
				},
				render_system::DescriptorWrite { // debug_normals
					binding_handle: debug_normals_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Image{ handle: debug_normals, layout: render_system::Layouts::General }
				},
				render_system::DescriptorWrite { // LightData
					binding_handle: light_data_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: light_data_buffer, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // MaterialsData
					binding_handle: materials_data_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::Buffer{ handle: materials_data_buffer_handle, size: render_system::Ranges::Whole },
				},
				render_system::DescriptorWrite { // OcclussionTexture
					binding_handle: occlussion_texture_binding,
					array_element: 0,
					descriptor: render_system::Descriptor::CombinedImageSampler{ image_handle: occlusion_map, sampler_handle: sampler, layout: render_system::Layouts::Read },
				},
			]);

			let material_evaluation_pipeline_layout = render_system.create_pipeline_layout(&[descriptor_set_layout, visibility_descriptor_set_layout, material_evaluation_descriptor_set_layout], &[render_system::PushConstantRange{ offset: 0, size: 4 }]);

			let transfer_synchronizer = render_system.create_synchronizer(Some("Transfer Synchronizer"), false);
			let transfer_command_buffer = render_system.create_command_buffer(Some("Transfer"));

			Self {
				visibility_info:  VisibilityInfo{ triangle_count: 0, instance_count: 0, meshlet_count:0, vertex_count:0, },

				shaders: HashMap::new(),

				camera: None,

				meshes: HashMap::new(),

				mesh_resources: HashMap::new(),

				material_evaluation_materials: HashMap::new(),

				pending_texture_loads: Vec::new(),
				
				occlusion_map,

				transfer_synchronizer,
				transfer_command_buffer,

				// Visibility

				pipeline_layout_handle,

				vertex_positions_buffer: vertex_positions_buffer_handle,
				vertex_normals_buffer: vertex_normals_buffer_handle,
				triangle_indices_buffer: triangle_indices_buffer_handle,
				vertex_indices_buffer: vertex_indices_buffer_handle,
				primitive_indices_buffer: primitive_indices_buffer_handle,

				descriptor_set_layout,
				descriptor_set,

				textures_binding,

				albedo,
				depth_target,

				camera_data_buffer_handle,

				meshes_data_buffer,
				meshlets_data_buffer,

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

				light_data_buffer,
				materials_data_buffer_handle,
				vertex_layout,
			}
		})
			// .add_post_creation_function(Box::new(Self::load_needed_assets))
			.add_listener::<camera::Camera>()
			.add_listener::<mesh::Mesh>()
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
					
					let sampler = render_system.create_sampler(render_system::FilteringModes::Linear, render_system::FilteringModes::Linear, render_system::SamplerAddressingModes::Clamp, None, 0f32, 0f32); // TODO: use actual sampler

					render_system.write(&[
						render_system::DescriptorWrite {
							binding_handle: self.textures_binding,
							array_element: 0, // TODO: use actual array element
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

					let new_shader = render_system.create_shader(render_system::ShaderSource::SPIRV(&buffer[offset..(offset + size)]), shader.stage,);

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

	pub fn render(&mut self, orchestrator: &OrchestratorReference, render_system: &dyn render_system::RenderSystem, command_buffer_recording: &mut dyn render_system::CommandBufferRecording) {
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

			command_buffer_recording.execute(&[], &[], self.transfer_synchronizer);
		}

		render_system.wait(self.transfer_synchronizer); // Bad

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

		command_buffer_recording.start_region("Visibility Model");

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
				handle: render_system::Handle::Buffer(self.primitive_indices_buffer),
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
				clear: render_system::ClearValue::Depth(0f32),
				load: false,
				store: true,
			},
		];

		command_buffer_recording.start_region("Visibility Render Model");

		command_buffer_recording.start_region("Visibility Buffer");

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
				handle: render_system::Handle::Buffer(self.primitive_indices_buffer),
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

		command_buffer_recording.start_render_pass(Extent::plane(1920, 1080), &attachments);

		command_buffer_recording.bind_raster_pipeline(&self.visibility_pass_pipeline);

		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout_handle, &[self.descriptor_set]);

		command_buffer_recording.dispatch_meshes(self.visibility_info.meshlet_count, 1, 1);

		command_buffer_recording.end_render_pass();

		command_buffer_recording.clear_buffers(&[self.material_count, self.material_offset, self.material_offset_scratch, self.material_evaluation_dispatches, self.material_xy]);

		command_buffer_recording.end_region();

		command_buffer_recording.start_region("Material Count");

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
		command_buffer_recording.bind_descriptor_sets(&self.visibility_pass_pipeline_layout, &[self.descriptor_set, self.visibility_passes_descriptor_set]);
		command_buffer_recording.dispatch(render_system::DispatchExtent { workgroup_extent: Extent::square(32), dispatch_extent: Extent::plane(1920, 1080) });

		command_buffer_recording.end_region();

		command_buffer_recording.start_region("Material Offset");

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
		command_buffer_recording.bind_descriptor_sets(&self.visibility_pass_pipeline_layout, &[self.descriptor_set, self.visibility_passes_descriptor_set]);
		command_buffer_recording.dispatch(render_system::DispatchExtent { workgroup_extent: Extent { width: 1, height: 1, depth: 1 }, dispatch_extent: Extent { width: 1, height: 1, depth: 1 } });

		command_buffer_recording.end_region();

		command_buffer_recording.start_region("Pixel Mapping");

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
		command_buffer_recording.bind_descriptor_sets(&self.visibility_pass_pipeline_layout, &[self.descriptor_set, self.visibility_passes_descriptor_set]);
		command_buffer_recording.dispatch(render_system::DispatchExtent { workgroup_extent: Extent::square(32), dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });

		command_buffer_recording.end_region();

		command_buffer_recording.start_region("Material Evaluation");
		
		command_buffer_recording.clear_images(&[(self.albedo, render_system::ClearValue::Color(crate::RGBA::black())),(self.occlusion_map, render_system::ClearValue::Color(crate::RGBA::white()))]);

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
			render_system::Consumption{
				handle: render_system::Handle::Image(self.occlusion_map),
				stages: render_system::Stages::COMPUTE,
				access: render_system::AccessPolicies::READ,
				layout: render_system::Layouts::Read,
			},
		]);

		command_buffer_recording.bind_descriptor_sets(&self.material_evaluation_pipeline_layout, &[self.descriptor_set, self.visibility_passes_descriptor_set, self.material_evaluation_descriptor_set]);

		for (_, (i, pipeline)) in self.material_evaluation_materials.iter() {
			// No need for sync here, as each thread across all invocations will write to a different pixel
			command_buffer_recording.bind_compute_pipeline(pipeline);
			command_buffer_recording.write_to_push_constant(&self.material_evaluation_pipeline_layout, 0, unsafe {
				std::slice::from_raw_parts(&(*i as u32) as *const u32 as *const u8, std::mem::size_of::<u32>())
			});
			command_buffer_recording.indirect_dispatch(&render_system::BufferDescriptor { buffer: self.material_evaluation_dispatches, offset: (*i as u64 * 12), range: 12, slot: 0 });
		}

		command_buffer_recording.end_region();

		// render_system.wait(self.transfer_synchronizer); // Wait for buffers to be copied over to the GPU, or else we might overwrite them on the CPU before they are copied over

		command_buffer_recording.end_region();
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
	vertex_triangles_offset: u16,
	triangle_offset: u16,
	vertex_count: u8,
	triangle_count: u8,
}

#[repr(C)]
struct ShaderInstanceData {
	model: Mat4f,
	material_id: u32,
	base_vertex_index: u32,
}

#[repr(C)]
struct LightingData {
	count: u32,
	lights: [LightData; MAX_LIGHTS],
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

impl orchestrator::EntitySubscriber<mesh::Mesh> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<mesh::Mesh>, mesh: &mesh::Mesh) {
		let render_system = orchestrator.get_by_class::<render_system::RenderSystemImplementation>();
		let mut render_system = render_system.get_mut();
		let render_system = render_system.downcast_mut::<render_system::RenderSystemImplementation>().unwrap();

		orchestrator.tie_self(Self::transform, &handle, mesh::Mesh::transform);

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

			let mut meshlet_stream_buffer = vec![0u8; 1024 * 8];

			for resource in &resource_request.resources {
				match resource.class.as_str() {
					"Mesh" => {
						let vertex_positions_buffer = render_system.get_mut_buffer_slice(self.vertex_positions_buffer);
						let vertex_normals_buffer = render_system.get_mut_buffer_slice(self.vertex_normals_buffer);
						let vertex_indices_buffer = render_system.get_mut_buffer_slice(self.vertex_indices_buffer);
						let primitive_indices_buffer = render_system.get_mut_buffer_slice(self.primitive_indices_buffer);
						let triangle_indices_buffer = render_system.get_mut_buffer_slice(self.triangle_indices_buffer);

						options.resources.push(resource_manager::OptionResource {
							url: resource.url.clone(),
							streams: vec![
								resource_manager::Stream{ buffer: &mut vertex_positions_buffer[(self.visibility_info.vertex_count as usize * std::mem::size_of::<Vector3>())..], name: "Vertex.Position".to_string() },
								resource_manager::Stream{ buffer: &mut vertex_normals_buffer[(self.visibility_info.vertex_count as usize * std::mem::size_of::<Vector3>())..], name: "Vertex.Normal".to_string() },
								resource_manager::Stream{ buffer: &mut triangle_indices_buffer[(self.visibility_info.triangle_count as usize * 3 * std::mem::size_of::<u16>())..], name: "TriangleIndices".to_string() },
								resource_manager::Stream{ buffer: &mut vertex_indices_buffer[(self.visibility_info.vertex_count as usize * std::mem::size_of::<u16>())..], name: "VertexIndices".to_string() },
								resource_manager::Stream{ buffer: &mut primitive_indices_buffer[(self.visibility_info.triangle_count as usize * 3 * std::mem::size_of::<u8>())..], name: "MeshletIndices".to_string() },
								resource_manager::Stream{ buffer: meshlet_stream_buffer.as_mut_slice() , name: "Meshlets".to_string() },
							],
						});

						break;
					}
					_ => {}
				}
			}

			let resource = if let Ok(a) = resource_manager.load_resource(resource_request, Some(options), None) { a } else { return; };

			let (response, _buffer) = (resource.0, resource.1.unwrap());

			for resource in &response.resources {
				match resource.class.as_str() {
					"Mesh" => {
						self.mesh_resources.insert(mesh.resource_id, self.visibility_info.triangle_count);

						let mesh_resource: &mesh_resource_handler::Mesh = resource.resource.downcast_ref().unwrap();

						let acceleration_structure = if false {
							let triangle_index_stream = mesh_resource.index_streams.iter().find(|is| is.stream_type == mesh_resource_handler::IndexStreamTypes::Triangles).unwrap();

							assert_eq!(triangle_index_stream.data_type, mesh_resource_handler::IntegralTypes::U16, "Triangle index stream is not u16");

							let bottom_level_acceleration_structure = render_system.create_bottom_level_acceleration_structure(&BottomLevelAccelerationStructure{
								description: BottomLevelAccelerationStructureDescriptions::Mesh {
									vertex_count: mesh_resource.vertex_count,
									vertex_position_encoding: Encodings::IEEE754,
									triangle_count: triangle_index_stream.count / 3,
									index_format: DataTypes::U16,
								}
							});

							// ray_tracing.pending_meshes.push(MeshState::Build { mesh_handle: mesh.resource_id.to_string() });

							Some(bottom_level_acceleration_structure)
						} else {
							None
						};

						{
							let vertex_triangles_offset = self.visibility_info.vertex_count;
							let primitive_triangle_offset = self.visibility_info.triangle_count;

							let meshlet_count;

							if let Some(meshlet_data) = &mesh_resource.meshlet_stream {
								meshlet_count = meshlet_data.count;
							} else {
								meshlet_count = 0;
							};

							let mut mesh_vertex_count = 0;
							let mut mesh_triangle_count = 0;

							let mut meshlets = Vec::with_capacity(meshlet_count as usize);

							let vertex_index_stream = mesh_resource.index_streams.iter().find(|is| is.stream_type == mesh_resource_handler::IndexStreamTypes::Vertices).unwrap();
							let meshlet_index_stream = mesh_resource.index_streams.iter().find(|is| is.stream_type == mesh_resource_handler::IndexStreamTypes::Meshlets).unwrap();

							assert_eq!(meshlet_index_stream.data_type, mesh_resource_handler::IntegralTypes::U8, "Meshlet index stream is not u8");

							struct Meshlet {
								vertex_count: u8,
								triangle_count: u8,
							}

							let meshlet_stream = unsafe {
								// assert_eq!(meshlet_count as usize, meshlet_stream_buffer.len() / std::mem::size_of::<Meshlet>());
								std::slice::from_raw_parts(meshlet_stream_buffer.as_ptr() as *const Meshlet, meshlet_count as usize)
							};

							for i in 0..meshlet_count as usize {
								let meshlet = &meshlet_stream[i];
								let meshlet_vertex_count = meshlet.vertex_count;
								let meshlet_triangle_count = meshlet.triangle_count;

								let meshlet_data = ShaderMeshletData {
									instance_index: self.visibility_info.instance_count,
									vertex_triangles_offset: vertex_triangles_offset as u16 + mesh_vertex_count as u16,
									triangle_offset:primitive_triangle_offset as u16 + mesh_triangle_count as u16,
									vertex_count: meshlet_vertex_count,
									triangle_count: meshlet_triangle_count,
								};
								
								meshlets.push(meshlet_data);

								mesh_vertex_count += meshlet_vertex_count as u32;
								mesh_triangle_count += meshlet_triangle_count as u32;
							}

							self.meshes.insert(resource.url.clone(), MeshData{ meshlets, vertex_count: mesh_vertex_count, triangle_count: mesh_triangle_count, vertex_offset: self.visibility_info.vertex_count, triangle_offset: self.visibility_info.triangle_count, acceleration_structure });
						}
					}
					_ => {}
				}
			}
		}

		let meshes_data_slice = render_system.get_mut_buffer_slice(self.meshes_data_buffer);

		let mesh_data = self.meshes.get(mesh.resource_id).expect("Mesh not loaded");

		let shader_mesh_data = ShaderInstanceData {
			model: mesh.transform,
			material_id: self.material_evaluation_materials.get(mesh.material_id).unwrap().0,
			base_vertex_index: mesh_data.vertex_offset,
		};

		let meshes_data_slice = unsafe { std::slice::from_raw_parts_mut(meshes_data_slice.as_mut_ptr() as *mut ShaderInstanceData, MAX_INSTANCES) };

		meshes_data_slice[self.visibility_info.instance_count as usize] = shader_mesh_data;

		if let (Some(ray_tracing), Some(acceleration_structure)) = (Option::<RayTracing>::None, mesh_data.acceleration_structure) {
			let transform = [
				[mesh.transform[0], mesh.transform[1], mesh.transform[2], mesh.transform[3]],
				[mesh.transform[4], mesh.transform[5], mesh.transform[6], mesh.transform[7]],
				[mesh.transform[8], mesh.transform[9], mesh.transform[10], mesh.transform[11]],
			];

			render_system.write_instance(ray_tracing.instances_buffer, self.visibility_info.instance_count as usize, transform, self.visibility_info.instance_count as u16, 0xFF, 0, acceleration_structure);
		}

		let meshlets_data_slice = render_system.get_mut_buffer_slice(self.meshlets_data_buffer);

		let meshlets_data_slice = unsafe { std::slice::from_raw_parts_mut(meshlets_data_slice.as_mut_ptr() as *mut ShaderMeshletData, MAX_MESHLETS) };

		for (i, meshlet) in mesh_data.meshlets.iter().enumerate() {
			let meshlet = ShaderMeshletData { instance_index: self.visibility_info.instance_count, ..(*meshlet) };
			meshlets_data_slice[self.visibility_info.meshlet_count as usize + i] = meshlet;
		}

		self.visibility_info.meshlet_count += mesh_data.meshlets.len() as u32;
		self.visibility_info.vertex_count += mesh_data.vertex_count;
		self.visibility_info.triangle_count += mesh_data.triangle_count;
		self.visibility_info.instance_count += 1;

		assert!((self.visibility_info.meshlet_count as usize) < MAX_MESHLETS, "Meshlet count exceeded");
		assert!((self.visibility_info.instance_count as usize) < MAX_INSTANCES, "Instance count exceeded");
		assert!((self.visibility_info.vertex_count as usize) < MAX_VERTICES, "Vertex count exceeded");
		assert!((self.visibility_info.vertex_count as usize) < MAX_PRIMITIVE_TRIANGLES, "Primitive triangle count exceeded");
		assert!((self.visibility_info.triangle_count as usize) < MAX_TRIANGLES, "Triangle count exceeded");
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

		assert!(lighting_data.count < MAX_LIGHTS as u32, "Light count exceeded");
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

		assert!(lighting_data.count < MAX_LIGHTS as u32, "Light count exceeded");
	}
}

impl Entity for VisibilityWorldRenderDomain {}
impl System for VisibilityWorldRenderDomain {}

impl WorldRenderDomain for VisibilityWorldRenderDomain {
	fn get_descriptor_set_template(&self) -> render_system::DescriptorSetTemplateHandle {
		self.descriptor_set_layout
	}

	fn get_result_image(&self) -> render_system::ImageHandle {
		self.albedo
	}
}

const VERTEX_COUNT: u32 = 64;
const TRIANGLE_COUNT: u32 = 126;

const MAX_MESHLETS: usize = 1024;
const MAX_INSTANCES: usize = 1024;
const MAX_MATERIALS: usize = 1024;
const MAX_LIGHTS: usize = 16;
const MAX_TRIANGLES: usize = 65536;
const MAX_PRIMITIVE_TRIANGLES: usize = 65536;
const MAX_VERTICES: usize = 65536;

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
		gl_MeshVerticesEXT[gl_LocalInvocationID.x].gl_Position = camera.view_projection * meshes[instance_index].model * vec4(vertex_positions[vertex_index], 1.0);
		// gl_MeshVerticesEXT[gl_LocalInvocationID.x].gl_Position = vec4(vertex_positions[vertex_index], 1.0);
	}
	
	if (gl_LocalInvocationID.x < uint(meshlet.triangle_count)) {
		uint triangle_index = uint(meshlet.triangle_offset) + gl_LocalInvocationID.x;
		uint triangle_indices[3] = uint[](primitive_indices[triangle_index * 3 + 0], primitive_indices[triangle_index * 3 + 1], primitive_indices[triangle_index * 3 + 2]);
		gl_PrimitiveTriangleIndicesEXT[gl_LocalInvocationID.x] = uvec3(triangle_indices[0], triangle_indices[1], triangle_indices[2]);
		out_instance_index[gl_LocalInvocationID.x] = instance_index;
		out_primitive_index[gl_LocalInvocationID.x] = (meshlet_index << 8) | (gl_LocalInvocationID.x & 0xFF);
	}
}";

const VISIBILITY_PASS_FRAGMENT_SOURCE: &'static str = r#"
#version 450
#pragma shader_stage(fragment)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_shader_explicit_arithmetic_types : enable
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

const MATERIAL_COUNT_SOURCE: &'static str = r#"
#version 450
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_explicit_arithmetic_types : enable

struct Mesh {
	mat4 model;
	uint material_index;
	uint32_t base_vertex_index;
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

const MATERIAL_OFFSET_SOURCE: &'static str = r#"
#version 450
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_explicit_arithmetic_types : enable

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

const PIXEL_MAPPING_SOURCE: &'static str = r#"
#version 450
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_explicit_arithmetic_types : enable

struct Mesh {
	mat4 model;
	uint material_index;
	uint32_t base_vertex_index;
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