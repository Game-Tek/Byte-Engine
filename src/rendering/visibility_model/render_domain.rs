use std::borrow::{BorrowMut, Borrow};
use std::collections::HashMap;
use std::ops::{DerefMut, Deref};
use std::rc::Rc;
use std::sync::{Arc, RwLock};

use log::error;
use maths_rs::{prelude::MatTranslate, Mat4f};

use crate::ghi;
use crate::orchestrator::EntityHandle;
use crate::rendering::{mesh, directional_light, point_light};
use crate::rendering::world_render_domain::WorldRenderDomain;
use crate::resource_management::resource_manager::ResourceManager;
use crate::{resource_management::{self, mesh_resource_handler, material_resource_handler::{Shader, Material, Variant}, texture_resource_handler}, Extent, orchestrator::{Entity, System, self, OrchestratorReference}, Vector3, camera::{self}, math};

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
	acceleration_structure: Option<ghi::BottomLevelAccelerationStructureHandle>,
}

enum MeshState {
	Build {
		mesh_handle: String,
	},
	Update {},
}

struct RayTracing {
	top_level_acceleration_structure: ghi::TopLevelAccelerationStructureHandle,
	descriptor_set_template: ghi::DescriptorSetTemplateHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	pipeline_layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,

	ray_gen_sbt_buffer: ghi::BaseBufferHandle,
	miss_sbt_buffer: ghi::BaseBufferHandle,
	hit_sbt_buffer: ghi::BaseBufferHandle,

	shadow_map_resolution: Extent,
	shadow_map: ghi::ImageHandle,

	instances_buffer: ghi::BaseBufferHandle,
	scratch_buffer: ghi::BaseBufferHandle,

	pending_meshes: Vec<MeshState>,
}

/// This the visibility buffer implementation of the world render domain.
pub struct VisibilityWorldRenderDomain {
	ghi: Rc<RwLock<dyn ghi::GraphicsHardwareInterface>>,

	resource_manager: EntityHandle<ResourceManager>,

	visibility_info: VisibilityInfo,

	camera: Option<EntityHandle<crate::camera::Camera>>,

	meshes: HashMap<String, MeshData>,

	mesh_resources: HashMap<&'static str, u32>,

	/// Maps resource ids to shaders
	/// The hash and the shader handle are stored to determine if the shader has changed
	shaders: std::collections::HashMap<u64, (u64, ghi::ShaderHandle, ghi::ShaderTypes)>,

	material_evaluation_materials: HashMap<String, (u32, ghi::PipelineHandle)>,

	pending_texture_loads: Vec<ghi::ImageHandle>,
	
	occlusion_map: ghi::ImageHandle,

	transfer_synchronizer: ghi::SynchronizerHandle,
	transfer_command_buffer: ghi::CommandBufferHandle,

	// Visibility

	pipeline_layout_handle: ghi::PipelineLayoutHandle,

	vertex_positions_buffer: ghi::BaseBufferHandle,
	vertex_normals_buffer: ghi::BaseBufferHandle,

	/// Indices laid out as a triangle list
	triangle_indices_buffer: ghi::BaseBufferHandle,

	/// Indices laid out as indices into the vertex buffers
	vertex_indices_buffer: ghi::BaseBufferHandle,
	/// Indices laid out as indices into the `vertex_indices_buffer`
	primitive_indices_buffer: ghi::BaseBufferHandle,

	albedo: ghi::ImageHandle,
	depth_target: ghi::ImageHandle,

	camera_data_buffer_handle: ghi::BaseBufferHandle,
	materials_data_buffer_handle: ghi::BaseBufferHandle,

	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	descriptor_set: ghi::DescriptorSetHandle,

	textures_binding: ghi::DescriptorSetBindingHandle,

	meshes_data_buffer: ghi::BaseBufferHandle,
	meshlets_data_buffer: ghi::BaseBufferHandle,
	vertex_layout: [ghi::VertexElement; 2],

	visibility_pass_pipeline_layout: ghi::PipelineLayoutHandle,
	visibility_passes_descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_pipeline: ghi::PipelineHandle,

	material_count_pipeline: ghi::PipelineHandle,
	material_offset_pipeline: ghi::PipelineHandle,
	pixel_mapping_pipeline: ghi::PipelineHandle,

	instance_id: ghi::ImageHandle,
	primitive_index: ghi::ImageHandle,

	material_count: ghi::BaseBufferHandle,
	material_offset: ghi::BaseBufferHandle,
	material_offset_scratch: ghi::BaseBufferHandle,
	material_evaluation_dispatches: ghi::BaseBufferHandle,
	material_xy: ghi::BaseBufferHandle,

	material_evaluation_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
	material_evaluation_pipeline_layout: ghi::PipelineLayoutHandle,

	debug_position: ghi::ImageHandle,
	debug_normal: ghi::ImageHandle,
	light_data_buffer: ghi::BaseBufferHandle,
}

impl VisibilityWorldRenderDomain {
	pub fn new<'a>(ghi: Rc<RwLock<dyn ghi::GraphicsHardwareInterface>>, resource_manager_handle: EntityHandle<ResourceManager>) -> orchestrator::EntityReturn<'a, Self> {
		orchestrator::EntityReturn::new_from_function(move |orchestrator| {
			let occlusion_map;
			let transfer_synchronizer;
			let transfer_command_buffer;
			let pipeline_layout_handle;
			let vertex_positions_buffer_handle;
			let vertex_normals_buffer_handle;
			let triangle_indices_buffer_handle;
			let vertex_indices_buffer_handle;
			let primitive_indices_buffer_handle;
			let descriptor_set_layout;
			let descriptor_set;
			let textures_binding;
			let albedo;
			let depth_target;
			let camera_data_buffer_handle;
			let meshes_data_buffer;
			let meshlets_data_buffer;
			let visibility_descriptor_set_layout;
			let visibility_pass_pipeline_layout;
			let visibility_passes_descriptor_set;
			let visibility_pass_pipeline;
			let material_count_pipeline;
			let material_offset_pipeline;
			let pixel_mapping_pipeline;
			let material_evaluation_descriptor_set_layout;
			let material_evaluation_descriptor_set;
			let material_evaluation_pipeline_layout;
			let primitive_index;
			let instance_id;
			let debug_position;
			let debug_normals;
			let material_count;
			let material_offset;
			let material_offset_scratch;
			let material_evaluation_dispatches;
			let material_xy;
			let light_data_buffer;
			let materials_data_buffer_handle;
			let vertex_layout;

			{
				let mut ghi_instance = ghi.write().unwrap();

				let bindings = [
					ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH | ghi::Stages::FRAGMENT | ghi::Stages::RAYGEN | ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH | ghi::Stages::FRAGMENT | ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH | ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(3, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH | ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(4, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH | ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(5, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH | ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(6, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH | ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(7, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE),
				];

				descriptor_set_layout = ghi_instance.create_descriptor_set_template(Some("Base Set Layout"), &bindings);

				descriptor_set = ghi_instance.create_descriptor_set(Some("Base Descriptor Set"), &descriptor_set_layout);

				let camera_data_binding = ghi_instance.create_descriptor_binding(descriptor_set, &bindings[0]);
				let meshes_data_binding = ghi_instance.create_descriptor_binding(descriptor_set, &bindings[1]);
				let vertex_positions_binding = ghi_instance.create_descriptor_binding(descriptor_set, &bindings[2]);
				let vertex_normals_binding = ghi_instance.create_descriptor_binding(descriptor_set, &bindings[3]);
				let vertex_indices_binding = ghi_instance.create_descriptor_binding(descriptor_set, &bindings[4]);
				let primitive_indices_binding = ghi_instance.create_descriptor_binding(descriptor_set, &bindings[5]);
				let meshlets_data_binding = ghi_instance.create_descriptor_binding(descriptor_set, &bindings[6]);
				textures_binding = ghi_instance.create_descriptor_binding(descriptor_set, &bindings[7]);

				pipeline_layout_handle = ghi_instance.create_pipeline_layout(&[descriptor_set_layout], &[]);
				
				vertex_positions_buffer_handle = ghi_instance.create_buffer(Some("Visibility Vertex Positions Buffer"), std::mem::size_of::<[[f32; 3]; MAX_VERTICES]>(), ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
				vertex_normals_buffer_handle = ghi_instance.create_buffer(Some("Visibility Vertex Normals Buffer"), std::mem::size_of::<[[f32; 3]; MAX_VERTICES]>(), ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
				triangle_indices_buffer_handle = ghi_instance.create_buffer(Some("Visibility Triangle Indices Buffer"), std::mem::size_of::<[[u16; 3]; MAX_TRIANGLES]>(), ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
				vertex_indices_buffer_handle = ghi_instance.create_buffer(Some("Visibility Index Buffer"), std::mem::size_of::<[[u8; 3]; MAX_TRIANGLES]>(), ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
				primitive_indices_buffer_handle = ghi_instance.create_buffer(Some("Visibility Primitive Indices Buffer"), std::mem::size_of::<[[u16; 3]; MAX_PRIMITIVE_TRIANGLES]>(), ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

				debug_position = ghi_instance.create_image(Some("debug position"), Extent::new(1920, 1080, 1), ghi::Formats::RGBAu16, None, ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
				debug_normals = ghi_instance.create_image(Some("debug normal"), Extent::new(1920, 1080, 1), ghi::Formats::RGBAu16, None, ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

				albedo = ghi_instance.create_image(Some("albedo"), Extent::new(1920, 1080, 1), ghi::Formats::RGBAu16, None, ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
				depth_target = ghi_instance.create_image(Some("depth_target"), Extent::new(1920, 1080, 1), ghi::Formats::Depth32, None, ghi::Uses::DepthStencil | ghi::Uses::Image, ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

				camera_data_buffer_handle = ghi_instance.create_buffer(Some("Visibility Camera Data"), 16 * 4 * 4, ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

				meshes_data_buffer = ghi_instance.create_buffer(Some("Visibility Meshes Data"), std::mem::size_of::<[ShaderInstanceData; MAX_INSTANCES]>(), ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
				meshlets_data_buffer = ghi_instance.create_buffer(Some("Visibility Meshlets Data"), std::mem::size_of::<[ShaderMeshletData; MAX_MESHLETS]>(), ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

				ghi_instance.write(&[
					ghi::DescriptorWrite {
						binding_handle: camera_data_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: camera_data_buffer_handle, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite {
						binding_handle: meshes_data_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: meshes_data_buffer, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite {
						binding_handle: vertex_positions_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: vertex_positions_buffer_handle, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite {
						binding_handle: vertex_normals_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: vertex_normals_buffer_handle, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite {
						binding_handle: vertex_indices_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: vertex_indices_buffer_handle, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite {
						binding_handle: primitive_indices_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: primitive_indices_buffer_handle, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite {
						binding_handle: meshlets_data_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer { handle: meshlets_data_buffer, size: ghi::Ranges::Whole },
					},
				]);

				let visibility_pass_mesh_shader = ghi_instance.create_shader(ghi::ShaderSource::GLSL(VISIBILITY_PASS_MESH_SOURCE), ghi::ShaderTypes::Mesh,
					&[
						ghi::ShaderBindingDescriptor::new(0, 0, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 1, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 2, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 3, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 4, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 5, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 6, ghi::AccessPolicies::READ),
					]
				);

				let visibility_pass_fragment_shader = ghi_instance.create_shader(ghi::ShaderSource::GLSL(VISIBILITY_PASS_FRAGMENT_SOURCE), ghi::ShaderTypes::Fragment, &[]);

				let visibility_pass_shaders = [
					(&visibility_pass_mesh_shader, ghi::ShaderTypes::Mesh, vec![]),
					(&visibility_pass_fragment_shader, ghi::ShaderTypes::Fragment, vec![]),
				];

				primitive_index = ghi_instance.create_image(Some("primitive index"), crate::Extent::new(1920, 1080, 1), ghi::Formats::U32, None, ghi::Uses::RenderTarget | ghi::Uses::Storage, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
				instance_id = ghi_instance.create_image(Some("instance_id"), crate::Extent::new(1920, 1080, 1), ghi::Formats::U32, None, ghi::Uses::RenderTarget | ghi::Uses::Storage, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

				let attachments = [
					ghi::AttachmentInformation {
						image: primitive_index,
						layout: ghi::Layouts::RenderTarget,
						format: ghi::Formats::U32,
						clear: ghi::ClearValue::Integer(!0u32, 0, 0, 0),
						load: false,
						store: true,
					},
					ghi::AttachmentInformation {
						image: instance_id,
						layout: ghi::Layouts::RenderTarget,
						format: ghi::Formats::U32,
						clear: ghi::ClearValue::Integer(!0u32, 0, 0, 0),
						load: false,
						store: true,
					},
					ghi::AttachmentInformation {
						image: depth_target,
						layout: ghi::Layouts::RenderTarget,
						format: ghi::Formats::Depth32,
						clear: ghi::ClearValue::Depth(0f32),
						load: false,
						store: true,
					},
				];

				vertex_layout = [
					ghi::VertexElement{ name: "POSITION".to_string(), format: ghi::DataTypes::Float3, binding: 0 },
					ghi::VertexElement{ name: "NORMAL".to_string(), format: ghi::DataTypes::Float3, binding: 1 },
				];

				visibility_pass_pipeline = ghi_instance.create_raster_pipeline(&[
					ghi::PipelineConfigurationBlocks::Layout { layout: &pipeline_layout_handle },
					ghi::PipelineConfigurationBlocks::Shaders { shaders: &visibility_pass_shaders },
					ghi::PipelineConfigurationBlocks::RenderTargets { targets: &attachments },
				]);

				material_count = ghi_instance.create_buffer(Some("Material Count"), std::mem::size_of::<[u32; MAX_MATERIALS]>(), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
				material_offset = ghi_instance.create_buffer(Some("Material Offset"), std::mem::size_of::<[u32; MAX_MATERIALS]>(), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
				material_offset_scratch = ghi_instance.create_buffer(Some("Material Offset Scratch"), std::mem::size_of::<[u32; MAX_MATERIALS]>(), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
				material_evaluation_dispatches = ghi_instance.create_buffer(Some("Material Evaluation Dipatches"), std::mem::size_of::<[[u32; 3]; MAX_MATERIALS]>(), ghi::Uses::Storage | ghi::Uses::TransferDestination | ghi::Uses::Indirect, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

				material_xy = ghi_instance.create_buffer(Some("Material XY"), 1920 * 1080 * 2 * 2, ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

				let bindings = [
					ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(3, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(4, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(5, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(6, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(7, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
				];

				visibility_descriptor_set_layout = ghi_instance.create_descriptor_set_template(Some("Visibility Set Layout"), &bindings);
				visibility_pass_pipeline_layout = ghi_instance.create_pipeline_layout(&[descriptor_set_layout, visibility_descriptor_set_layout], &[]);
				visibility_passes_descriptor_set = ghi_instance.create_descriptor_set(Some("Visibility Descriptor Set"), &visibility_descriptor_set_layout);

				let material_count_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[0]);
				let material_offset_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[1]);
				let material_offset_scratch_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[2]);
				let material_evaluation_dispatches_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[3]);
				let material_xy_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[4]);
				let _material_id_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[5]);
				let vertex_id_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[6]);
				let instance_id_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, &bindings[7]);

				ghi_instance.write(&[
					ghi::DescriptorWrite { // MaterialCount
						binding_handle: material_count_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: material_count, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite { // MaterialOffset
						binding_handle: material_offset_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: material_offset, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite { // MaterialOffsetScratch
						binding_handle: material_offset_scratch_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: material_offset_scratch, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite { // MaterialEvaluationDispatches
						binding_handle: material_evaluation_dispatches_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: material_evaluation_dispatches, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite { // MaterialXY
						binding_handle: material_xy_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: material_xy, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite { // Primitive Index
						binding_handle: vertex_id_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Image{ handle: primitive_index, layout: ghi::Layouts::General },
					},
					ghi::DescriptorWrite { // InstanceId
						binding_handle: instance_id_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Image{ handle: instance_id, layout: ghi::Layouts::General },
					},
				]);

				let material_count_shader = ghi_instance.create_shader(ghi::ShaderSource::GLSL(MATERIAL_COUNT_SOURCE), ghi::ShaderTypes::Compute,
					&[
						ghi::ShaderBindingDescriptor::new(0, 0, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(1, 0, ghi::AccessPolicies::READ | ghi::AccessPolicies::WRITE),
						ghi::ShaderBindingDescriptor::new(1, 7, ghi::AccessPolicies::READ),
					]
				);
				material_count_pipeline = ghi_instance.create_compute_pipeline(&visibility_pass_pipeline_layout, (&material_count_shader, ghi::ShaderTypes::Compute, vec![]));

				let material_offset_shader = ghi_instance.create_shader(ghi::ShaderSource::GLSL(MATERIAL_OFFSET_SOURCE), ghi::ShaderTypes::Compute,
					&[
						ghi::ShaderBindingDescriptor::new(1, 0, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(1, 1, ghi::AccessPolicies::WRITE),
						ghi::ShaderBindingDescriptor::new(1, 2, ghi::AccessPolicies::WRITE),
						ghi::ShaderBindingDescriptor::new(1, 3, ghi::AccessPolicies::WRITE),
					]
				);
				material_offset_pipeline = ghi_instance.create_compute_pipeline(&visibility_pass_pipeline_layout, (&material_offset_shader, ghi::ShaderTypes::Compute, vec![]));

				let pixel_mapping_shader = ghi_instance.create_shader(ghi::ShaderSource::GLSL(PIXEL_MAPPING_SOURCE), ghi::ShaderTypes::Compute,
					&[
						ghi::ShaderBindingDescriptor::new(0, 1, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(1, 1, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(1, 2, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(1, 4, ghi::AccessPolicies::WRITE),
						ghi::ShaderBindingDescriptor::new(1, 7, ghi::AccessPolicies::READ),
					]
				);
				pixel_mapping_pipeline = ghi_instance.create_compute_pipeline(&visibility_pass_pipeline_layout, (&pixel_mapping_shader, ghi::ShaderTypes::Compute, vec![]));

				light_data_buffer = ghi_instance.create_buffer(Some("Light Data"), std::mem::size_of::<LightingData>(), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
				
				let lighting_data = unsafe { (ghi_instance.get_mut_buffer_slice(light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };
				
				lighting_data.count = 0; // Initially, no lights
				
				materials_data_buffer_handle = ghi_instance.create_buffer(Some("Materials Data"), MAX_MATERIALS * std::mem::size_of::<MaterialData>(), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

				let bindings = [
					ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(3, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(4, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(5, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
					ghi::DescriptorSetBindingTemplate::new(10, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE),
				];

				material_evaluation_descriptor_set_layout = ghi_instance.create_descriptor_set_template(Some("Material Evaluation Set Layout"), &bindings);
				material_evaluation_descriptor_set = ghi_instance.create_descriptor_set(Some("Material Evaluation Descriptor Set"), &material_evaluation_descriptor_set_layout);

				let albedo_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[0]);
				let camera_data_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[1]);
				let debug_position_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[2]);
				let debug_normals_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[3]);
				let light_data_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[4]);
				let materials_data_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[5]);
				let occlussion_texture_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[6]);

				let sampler = ghi_instance.create_sampler(ghi::FilteringModes::Linear, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32);
				occlusion_map = ghi_instance.create_image(Some("Occlusion Map"), Extent::new(1920, 1080, 1), ghi::Formats::R8(ghi::Encodings::UnsignedNormalized), None, ghi::Uses::Storage | ghi::Uses::Image, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

				ghi_instance.write(&[
					ghi::DescriptorWrite { // albedo
						binding_handle: albedo_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Image{ handle: albedo, layout: ghi::Layouts::General },
					},
					ghi::DescriptorWrite { // CameraData
						binding_handle: camera_data_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: camera_data_buffer_handle, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite { // debug_position
						binding_handle: debug_position_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Image{ handle: debug_position, layout: ghi::Layouts::General }
					},
					ghi::DescriptorWrite { // debug_normals
						binding_handle: debug_normals_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Image{ handle: debug_normals, layout: ghi::Layouts::General }
					},
					ghi::DescriptorWrite { // LightData
						binding_handle: light_data_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: light_data_buffer, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite { // MaterialsData
						binding_handle: materials_data_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::Buffer{ handle: materials_data_buffer_handle, size: ghi::Ranges::Whole },
					},
					ghi::DescriptorWrite { // OcclussionTexture
						binding_handle: occlussion_texture_binding,
						array_element: 0,
						descriptor: ghi::Descriptor::CombinedImageSampler{ image_handle: occlusion_map, sampler_handle: sampler, layout: ghi::Layouts::Read },
					},
				]);

				material_evaluation_pipeline_layout = ghi_instance.create_pipeline_layout(&[descriptor_set_layout, visibility_descriptor_set_layout, material_evaluation_descriptor_set_layout], &[ghi::PushConstantRange{ offset: 0, size: 4 }]);

				transfer_synchronizer = ghi_instance.create_synchronizer(Some("Transfer Synchronizer"), false);
				transfer_command_buffer = ghi_instance.create_command_buffer(Some("Transfer"));
			}

			Self {
				ghi,

				resource_manager: resource_manager_handle,

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
			.add_listener::<directional_light::DirectionalLight>()
			.add_listener::<point_light::PointLight>()
	}

	fn load_material(&mut self, (response, buffer): (resource_management::Response, Vec<u8>),) {	
		let mut ghi = self.ghi.write().unwrap();

		for resource_document in &response.resources {
			match resource_document.class.as_str() {
				"Texture" => {
					let texture: &texture_resource_handler::Texture = resource_document.resource.downcast_ref().unwrap();

					let compression = if let Some(compression) = &texture.compression {
						match compression {
							texture_resource_handler::CompressionSchemes::BC7 => Some(ghi::CompressionSchemes::BC7)
						}
					} else {
						None
					};

					let new_texture = ghi.create_image(Some(&resource_document.url), texture.extent, ghi::Formats::RGBAu8, compression, ghi::Uses::Image | ghi::Uses::TransferDestination, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

					ghi.get_texture_slice_mut(new_texture).copy_from_slice(&buffer[resource_document.offset as usize..(resource_document.offset + resource_document.size) as usize]);
					
					let sampler = ghi.create_sampler(ghi::FilteringModes::Linear, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32); // TODO: use actual sampler

					ghi.write(&[
						ghi::DescriptorWrite {
							binding_handle: self.textures_binding,
							array_element: 0, // TODO: use actual array element
							descriptor: ghi::Descriptor::CombinedImageSampler { image_handle: new_texture, sampler_handle: sampler, layout: ghi::Layouts::Read },
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

					let stage = match shader.stage {
						resource_management::material_resource_handler::ShaderTypes::AnyHit => ghi::ShaderTypes::AnyHit,
						resource_management::material_resource_handler::ShaderTypes::ClosestHit => ghi::ShaderTypes::ClosestHit,
						resource_management::material_resource_handler::ShaderTypes::Compute => ghi::ShaderTypes::Compute,
						resource_management::material_resource_handler::ShaderTypes::Fragment => ghi::ShaderTypes::Fragment,
						resource_management::material_resource_handler::ShaderTypes::Intersection => ghi::ShaderTypes::Intersection,
						resource_management::material_resource_handler::ShaderTypes::Mesh => ghi::ShaderTypes::Mesh,
						resource_management::material_resource_handler::ShaderTypes::Miss => ghi::ShaderTypes::Miss,
						resource_management::material_resource_handler::ShaderTypes::RayGen => ghi::ShaderTypes::RayGen,
						resource_management::material_resource_handler::ShaderTypes::Callable => ghi::ShaderTypes::Callable,
						resource_management::material_resource_handler::ShaderTypes::Task => ghi::ShaderTypes::Task,
						resource_management::material_resource_handler::ShaderTypes::Vertex => ghi::ShaderTypes::Vertex,
					};

					let new_shader = ghi.create_shader(ghi::ShaderSource::SPIRV(&buffer[offset..(offset + size)]), stage, &[
						ghi::ShaderBindingDescriptor::new(0, 1, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 2, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 3, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 4, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 5, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 6, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(0, 7, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(1, 0, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(1, 1, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(1, 4, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(1, 6, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(2, 0, ghi::AccessPolicies::WRITE),
						ghi::ShaderBindingDescriptor::new(2, 1, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(2, 2, ghi::AccessPolicies::WRITE),
						ghi::ShaderBindingDescriptor::new(2, 3, ghi::AccessPolicies::WRITE),
						ghi::ShaderBindingDescriptor::new(2, 4, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(2, 5, ghi::AccessPolicies::READ),
						ghi::ShaderBindingDescriptor::new(2, 10, ghi::AccessPolicies::READ),
					]);

					self.shaders.insert(resource_id, (hash, new_shader, stage));
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

						let mut specialization_constants: Vec<Box<dyn ghi::SpecializationMapEntry>> = vec![];

						for (i, variable) in variant.variables.iter().enumerate() {
							// TODO: use actual variable type

							match variable.value.as_str() {
								"White" => {
									specialization_constants.push(
										Box::new(ghi::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [1f32, 1f32, 1f32, 1f32] })
									);
								}
								"Red" => {
									specialization_constants.push(
										Box::new(ghi::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [1f32, 0f32, 0f32, 1f32] })
									);
								}
								"Green" => {
									specialization_constants.push(
										Box::new(ghi::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [0f32, 1f32, 0f32, 1f32] })
									);
								}
								"Blue" => {
									specialization_constants.push(
										Box::new(ghi::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [0f32, 0f32, 1f32, 1f32] })
									);
								}
								"Purple" => {
									specialization_constants.push(
										Box::new(ghi::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [1f32, 0f32, 1f32, 1f32] })
									);
								}
								"Yellow" => {
									specialization_constants.push(
										Box::new(ghi::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [1f32, 1f32, 0f32, 1f32] })
									);
								}
								"Black" => {
									specialization_constants.push(
										Box::new(ghi::GenericSpecializationMapEntry{ constant_id: i as u32, r#type: "vec4f".to_string(), value: [0f32, 0f32, 0f32, 1f32] })
									);
								}
								_ => {
									error!("Unknown variant value: {}", variable.value);
								}
							}

						}

						let pipeline = ghi.create_compute_pipeline(&self.material_evaluation_pipeline_layout, (&shaders[0].0, ghi::ShaderTypes::Compute, specialization_constants));
						
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
						
						let materials_buffer_slice = ghi.get_mut_buffer_slice(self.materials_data_buffer_handle);

						let material_data = materials_buffer_slice.as_mut_ptr() as *mut MaterialData;

						let material_data = unsafe { material_data.as_mut().unwrap() };

						material_data.textures[0] = 0; // TODO: make dynamic based on supplied textures

						match material.model.name.as_str() {
							"Visibility" => {
								match material.model.pass.as_str() {
									"MaterialEvaluation" => {
										let pipeline = ghi.create_compute_pipeline(&self.material_evaluation_pipeline_layout, (&shaders[0].0, ghi::ShaderTypes::Compute, vec![Box::new(ghi::GenericSpecializationMapEntry{ constant_id: 0, r#type: "vec4f".to_string(), value: [0f32, 1f32, 0f32, 1f32] })]));
										
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
		let ghi = self.ghi.write().unwrap();

		let meshes_data_slice = ghi.get_mut_buffer_slice(self.meshes_data_buffer);

		let meshes_data = [
			value,
		];

		let meshes_data_bytes = unsafe { std::slice::from_raw_parts(meshes_data.as_ptr() as *const u8, std::mem::size_of_val(&meshes_data)) };

		unsafe {
			std::ptr::copy_nonoverlapping(meshes_data_bytes.as_ptr(), meshes_data_slice.as_mut_ptr().add(0 as usize * std::mem::size_of::<maths_rs::Mat4f>()), meshes_data_bytes.len());
		}
	}

	pub fn render(&mut self, ghi: &dyn ghi::GraphicsHardwareInterface, command_buffer_recording: &mut dyn ghi::CommandBufferRecording) {
		let camera_handle = if let Some(camera_handle) = &self.camera { camera_handle } else { return; };

		{
			let mut command_buffer_recording = ghi.create_command_buffer_recording(self.transfer_command_buffer, None);

			command_buffer_recording.transfer_textures(&self.pending_texture_loads);

			self.pending_texture_loads.clear();

			command_buffer_recording.execute(&[], &[], self.transfer_synchronizer);
		}

		ghi.wait(self.transfer_synchronizer); // Bad

		let camera_data_buffer = ghi.get_mut_buffer_slice(self.camera_data_buffer_handle);

		let (camera_position, camera_orientation) = camera_handle.get(|camera| (camera.get_position(), camera.get_orientation()));

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

		let attachments = [
			ghi::AttachmentInformation {
				image: self.primitive_index,
				layout: ghi::Layouts::RenderTarget,
				format: ghi::Formats::U32,
				clear: ghi::ClearValue::Integer(!0u32, 0, 0, 0),
				load: false,
				store: true,
			},
			ghi::AttachmentInformation {
				image: self.instance_id,
				layout: ghi::Layouts::RenderTarget,
				format: ghi::Formats::U32,
				clear: ghi::ClearValue::Integer(!0u32, 0, 0, 0),
				load: false,
				store: true,
			},
			ghi::AttachmentInformation {
				image: self.depth_target,
				layout: ghi::Layouts::RenderTarget,
				format: ghi::Formats::Depth32,
				clear: ghi::ClearValue::Depth(0f32),
				load: false,
				store: true,
			},
		];

		command_buffer_recording.start_region("Visibility Render Model");

		command_buffer_recording.start_region("Visibility Buffer");
		command_buffer_recording.bind_raster_pipeline(&self.visibility_pass_pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.pipeline_layout_handle, &[self.descriptor_set]);
		command_buffer_recording.start_render_pass(Extent::plane(1920, 1080), &attachments);
		command_buffer_recording.dispatch_meshes(self.visibility_info.meshlet_count, 1, 1);
		command_buffer_recording.end_render_pass();
		command_buffer_recording.end_region();

		command_buffer_recording.clear_buffers(&[self.material_count, self.material_offset, self.material_offset_scratch, self.material_evaluation_dispatches, self.material_xy]);

		command_buffer_recording.start_region("Material Count");
		command_buffer_recording.bind_compute_pipeline(&self.material_count_pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.visibility_pass_pipeline_layout, &[self.descriptor_set, self.visibility_passes_descriptor_set]);
		command_buffer_recording.dispatch(ghi::DispatchExtent { workgroup_extent: Extent::square(32), dispatch_extent: Extent::plane(1920, 1080) });
		command_buffer_recording.end_region();

		command_buffer_recording.start_region("Material Offset");
		command_buffer_recording.bind_compute_pipeline(&self.material_offset_pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.visibility_pass_pipeline_layout, &[self.descriptor_set, self.visibility_passes_descriptor_set]);
		command_buffer_recording.dispatch(ghi::DispatchExtent { workgroup_extent: Extent { width: 1, height: 1, depth: 1 }, dispatch_extent: Extent { width: 1, height: 1, depth: 1 } });
		command_buffer_recording.end_region();

		command_buffer_recording.start_region("Pixel Mapping");
		command_buffer_recording.bind_compute_pipeline(&self.pixel_mapping_pipeline);
		command_buffer_recording.bind_descriptor_sets(&self.visibility_pass_pipeline_layout, &[self.descriptor_set, self.visibility_passes_descriptor_set]);
		command_buffer_recording.dispatch(ghi::DispatchExtent { workgroup_extent: Extent::square(32), dispatch_extent: Extent { width: 1920, height: 1080, depth: 1 } });
		command_buffer_recording.end_region();

		command_buffer_recording.start_region("Material Evaluation");
		command_buffer_recording.clear_images(&[(self.albedo, ghi::ClearValue::Color(crate::RGBA::black())),(self.occlusion_map, ghi::ClearValue::Color(crate::RGBA::white()))]);
		for (_, (i, pipeline)) in self.material_evaluation_materials.iter() {
			// No need for sync here, as each thread across all invocations will write to a different pixel
			command_buffer_recording.bind_compute_pipeline(pipeline);
			command_buffer_recording.bind_descriptor_sets(&self.material_evaluation_pipeline_layout, &[self.descriptor_set, self.visibility_passes_descriptor_set, self.material_evaluation_descriptor_set]);
			command_buffer_recording.write_to_push_constant(&self.material_evaluation_pipeline_layout, 0, unsafe {
				std::slice::from_raw_parts(&(*i as u32) as *const u32 as *const u8, std::mem::size_of::<u32>())
			});
			command_buffer_recording.indirect_dispatch(&ghi::BufferDescriptor { buffer: self.material_evaluation_dispatches, offset: (*i as u64 * 12), range: 12, slot: 0 });
		}
		command_buffer_recording.end_region();

		// ghi.wait(self.transfer_synchronizer); // Wait for buffers to be copied over to the GPU, or else we might overwrite them on the CPU before they are copied over

		command_buffer_recording.end_region();
	}
}

impl orchestrator::EntitySubscriber<camera::Camera> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<camera::Camera>, camera: &camera::Camera) {
		self.camera = Some(handle);
	}

	fn on_update(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<camera::Camera>, params: &camera::Camera) {
		
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
		
		{
			let response_and_data = self.resource_manager.get(|resource_manager| {
				resource_manager.get(mesh.get_material_id()).unwrap()
			});

			self.load_material(response_and_data,);
		}

		if !self.mesh_resources.contains_key(mesh.get_resource_id()) { // Load only if not already loaded
			let mut ghi = self.ghi.write().unwrap();

			let resource_request = self.resource_manager.get(|resource_manager| {
				resource_manager.request_resource(mesh.get_resource_id())
			});

			let resource_request = if let Some(resource_info) = resource_request { resource_info } else { return; };

			let mut options = resource_management::Options { resources: Vec::new(), };

			let mut meshlet_stream_buffer = vec![0u8; 1024 * 8];

			for resource in &resource_request.resources {
				match resource.class.as_str() {
					"Mesh" => {
						let vertex_positions_buffer = ghi.get_mut_buffer_slice(self.vertex_positions_buffer);
						let vertex_normals_buffer = ghi.get_mut_buffer_slice(self.vertex_normals_buffer);
						let vertex_indices_buffer = ghi.get_mut_buffer_slice(self.vertex_indices_buffer);
						let primitive_indices_buffer = ghi.get_mut_buffer_slice(self.primitive_indices_buffer);
						let triangle_indices_buffer = ghi.get_mut_buffer_slice(self.triangle_indices_buffer);

						options.resources.push(resource_management::OptionResource {
							url: resource.url.clone(),
							streams: vec![
								resource_management::Stream{ buffer: &mut vertex_positions_buffer[(self.visibility_info.vertex_count as usize * std::mem::size_of::<Vector3>())..], name: "Vertex.Position".to_string() },
								resource_management::Stream{ buffer: &mut vertex_normals_buffer[(self.visibility_info.vertex_count as usize * std::mem::size_of::<Vector3>())..], name: "Vertex.Normal".to_string() },
								resource_management::Stream{ buffer: &mut triangle_indices_buffer[(self.visibility_info.triangle_count as usize * 3 * std::mem::size_of::<u16>())..], name: "TriangleIndices".to_string() },
								resource_management::Stream{ buffer: &mut vertex_indices_buffer[(self.visibility_info.vertex_count as usize * std::mem::size_of::<u16>())..], name: "VertexIndices".to_string() },
								resource_management::Stream{ buffer: &mut primitive_indices_buffer[(self.visibility_info.triangle_count as usize * 3 * std::mem::size_of::<u8>())..], name: "MeshletIndices".to_string() },
								resource_management::Stream{ buffer: meshlet_stream_buffer.as_mut_slice() , name: "Meshlets".to_string() },
							],
						});

						break;
					}
					_ => {}
				}
			}

			let resource = if let Ok(a) = self.resource_manager.get(|resource_manager| { resource_manager.load_resource(resource_request, Some(options), None) }) { a } else { return; };

			let (response, _buffer) = (resource.0, resource.1.unwrap());

			for resource in &response.resources {
				match resource.class.as_str() {
					"Mesh" => {
						self.mesh_resources.insert(mesh.get_resource_id(), self.visibility_info.triangle_count);

						let mesh_resource: &mesh_resource_handler::Mesh = resource.resource.downcast_ref().unwrap();

						let acceleration_structure = if false {
							let triangle_index_stream = mesh_resource.index_streams.iter().find(|is| is.stream_type == mesh_resource_handler::IndexStreamTypes::Triangles).unwrap();

							assert_eq!(triangle_index_stream.data_type, mesh_resource_handler::IntegralTypes::U16, "Triangle index stream is not u16");

							let bottom_level_acceleration_structure = ghi.create_bottom_level_acceleration_structure(&ghi::BottomLevelAccelerationStructure{
								description: ghi::BottomLevelAccelerationStructureDescriptions::Mesh {
									vertex_count: mesh_resource.vertex_count,
									vertex_position_encoding: ghi::Encodings::IEEE754,
									triangle_count: triangle_index_stream.count / 3,
									index_format: ghi::DataTypes::U16,
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

		let mut ghi = self.ghi.write().unwrap();

		let meshes_data_slice = ghi.get_mut_buffer_slice(self.meshes_data_buffer);

		let mesh_data = self.meshes.get(mesh.get_resource_id()).expect("Mesh not loaded");

		let shader_mesh_data = ShaderInstanceData {
			model: mesh.get_transform(),
			material_id: self.material_evaluation_materials.get(mesh.get_material_id()).unwrap().0,
			base_vertex_index: mesh_data.vertex_offset,
		};

		let meshes_data_slice = unsafe { std::slice::from_raw_parts_mut(meshes_data_slice.as_mut_ptr() as *mut ShaderInstanceData, MAX_INSTANCES) };

		meshes_data_slice[self.visibility_info.instance_count as usize] = shader_mesh_data;

		if let (Some(ray_tracing), Some(acceleration_structure)) = (Option::<RayTracing>::None, mesh_data.acceleration_structure) {
			let mesh_transform = mesh.get_transform();

			let transform = [
				[mesh_transform[0], mesh_transform[1], mesh_transform[2], mesh_transform[3]],
				[mesh_transform[4], mesh_transform[5], mesh_transform[6], mesh_transform[7]],
				[mesh_transform[8], mesh_transform[9], mesh_transform[10], mesh_transform[11]],
			];

			ghi.write_instance(ray_tracing.instances_buffer, self.visibility_info.instance_count as usize, transform, self.visibility_info.instance_count as u16, 0xFF, 0, acceleration_structure);
		}

		let meshlets_data_slice = ghi.get_mut_buffer_slice(self.meshlets_data_buffer);

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

	fn on_update(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<mesh::Mesh>, params: &mesh::Mesh) {
		
	}
}

impl orchestrator::EntitySubscriber<directional_light::DirectionalLight> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<directional_light::DirectionalLight>, light: &directional_light::DirectionalLight) {
		let mut ghi = self.ghi.write().unwrap();

		let lighting_data = unsafe { (ghi.get_mut_buffer_slice(self.light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };

		let light_index = lighting_data.count as usize;

		lighting_data.lights[light_index].position = crate::Vec3f::new(0.0, 2.0, 0.0);
		lighting_data.lights[light_index].color = light.color;
		
		lighting_data.count += 1;

		assert!(lighting_data.count < MAX_LIGHTS as u32, "Light count exceeded");
	}

	fn on_update(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<directional_light::DirectionalLight>, params: &directional_light::DirectionalLight) {
		
	}
}

impl orchestrator::EntitySubscriber<point_light::PointLight> for VisibilityWorldRenderDomain {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<point_light::PointLight>, light: &point_light::PointLight) {
		let mut ghi = self.ghi.write().unwrap();

		let lighting_data = unsafe { (ghi.get_mut_buffer_slice(self.light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };

		let light_index = lighting_data.count as usize;

		lighting_data.lights[light_index].position = light.position;
		lighting_data.lights[light_index].color = light.color;
		
		lighting_data.count += 1;

		assert!(lighting_data.count < MAX_LIGHTS as u32, "Light count exceeded");
	}

	fn on_update(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<point_light::PointLight>, params: &point_light::PointLight) {
		
	}
}

impl Entity for VisibilityWorldRenderDomain {}
impl System for VisibilityWorldRenderDomain {}

impl WorldRenderDomain for VisibilityWorldRenderDomain {
	fn get_descriptor_set_template(&self) -> ghi::DescriptorSetTemplateHandle {
		self.descriptor_set_layout
	}

	fn get_result_image(&self) -> ghi::ImageHandle {
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