use std::ops::DerefMut;
use std::rc::Rc;
use std::sync::Arc;

use ghi::{graphics_hardware_interface, ImageHandle};
use ghi::{GraphicsHardwareInterface, CommandBufferRecording, BoundComputePipelineMode, RasterizationRenderPassMode, BoundRasterizationPipelineMode};
use gxhash::{HashMap, HashMapExt};
use json::object;
use log::error;
use maths_rs::mat::{MatInverse, MatProjection, MatRotate3D};
use maths_rs::{prelude::MatTranslate, Mat4f};
use resource_management::asset::material_asset_handler::ProgramGenerator;
use resource_management::shader_generation::{ShaderGenerationSettings, ShaderGenerator};
use resource_management::Reference;
use resource_management::resource::{image_resource_handler, mesh_resource_handler};
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::types::{IndexStreamTypes, IntegralTypes, ShaderTypes};
use resource_management::image::Image as ResourceImage;
use resource_management::mesh::{Mesh as ResourceMesh, Primitive};
use resource_management::material::{Material as ResourceMaterial, Parameter, Shader, Value, VariantVariable};
use resource_management::material::Variant as ResourceVariant;
use utils::r#async::{join_all, OnceCell};
use utils::sync::RwLock;
use utils::{Extent, RGBA};

use crate::core::entity::EntityBuilder;
use crate::core::listener::{Listener, EntitySubscriber};
use crate::core::{self, Entity, EntityHandle};
use crate::rendering::common_shader_generator::CommonShaderGenerator;
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::shadow_render_pass::{self, ShadowRenderingPass};
use crate::rendering::texture_manager::TextureManager;
use crate::rendering::{directional_light, mesh, point_light, world_render_domain};
use crate::rendering::world_render_domain::{VisibilityInfo, WorldRenderDomain};
use crate::Vector2;
use crate::{resource_management::{self, }, core::orchestrator::{self, OrchestratorReference}, Vector3, camera::{self}, math};

#[derive(Debug, Clone)]
struct MeshPrimitive {
	/// The material index.
	material_index: u32,
	/// The meshlet count.
	meshlet_count: u32,
	/// The meshlet offset.
	/// The base position into the meshlets buffer relative to the primitive in the mesh
	meshlet_offset: u32,
	/// The vertex offset.
	/// The base position into the vertex buffer
	vertex_offset: u32,
	/// The primitive indices offset.
	/// The base position into the primitive indices buffer
	primitive_offset: u32,
	/// The triangle offset.
	/// The base position into the primitive indices buffer, to get the actual index this value has to be multiplied by 3
	triangle_offset: u32,
}

/// This structure hosts data analogous to the mesh resource's data.
/// It stores data relevant to the renderer which allows not to have to access/request the mesh resource.
#[derive(Debug, Clone)]
pub struct MeshData {
	// (material_id)
	primitives: Vec<MeshPrimitive>,
	/// The base position into the vertex buffer
	vertex_offset: u32,
	primitive_offset: u32,
	/// The base position into the primitive indices buffer, to get the actual index this value has to be multiplied by 3
	triangle_offset: u32,
	/// The meshlet offset.
	/// The base position into the meshlets buffer relative to the mesh
	meshlet_offset: u32,
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

enum RenderDescriptionVariants {
	Material {
		shaders: Vec<String>,
	},
	Variant {},
}

struct RenderDescription {
	index: u32,
	pipeline: ghi::PipelineHandle,
	name: String,
	alpha: bool,
	variant: RenderDescriptionVariants,
}

pub struct Instance {
	pub meshlet_count: u32,
}

struct RenderInfo {
	instances: Vec<Instance>,
}

/// This structure hosts data analogous to the image resource's data.
struct Image {
	/// This is the index of the image in the descriptor set.
	index: u32,
}

/// This the visibility buffer implementation of the world render domain.
pub struct VisibilityWorldRenderDomain {
	ghi: Rc<RwLock<ghi::GHI>>,

	resource_manager: EntityHandle<ResourceManager>,

	visibility_info: world_render_domain::VisibilityInfo,

	camera: Option<EntityHandle<crate::camera::Camera>>,

	render_entities: Vec<((usize, usize), EntityHandle<dyn mesh::RenderEntity>)>,

	meshes: HashMap<String, MeshData>,
	images: RwLock<HashMap<String, Image>>,

	texture_manager: Arc<utils::r#async::RwLock<TextureManager>>,
	pipeline_manager: PipelineManager,

	mesh_resources: HashMap<String, u32>,

	material_evaluation_materials: RwLock<HashMap<String, Arc<OnceCell<RenderDescription>>>>,

	occlusion_map: ghi::ImageHandle,

	transfer_synchronizer: ghi::SynchronizerHandle,
	transfer_command_buffer: ghi::CommandBufferHandle,

	// Visibility

	pipeline_layout_handle: ghi::PipelineLayoutHandle,

	vertex_positions_buffer: ghi::BaseBufferHandle,
	vertex_normals_buffer: ghi::BaseBufferHandle,
	vertex_uvs_buffer: ghi::BaseBufferHandle,

	/// Indices laid out as indices into the vertex buffers
	vertex_indices_buffer: ghi::BaseBufferHandle,
	/// Indices laid out as indices into the `vertex_indices_buffer`
	primitive_indices_buffer: ghi::BaseBufferHandle,

	albedo: ghi::ImageHandle,
	diffuse: ghi::ImageHandle,
	depth_target: ghi::ImageHandle,

	camera_data_buffer_handle: ghi::BaseBufferHandle,
	materials_data_buffer_handle: ghi::BaseBufferHandle,

	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	descriptor_set: ghi::DescriptorSetHandle,

	textures_binding: ghi::DescriptorSetBindingHandle,

	meshes_data_buffer: ghi::BaseBufferHandle,
	meshlets_data_buffer: ghi::BaseBufferHandle,

	visibility_pass_pipeline_layout: ghi::PipelineLayoutHandle,
	visibility_passes_descriptor_set: ghi::DescriptorSetHandle,

	instance_id: ghi::ImageHandle,
	primitive_index: ghi::ImageHandle,

	material_evaluation_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
	material_evaluation_pipeline_layout: ghi::PipelineLayoutHandle,

	light_data_buffer: ghi::BaseBufferHandle,

	visibility_pass: VisibilityPass,
	material_count_pass: MaterialCountPass,
	material_offset_pass: MaterialOffsetPass,
	pixel_mapping_pass: PixelMappingPass,

	shadow_render_pass: EntityHandle<ShadowRenderingPass>,

	shadow_map_binding: ghi::DescriptorSetBindingHandle,

	lights: Vec<LightData>,

	render_info: RenderInfo,
}

/* BASE */
pub const CAMERA_DATA_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH.union(ghi::Stages::FRAGMENT).union(ghi::Stages::RAYGEN).union(ghi::Stages::COMPUTE));
pub const MESH_DATA_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH.union(ghi::Stages::FRAGMENT).union(ghi::Stages::COMPUTE));
pub const VERTEX_POSITIONS_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH.union(ghi::Stages::COMPUTE));
pub const VERTEX_NORMALS_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(3, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH.union(ghi::Stages::COMPUTE));
pub const VERTEX_UV_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(5, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH.union(ghi::Stages::COMPUTE));
pub const VERTEX_INDICES_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(6, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH.union(ghi::Stages::COMPUTE));
pub const PRIMITIVE_INDICES_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(7, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH.union(ghi::Stages::COMPUTE));
pub const MESHLET_DATA_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(8, ghi::DescriptorType::StorageBuffer, ghi::Stages::MESH.union(ghi::Stages::COMPUTE));
pub const TEXTURES_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new_array(9, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE, 16);

/* Visibility */
pub const MATERIAL_COUNT_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub const MATERIAL_OFFSET_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub const MATERIAL_OFFSET_SCRATCH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub const MATERIAL_EVALUATION_DISPATCHES_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(3, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub const MATERIAL_XY_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(4, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
pub const TRIANGLE_INDEX_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(6, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const INSTANCE_ID_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(7, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

/* Material Evaluation */
pub const OUT_ALBEDO: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const CAMERA: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const LIGHTING_DATA: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(4, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const MATERIALS: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(5, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const AO: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(10, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
pub const DEPTH_SHADOW_MAP: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(11, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

impl VisibilityWorldRenderDomain {
	pub fn new<'a>(ghi: Rc<RwLock<ghi::GHI>>, resource_manager_handle: EntityHandle<ResourceManager>) -> EntityBuilder<'a, Self> {
		EntityBuilder::new_from_async_function(async move || {
			let mut ghi_instance = ghi.write();

			let extent = Extent::square(0);
			// let extent = Extent::rectangle(1920, 1080);

			let vertex_positions_buffer_handle = ghi_instance.create_buffer(Some("Visibility Vertex Positions Buffer"), std::mem::size_of::<[[f32; 3]; MAX_VERTICES]>(), ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
			let vertex_normals_buffer_handle = ghi_instance.create_buffer(Some("Visibility Vertex Normals Buffer"), std::mem::size_of::<[[f32; 3]; MAX_VERTICES]>(), ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
			let vertex_uv_buffer_handle = ghi_instance.create_buffer(Some("Visibility Vertex UV Buffer"), std::mem::size_of::<[[f32; 2]; MAX_VERTICES]>(), ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
			// let triangle_indices_buffer_handle = ghi_instance.create_buffer(Some("Visibility Triangle Indices Buffer"), std::mem::size_of::<[[u16; 3]; MAX_TRIANGLES]>(), ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
			let vertex_indices_buffer_handle = ghi_instance.create_buffer(Some("Visibility Index Buffer"), std::mem::size_of::<[[u8; 3]; MAX_TRIANGLES]>(), ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
			let primitive_indices_buffer_handle = ghi_instance.create_buffer(Some("Visibility Primitive Indices Buffer"), std::mem::size_of::<[[u16; 3]; MAX_PRIMITIVE_TRIANGLES]>(), ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
			let meshlets_data_buffer = ghi_instance.create_buffer(Some("Visibility Meshlets Data"), std::mem::size_of::<[ShaderMeshletData; MAX_MESHLETS]>(), ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

			let albedo = ghi_instance.create_image(Some("albedo"), extent, ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
			let diffuse = ghi_instance.create_image(Some("diffuse"), extent, ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
			let depth_target = ghi_instance.create_image(Some("depth_target"), extent, ghi::Formats::Depth32, ghi::Uses::DepthStencil | ghi::Uses::Image, ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

			let camera_data_buffer_handle = ghi_instance.create_buffer(Some("Visibility Camera Data"), std::mem::size_of::<[ShaderCameraData; 8]>(), ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

			let meshes_data_buffer = ghi_instance.create_buffer(Some("Visibility Meshes Data"), std::mem::size_of::<[ShaderMesh; MAX_INSTANCES]>(), ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

			let bindings = [
				CAMERA_DATA_BINDING,
				MESH_DATA_BINDING,
				VERTEX_POSITIONS_BINDING,
				VERTEX_NORMALS_BINDING,
				VERTEX_UV_BINDING,
				VERTEX_INDICES_BINDING,
				PRIMITIVE_INDICES_BINDING,
				MESHLET_DATA_BINDING,
				TEXTURES_BINDING,
			];

			let descriptor_set_layout = ghi_instance.create_descriptor_set_template(Some("Base Set Layout"), &bindings);

			let pipeline_layout_handle = ghi_instance.create_pipeline_layout(&[descriptor_set_layout], &[ghi::PushConstantRange::new(0, 4)]);

			let descriptor_set = ghi_instance.create_descriptor_set(Some("Base Descriptor Set"), &descriptor_set_layout);

			let camera_data_binding = ghi_instance.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&CAMERA_DATA_BINDING, camera_data_buffer_handle));
			let meshes_data_binding = ghi_instance.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&MESH_DATA_BINDING, meshes_data_buffer));
			let vertex_positions_binding = ghi_instance.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&VERTEX_POSITIONS_BINDING, vertex_positions_buffer_handle));
			let vertex_normals_binding = ghi_instance.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&VERTEX_NORMALS_BINDING, vertex_normals_buffer_handle));
			let vertex_uv_binding = ghi_instance.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&VERTEX_UV_BINDING, vertex_uv_buffer_handle));
			let vertex_indices_binding = ghi_instance.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&VERTEX_INDICES_BINDING, vertex_indices_buffer_handle));
			let primitive_indices_binding = ghi_instance.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&PRIMITIVE_INDICES_BINDING, primitive_indices_buffer_handle));
			let meshlets_data_binding = ghi_instance.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&MESHLET_DATA_BINDING, meshlets_data_buffer));
			let textures_binding = ghi_instance.create_descriptor_binding_array(descriptor_set, &TEXTURES_BINDING);

			let primitive_index = ghi_instance.create_image(Some("primitive index"), extent, ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Storage, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
			let instance_id = ghi_instance.create_image(Some("instance_id"), extent, ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Storage, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

			let bindings = [
				MATERIAL_COUNT_BINDING,
				MATERIAL_OFFSET_BINDING,
				MATERIAL_OFFSET_SCRATCH_BINDING,
				MATERIAL_EVALUATION_DISPATCHES_BINDING,
				MATERIAL_XY_BINDING,
				TRIANGLE_INDEX_BINDING,
				INSTANCE_ID_BINDING,
			];

			let visibility_descriptor_set_layout = ghi_instance.create_descriptor_set_template(Some("Visibility Set Layout"), &bindings);
			let visibility_pass_pipeline_layout = ghi_instance.create_pipeline_layout(&[descriptor_set_layout, visibility_descriptor_set_layout], &[]);
			let visibility_passes_descriptor_set = ghi_instance.create_descriptor_set(Some("Visibility Descriptor Set"), &visibility_descriptor_set_layout);

			let visibility_pass = VisibilityPass::new(ghi_instance.deref_mut(), pipeline_layout_handle, descriptor_set, primitive_index, instance_id, depth_target);
			let material_count_pass = MaterialCountPass::new(ghi_instance.deref_mut(), visibility_pass_pipeline_layout, descriptor_set, visibility_passes_descriptor_set, &visibility_pass);
			let material_offset_pass = MaterialOffsetPass::new(ghi_instance.deref_mut(), visibility_pass_pipeline_layout, descriptor_set, visibility_passes_descriptor_set);
			let pixel_mapping_pass = PixelMappingPass::new(ghi_instance.deref_mut(), visibility_pass_pipeline_layout, descriptor_set, visibility_passes_descriptor_set,);

			let material_count_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::buffer(&MATERIAL_COUNT_BINDING, material_count_pass.get_material_count_buffer()));
			let material_offset_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::buffer(&MATERIAL_OFFSET_BINDING, material_offset_pass.get_material_offset_buffer()));
			let material_offset_scratch_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::buffer(&MATERIAL_OFFSET_SCRATCH_BINDING, material_offset_pass.get_material_offset_scratch_buffer()));
			let material_evaluation_dispatches_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::buffer(&MATERIAL_EVALUATION_DISPATCHES_BINDING, material_offset_pass.material_evaluation_dispatches));
			let material_xy_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::buffer(&MATERIAL_XY_BINDING, pixel_mapping_pass.material_xy));
			let vertex_id_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::image(&TRIANGLE_INDEX_BINDING, primitive_index, ghi::Layouts::General));
			let instance_id_binding = ghi_instance.create_descriptor_binding(visibility_passes_descriptor_set, ghi::BindingConstructor::image(&INSTANCE_ID_BINDING, instance_id, ghi::Layouts::General));

			let light_data_buffer = ghi_instance.create_buffer(Some("Light Data"), std::mem::size_of::<LightingData>(), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

			let lighting_data = unsafe { (ghi_instance.get_mut_buffer_slice(light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };

			lighting_data.count = 0; // Initially, no lights

			let materials_data_buffer_handle = ghi_instance.create_buffer(Some("Materials Data"), std::mem::size_of::<[MaterialData; MAX_MATERIALS]>(), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

			let bindings = [
				ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
				ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
				ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
				ghi::DescriptorSetBindingTemplate::new(3, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
				ghi::DescriptorSetBindingTemplate::new(4, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
				ghi::DescriptorSetBindingTemplate::new(5, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
				ghi::DescriptorSetBindingTemplate::new(10, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE),
				ghi::DescriptorSetBindingTemplate::new(11, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE),
			];

			let shadow_render_pass = core::spawn(ShadowRenderingPass::new(ghi_instance.deref_mut(), &descriptor_set_layout, &depth_target)).await;

			let sampler = ghi_instance.create_sampler(ghi::FilteringModes::Linear, ghi::SamplingReductionModes::WeightedAverage, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32);
			let occlusion_map = ghi_instance.create_image(Some("Occlusion Map"), extent, ghi::Formats::R8(ghi::Encodings::UnsignedNormalized), ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

			let shadow_map_image = {
				shadow_render_pass.read_sync().get_shadow_map_image()
			};

			let material_evaluation_descriptor_set_layout = ghi_instance.create_descriptor_set_template(Some("Material Evaluation Set Layout"), &bindings);
			let material_evaluation_descriptor_set = ghi_instance.create_descriptor_set(Some("Material Evaluation Descriptor Set"), &material_evaluation_descriptor_set_layout);

			let albedo_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::image(&bindings[0], albedo, ghi::Layouts::General));
			let camera_data_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::buffer(&bindings[1], camera_data_buffer_handle));
			let diffuse_target_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::image(&bindings[2], diffuse, ghi::Layouts::General));
			let light_data_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::buffer(&bindings[4], light_data_buffer));
			let materials_data_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::buffer(&bindings[5], materials_data_buffer_handle));
			let occlussion_texture_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&bindings[6], occlusion_map, sampler, ghi::Layouts::Read));
			let shadow_map_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, ghi::BindingConstructor::combined_image_sampler(&bindings[7], shadow_map_image, sampler, ghi::Layouts::Read));

			let material_evaluation_pipeline_layout = ghi_instance.create_pipeline_layout(&[descriptor_set_layout, visibility_descriptor_set_layout, material_evaluation_descriptor_set_layout], &[ghi::PushConstantRange{ offset: 0, size: 4 }]);

			let transfer_synchronizer = ghi_instance.create_synchronizer(Some("Transfer Synchronizer"), false);
			let transfer_command_buffer = ghi_instance.create_command_buffer(Some("Transfer"));

			drop(ghi_instance);

			Self {
				ghi,

				render_entities: Vec::with_capacity(512),

				resource_manager: resource_manager_handle,

				visibility_info:  VisibilityInfo{ triangle_count: 0, instance_count: 0, meshlet_count:0, vertex_count:0, primitives_count: 0, },

				visibility_pass,
				material_count_pass,
				material_offset_pass,
				pixel_mapping_pass,

				shadow_render_pass,

				camera: None,

				meshes: HashMap::with_capacity(1024),
				images: RwLock::new(HashMap::with_capacity(1024)),

				texture_manager: Arc::new(utils::r#async::RwLock::new(TextureManager::new())),
				pipeline_manager: PipelineManager::new(),

				mesh_resources: HashMap::new(),

				material_evaluation_materials: RwLock::new(HashMap::new()),

				occlusion_map,

				transfer_synchronizer,
				transfer_command_buffer,

				// Visibility

				pipeline_layout_handle,

				vertex_positions_buffer: vertex_positions_buffer_handle,
				vertex_normals_buffer: vertex_normals_buffer_handle,
				vertex_uvs_buffer: vertex_uv_buffer_handle,

				vertex_indices_buffer: vertex_indices_buffer_handle,
				primitive_indices_buffer: primitive_indices_buffer_handle,

				descriptor_set_layout,
				descriptor_set,

				textures_binding,

				albedo,
				diffuse,
				depth_target,

				camera_data_buffer_handle,

				meshes_data_buffer,
				meshlets_data_buffer,

				visibility_pass_pipeline_layout,
				visibility_passes_descriptor_set,

				material_evaluation_descriptor_set_layout,
				material_evaluation_descriptor_set,
				material_evaluation_pipeline_layout,

				primitive_index,
				instance_id,

				light_data_buffer,
				materials_data_buffer_handle,

				shadow_map_binding,

				lights: Vec::new(),

				render_info: RenderInfo { instances: Vec::with_capacity(4096) }
			}
		})
			.listen_to::<camera::Camera>()
			.listen_to::<directional_light::DirectionalLight>()
			.listen_to::<point_light::PointLight>()
			.listen_to::<dyn mesh::RenderEntity>()
	}

	/// Creates the needed GHI resource for the given mesh.
	/// Does nothing if the mesh has already been loaded.
	async fn create_mesh_resources<'a, 's: 'a>(&'s mut self, id: &'a str) -> Result<&'a str, ()> {
		if let Some(entry) = self.meshes.get(id) {
			return Ok(id);
		}

		let mut meshlet_stream_buffer = vec![0u8; 1024 * 8];

		let mut resource_request: Reference<ResourceMesh> = {
			let resource_manager = self.resource_manager.read().await;
			let resource_request: Reference<ResourceMesh> = if let Some(resource_info) = resource_manager.request(id).await { resource_info } else {
				log::error!("Failed to load mesh resource {}", id);
				return Err(());
			};
			resource_request
		};

		let vertex_positions_stream;
		let vertex_normals_stream;
		let vertex_uv_stream;
		let vertex_indices_stream;
		let primitive_indices_stream;
		let meshlet_stream;

		{
			let mesh_resource = resource_request.resource();

			if let Some(stream) = mesh_resource.position_stream() {
				vertex_positions_stream = stream;
			} else {
				log::error!("Mesh resource does not contain vertex position stream");
				return Err(());
			}

			if let Some(stream) = mesh_resource.normal_stream() {
				vertex_normals_stream = stream;
			} else {
				log::error!("Mesh resource does not contain vertex normal stream");
				return Err(());
			}

			if let Some(stream) = mesh_resource.uv_stream() {
				vertex_uv_stream = stream;
			} else {
				log::error!("Mesh resource does not contain vertex uv stream");
				return Err(());
			}

			if let Some(stream) = mesh_resource.vertex_indices_stream() {
				vertex_indices_stream = stream;
			} else {
				log::error!("Mesh resource does not contain vertex index stream");
				return Err(());
			}

			if let Some(stream) = mesh_resource.meshlet_indices_stream() {
				primitive_indices_stream = stream;
			} else {
				log::error!("Mesh resource does not contain meshlet index stream");
				return Err(());
			}

			if let Some(stream) = mesh_resource.meshlets_stream() {
				meshlet_stream = stream;
			} else {
				log::error!("Mesh resource does not contain meshlet stream");
				return Err(());
			}
		}

		let mut ghi = self.ghi.write();

		let mut vertex_positions_buffer = ghi.get_splitter(self.vertex_positions_buffer, self.visibility_info.vertex_count as usize * std::mem::size_of::<Vector3>());
		let mut vertex_normals_buffer = ghi.get_splitter(self.vertex_normals_buffer, self.visibility_info.vertex_count as usize * std::mem::size_of::<Vector3>());
		let mut vertex_uv_buffer = ghi.get_splitter(self.vertex_uvs_buffer, self.visibility_info.vertex_count as usize * std::mem::size_of::<Vector2>());
		let mut vertex_indices_buffer = ghi.get_splitter(self.vertex_indices_buffer, self.visibility_info.primitives_count as usize * std::mem::size_of::<u16>());
		let mut primitive_indices_buffer = ghi.get_splitter(self.primitive_indices_buffer, self.visibility_info.triangle_count as usize * 3 * std::mem::size_of::<u8>());

		drop(ghi);

		let mut buffer_allocator = utils::BufferAllocator::new(&mut meshlet_stream_buffer);

		assert_eq!(primitive_indices_stream.stride, 1, "Meshlet index stream is not u8");
		assert_eq!(vertex_indices_stream.stride, 2, "Vertex index stream is not u16");
		assert_eq!(meshlet_stream.stride, 2, "Meshlet stream stride is not of size 2");

		let streams = vec![
			resource_management::StreamMut::new("Vertex.Position", vertex_positions_buffer.take(vertex_positions_stream.size)),
			resource_management::StreamMut::new("Vertex.Normal", vertex_normals_buffer.take(vertex_normals_stream.size)),
			resource_management::StreamMut::new("Vertex.UV", vertex_uv_buffer.take(vertex_uv_stream.size)),
			resource_management::StreamMut::new("VertexIndices", vertex_indices_buffer.take(vertex_indices_stream.size)),
			resource_management::StreamMut::new("MeshletIndices", primitive_indices_buffer.take(primitive_indices_stream.size)),
			resource_management::StreamMut::new("Meshlets", buffer_allocator.take(meshlet_stream.size)),
		];

		let load_target = resource_request.load(streams.into()).await.unwrap();

		let Reference { resource: ResourceMesh { vertex_components, streams, primitives }, .. } = resource_request;

		let vcps = primitives.iter().scan(0, |state, p| {
			let offset = *state;
			*state += p.vertex_count;
			offset.into()
		}).collect::<Vec<_>>();

		self.mesh_resources.insert(id.to_string(), self.visibility_info.triangle_count);

		let acceleration_structure = if false {
			assert_eq!(primitive_indices_stream.stride, 2, "Triangle index stream is not u16");

			let index_format = match primitive_indices_stream.stride {
				2 => ghi::DataTypes::U16,
				4 => ghi::DataTypes::U32,
				_ => panic!("Unsupported index format"),
			};

			let mut ghi = self.ghi.write();

			let bottom_level_acceleration_structure = ghi.create_bottom_level_acceleration_structure(&ghi::BottomLevelAccelerationStructure{
				description: ghi::BottomLevelAccelerationStructureDescriptions::Mesh {
					vertex_count: vertex_positions_stream.count() as u32,
					vertex_position_encoding: ghi::Encodings::FloatingPoint,
					triangle_count: primitive_indices_stream.count() as u32 / 3,
					index_format,
				}
			});

			// ray_tracing.pending_meshes.push(MeshState::Build { mesh_handle: mesh.resource_id.to_string() });

			Some(bottom_level_acceleration_structure)
		} else {
			None
		};

		let acceleration_structure = None;

		let total_meshlet_count = meshlet_stream.count();

		struct Meshlet {
			primitive_count: u8,
			triangle_count: u8,
		}

		let meshlets_per_primitive = primitives.into_iter().zip(vcps.iter()).scan((0, 0, 0), |(mesh_primitive_counter, mesh_triangle_counter, mesh_meshlet_counter), (primitive, vcps)| {
			let vertex_offset = *vcps;
			let primitive_offset = *mesh_primitive_counter;
			let triangle_offset = *mesh_triangle_counter;
			let meshlet_offset = *mesh_meshlet_counter;

			let meshlets = if let Some(stream) = primitive.meshlet_stream() {
				let m = load_target.get_stream("Meshlets").unwrap();

				let meshlet_stream = unsafe {
					std::slice::from_raw_parts(m.buffer().as_ptr().byte_add(stream.offset) as *const Meshlet, stream.count())
				};

				meshlet_stream.iter().scan((0, 0), |(primitive_primitive_counter, primitive_triangle_counter), meshlet| {
					let meshlet_primitive_count = meshlet.primitive_count;
					let meshlet_triangle_count = meshlet.triangle_count;

					let primitive_offset = *primitive_primitive_counter as u16;
					let triangle_offset = *primitive_triangle_counter as u16;

					// Update vertex and triangle offsets per meshlet, relative to the primitive
					*primitive_primitive_counter += meshlet_primitive_count as u32;
					*primitive_triangle_counter += meshlet_triangle_count as u32;

					// Update vertex, triangle and meshlet offsets per meshlet, relative to the mesh
					*mesh_primitive_counter += meshlet_primitive_count as u32;
					*mesh_triangle_counter += meshlet_triangle_count as u32;
					*mesh_meshlet_counter += 1;

					ShaderMeshletData {
						primitive_offset,
						triangle_offset,
						primitive_count: meshlet_primitive_count,
						triangle_count: meshlet_triangle_count,
					}.into()
				}).collect::<Vec<_>>()
			} else {
				panic!();
			};

			(MeshPrimitive {
				material_index: 0,
				meshlet_count: meshlets.len() as u32,
				meshlet_offset,
				vertex_offset,
				primitive_offset,
				triangle_offset,
			},
			meshlets, primitive).into()
		});

		let meshlets_per_primitive: Vec<(MeshPrimitive, Vec<ShaderMeshletData>)> = join_all(meshlets_per_primitive.map(async |(mp, meshlets, primitive): (MeshPrimitive, Vec<ShaderMeshletData>, Primitive)| {
			let variant = self.create_variant_resources(primitive.material).await.unwrap();
			(
				MeshPrimitive {
					material_index: variant,
					..mp
				},
				meshlets,
			)
		})).await;

		let mut ghi = self.ghi.write();

		let meshlets_data_slice = ghi.get_mut_buffer_slice(self.meshlets_data_buffer);

		let meshlets_data_slice = unsafe { std::slice::from_raw_parts_mut(meshlets_data_slice.as_mut_ptr() as *mut ShaderMeshletData, MAX_MESHLETS) };

		for (i, (primitive, meshlets)) in meshlets_per_primitive.iter().enumerate() {
			for (j, meshlet) in meshlets.iter().enumerate() {
				meshlets_data_slice[self.visibility_info.meshlet_count as usize + primitive.meshlet_offset as usize + j] = *meshlet;
			}
		}

		let primitives = meshlets_per_primitive.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>();

		let meshlet_offset = self.visibility_info.meshlet_count;

		self.meshes.insert(id.to_string(), MeshData { vertex_offset: self.visibility_info.vertex_count, primitive_offset: self.visibility_info.primitives_count, triangle_offset: self.visibility_info.triangle_count, meshlet_offset, acceleration_structure, primitives });

		let vertex_count = vertex_positions_stream.count();
		let primitive_count = vertex_indices_stream.count();
		let triangle_count = primitive_indices_stream.count() / 3;

		self.visibility_info.vertex_count += vertex_count as u32;
		self.visibility_info.primitives_count += primitive_count as u32;
		self.visibility_info.triangle_count += triangle_count as u32;
		self.visibility_info.meshlet_count += total_meshlet_count as u32;

		return Ok(id);
	}

	async fn create_material_resources<'a>(&'a self, resource: &mut resource_management::Reference<ResourceMaterial>) -> Result<u32, ()> {
		let (index, v) = {
			let mut material_evaluation_materials = self.material_evaluation_materials.write();
			let i = material_evaluation_materials.len() as u32;
			(i, material_evaluation_materials.entry(resource.id().to_string()).or_insert_with(|| Arc::new(OnceCell::new())).clone())
		};

		let material = v.get_or_try_init(async || {
			let ghi = self.ghi.clone();
	
			let material_id = resource.id().to_string();
	
			let shader_names = resource.resource().shaders().iter().map(|shader| shader.id().to_string()).collect::<Vec<_>>();
	
			let parameters = &mut resource.resource_mut().parameters;
	
			let textures_indices = join_all(parameters.iter_mut().map(async |parameter: &mut Parameter| {
				match parameter.value {
					Value::Image(ref mut image) => {
						let texture_manager = self.texture_manager.clone();
						let mut texture_manager = texture_manager.write().await;
						texture_manager.load(image, ghi.clone()).await
					}
					_ => { None }
				}
			})).await;
	
			let textures_indices = textures_indices.into_iter().map(|v| {
				if let Some((name, image, sampler)) = v {
					let texture_index = {
						let mut images = self.images.write();
						let index = images.len() as u32;
						match images.entry(name) {
							std::collections::hash_map::Entry::Occupied(v) => {
								v.get().index
							}
							std::collections::hash_map::Entry::Vacant(v) => {
								v.insert(Image { index });
								index
							}
						}
					};
	
					let mut ghi = ghi.write();
					ghi.write(&[ghi::DescriptorWrite::combined_image_sampler_array(self.textures_binding, image, sampler, ghi::Layouts::Read, texture_index),]);
	
					Some(texture_index)
				} else {
					None
				}
			}).collect::<Vec<_>>();
	
			match resource.resource().model.name.as_str() {
				"Visibility" => {
					match resource.resource().model.pass.as_str() {
						"MaterialEvaluation" => {
							let pipeline_handle = self.pipeline_manager.load_material(&self.material_evaluation_pipeline_layout, &[
								CAMERA_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
								MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
								VERTEX_POSITIONS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
								VERTEX_NORMALS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
								VERTEX_UV_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
								VERTEX_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
								PRIMITIVE_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
								MESHLET_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
								TEXTURES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
								MATERIAL_COUNT_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
								MATERIAL_OFFSET_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
								MATERIAL_XY_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
								TRIANGLE_INDEX_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
								OUT_ALBEDO.into_shader_binding_descriptor(2, ghi::AccessPolicies::WRITE),
								CAMERA.into_shader_binding_descriptor(2, ghi::AccessPolicies::READ),
								LIGHTING_DATA.into_shader_binding_descriptor(2, ghi::AccessPolicies::READ),
								MATERIALS.into_shader_binding_descriptor(2, ghi::AccessPolicies::READ),
								AO.into_shader_binding_descriptor(2, ghi::AccessPolicies::READ),
								DEPTH_SHADOW_MAP.into_shader_binding_descriptor(2, ghi::AccessPolicies::READ),
							], resource, ghi.clone()).await.unwrap();
	
							let mut ghi = ghi.write();
	
							let materials_buffer_slice = ghi.get_mut_buffer_slice(self.materials_data_buffer_handle);

							println!("Index {} for material {}", index, resource.id());
					
							let material_data = materials_buffer_slice.as_mut_ptr() as *mut MaterialData;
					
							let material_data = unsafe { material_data.add(index as usize).as_mut().unwrap() };
					
							for (i, e) in textures_indices.iter().enumerate() {
								material_data.textures[i] = e.unwrap_or(0xFFFFFFFFu32) as u32;
							}

							Ok(RenderDescription {
								name: material_id,
								index,
								pipeline: pipeline_handle,
								alpha: false,
								variant: RenderDescriptionVariants::Material { shaders: shader_names },
							})
						}
						_ => {
							error!("Unknown material pass: {}", resource.resource().model.pass);
							Err(())
						}
					}
				}
				_ => {
					error!("Unknown material model");
					Err(())
				}
			}
		}).await?;

		return Ok(material.index);
	}

	/// Creates the needed GHI resource for the given material.
	/// Does nothing if the material has already been loaded.
	async fn create_variant_resources<'s, 'a>(&'s self, mut resource: resource_management::Reference<ResourceVariant>) -> Result<u32, ()> {
		let (index, v) = {
			let mut material_evaluation_materials = self.material_evaluation_materials.write();
			let i = material_evaluation_materials.len() as u32;
			(i, material_evaluation_materials.entry(resource.id().to_string()).or_insert_with(|| Arc::new(OnceCell::new())).clone())
		};

		let material = v.get_or_try_init(async || {
			let variant_id = resource.id().to_string();

			let ghi = self.ghi.clone();

			let specialization_constants: Vec<ghi::SpecializationMapEntry> = resource.resource_mut().variables.iter().enumerate().filter_map(|(i, variable)| {
				match &variable.value {
					Value::Scalar(scalar) => {
						ghi::SpecializationMapEntry::new(i as u32, "f32".to_string(), *scalar).into()
					}
					Value::Vector3(value) => {
						ghi::SpecializationMapEntry::new(i as u32, "vec3f".to_string(), *value).into()
					}
					Value::Vector4(value) => {
						ghi::SpecializationMapEntry::new(i as u32, "vec4f".to_string(), *value).into()
					}
					_ => { None }
				}
			}).collect();

			let pipeline = self.pipeline_manager.load_variant(&self.material_evaluation_pipeline_layout, &[
				CAMERA_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				VERTEX_POSITIONS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				VERTEX_NORMALS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				VERTEX_UV_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				VERTEX_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				PRIMITIVE_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				MESHLET_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				TEXTURES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				MATERIAL_COUNT_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
				MATERIAL_OFFSET_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
				MATERIAL_XY_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
				TRIANGLE_INDEX_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
				OUT_ALBEDO.into_shader_binding_descriptor(2, ghi::AccessPolicies::WRITE),
				CAMERA.into_shader_binding_descriptor(2, ghi::AccessPolicies::READ),
				LIGHTING_DATA.into_shader_binding_descriptor(2, ghi::AccessPolicies::READ),
				MATERIALS.into_shader_binding_descriptor(2, ghi::AccessPolicies::READ),
				AO.into_shader_binding_descriptor(2, ghi::AccessPolicies::READ),
				DEPTH_SHADOW_MAP.into_shader_binding_descriptor(2, ghi::AccessPolicies::READ),
			], &specialization_constants, &mut resource, ghi.clone()).await;

			let pipeline = pipeline.unwrap();

			let variant = resource.resource_mut();

			let material_id = variant.material.id().to_string();

			self.create_material_resources(&mut variant.material).await?;	

			let textures_indices = {
				let ghi = ghi.clone();
				let texture_manager = self.texture_manager.clone();
				join_all(variant.variables.iter_mut().map(async |parameter: &mut VariantVariable| {
					match parameter.value {
						Value::Image(ref mut image) => {
							texture_manager.write().await.load(image, ghi.clone()).await
						}
						_ => { None }
					}
				})).await
			};

			let textures_indices = textures_indices.into_iter().map(|v| {
				if let Some((name, image, sampler)) = v {
					let texture_index = {
						let mut images = self.images.write();
						let index = images.len() as u32;
						match images.entry(name) {
							std::collections::hash_map::Entry::Occupied(v) => {
								v.get().index
							}
							std::collections::hash_map::Entry::Vacant(v) => {
								v.insert(Image { index });
								index
							}
						}
					};

					let mut ghi = ghi.write();
					ghi.write(&[ghi::DescriptorWrite::combined_image_sampler_array(self.textures_binding, image, sampler, ghi::Layouts::Read, texture_index),]);

					Some(texture_index)
				} else {
					None
				}
			}).collect::<Vec<_>>();

			let alpha = variant.alpha_mode == resource_management::types::AlphaMode::Blend;

			{
				let mut ghi = ghi.write();
		
				let materials_buffer_slice = ghi.get_mut_buffer_slice(self.materials_data_buffer_handle);
		
				let material_data = materials_buffer_slice.as_mut_ptr() as *mut MaterialData;

				println!("Index {} for material variant {}", index, resource.id());
		
				let material_data = unsafe { material_data.add(index as usize).as_mut().unwrap() };
		
				for (i, e) in textures_indices.iter().enumerate() {
					material_data.textures[i] = e.unwrap_or(0xFFFFFFFFu32) as u32;
				}

				Ok(RenderDescription {
					name: variant_id,
					index,
					pipeline,
					alpha,
					variant: RenderDescriptionVariants::Variant {  },
				})
			}
		}).await?;

		return Ok(material.index);
	}

	fn get_transform(&self) -> Mat4f { Mat4f::identity() }
	fn set_transform(&mut self, orchestrator: OrchestratorReference, value: Mat4f) {
		let mut ghi = self.ghi.write();

		let meshes_data_slice = ghi.get_mut_buffer_slice(self.meshes_data_buffer);

		let meshes_data = [
			value,
		];

		let meshes_data_bytes = unsafe { std::slice::from_raw_parts(meshes_data.as_ptr() as *const u8, std::mem::size_of_val(&meshes_data)) };

		unsafe {
			std::ptr::copy_nonoverlapping(meshes_data_bytes.as_ptr(), meshes_data_slice.as_mut_ptr().add(0 as usize * std::mem::size_of::<maths_rs::Mat4f>()), meshes_data_bytes.len());
		}
	}

	pub fn prepare(&mut self, ghi: &mut ghi::GHI, extent: Extent, modulo_frame_index: u32) -> Option<()> {
		let camera_handle = if let Some(camera_handle) = &self.camera { camera_handle } else { return None; };

		let camera_data_buffer = ghi.get_mut_buffer_slice(self.camera_data_buffer_handle);

		let (camera_position, camera_orientation, fov_y) = camera_handle.map(|camera| { let camera = camera.read_sync(); (camera.get_position(), camera.get_orientation(), camera.get_fov()) });

		let aspect_ratio = extent.width() as f32 / extent.height() as f32;

		let view_matrix = maths_rs::Mat4f::from_translation(-camera_position) * math::look_at(camera_orientation);
		let projection_matrix = math::projection_matrix(fov_y, aspect_ratio, 0.1f32, 100f32);
		let view_projection_matrix = projection_matrix * view_matrix;
		let fov = {
			let fov_x = 2f32 * ((fov_y / 2f32).to_radians().tan() * aspect_ratio).atan();
			let fov_y = fov_y.to_radians();
			[fov_x, fov_y]
		};

		let camera_data_reference = unsafe { (camera_data_buffer.as_mut_ptr() as *mut ShaderCameraData).as_mut().unwrap() };

		let camera = ShaderCameraData {
			view_matrix,
			projection_matrix,
			view_projection_matrix,
			inverse_view_matrix: math::inverse(view_matrix),
			inverse_projection_matrix: math::inverse(projection_matrix),
			inverse_view_projection_matrix: math::inverse(view_projection_matrix),
			fov,
		};

		*camera_data_reference = camera;

		{
			let meshes_data_slice = ghi.get_mut_buffer_slice(self.meshes_data_buffer);
			let meshes_data_slice = unsafe { std::slice::from_raw_parts_mut(meshes_data_slice.as_mut_ptr() as *mut ShaderMesh, MAX_INSTANCES) };

			for ((b, e), m) in self.render_entities.iter() {
				let mesh = m.write_sync();
				meshes_data_slice[*b..*e].iter_mut().for_each(|m| {
					m.model = mesh.get_transform();
				});
			}
		}

		{
			let _ = ghi.get_mut_buffer_slice(self.light_data_buffer); // Keep this here to trigger a copy
		}

		{
			let shadow_render_pass = self.shadow_render_pass.read_sync();

			let mut directional_lights: Vec<&LightData> = self.lights.iter().filter(|l| l.light_type == 'D' as u8).collect();
			directional_lights.sort_by(|a, b| maths_rs::length(a.color).partial_cmp(&maths_rs::length(b.color)).unwrap()); // Sort by intensity

			if let Some(most_significant_light) = directional_lights.get(0) {
				let normal = most_significant_light.view_matrix;

				shadow_render_pass.prepare(ghi, normal);
			}
		}

		Some(())
	}

	pub fn render_a(&mut self, command_buffer_recording: &mut impl ghi::CommandBufferRecording, extent: Extent, modulo_frame_index: u32) -> Option<()> {
		let camera_handle = if let Some(camera_handle) = &self.camera { camera_handle } else { return None; };

		command_buffer_recording.start_region("Visibility Render Model");

		self.visibility_pass.render(command_buffer_recording, &self.visibility_info, &self.render_info.instances, self.primitive_index, self.instance_id, self.depth_target, extent);
		self.material_count_pass.render(command_buffer_recording, extent);
		self.material_offset_pass.render(command_buffer_recording);
		self.pixel_mapping_pass.render(command_buffer_recording, extent);

		command_buffer_recording.end_region();

		Some(())
	}

	pub fn render_b(&mut self, command_buffer_recording: &mut impl ghi::CommandBufferRecording) {
		{
			let shadow_render_pass = self.shadow_render_pass.read_sync();

			let mut directional_lights: Vec<&LightData> = self.lights.iter().filter(|l| l.light_type == 'D' as u8).collect();
			directional_lights.sort_by(|a, b| maths_rs::length(a.color).partial_cmp(&maths_rs::length(b.color)).unwrap()); // Sort by intensity

			if true {
				if let Some(most_significant_light) = directional_lights.get(0) {
					shadow_render_pass.render(command_buffer_recording, self, &self.render_info.instances);
				} else {
					command_buffer_recording.clear_images(&[(shadow_render_pass.get_shadow_map_image(), ghi::ClearValue::Depth(1f32)),]);
				}
			} else {
				command_buffer_recording.clear_images(&[(shadow_render_pass.get_shadow_map_image(), ghi::ClearValue::Depth(0f32)),]);
			}
		}

		command_buffer_recording.start_region("Material Evaluation");
		command_buffer_recording.clear_images(&[(self.albedo, ghi::ClearValue::Color(RGBA::black())),]);

		command_buffer_recording.start_region("Opaque");

		let opaque_materials = self.material_evaluation_materials.read().values().filter_map(|v| v.get()).filter(|v| v.alpha == false).map(|v| (v.name.clone(), v.index, v.pipeline)).collect::<Vec<_>>();

		for (name, index, pipeline) in opaque_materials {
			command_buffer_recording.start_region(&format!("Material: {}", name));
			// No need for sync here, as each thread across all invocations will write to a different pixel
			let compute_pipeline_command = command_buffer_recording.bind_compute_pipeline(&pipeline);
			compute_pipeline_command.bind_descriptor_sets(&self.material_evaluation_pipeline_layout, &[self.descriptor_set, self.visibility_passes_descriptor_set, self.material_evaluation_descriptor_set]);
			compute_pipeline_command.write_push_constant(&self.material_evaluation_pipeline_layout, 0, index);
			compute_pipeline_command.indirect_dispatch(&self.material_offset_pass.material_evaluation_dispatches, index as usize);
			command_buffer_recording.end_region();
		}

		command_buffer_recording.end_region();

		command_buffer_recording.start_region("Transparent");

		let transparent_materials = self.material_evaluation_materials.read().values().filter_map(|v| v.get()).filter(|v| v.alpha == true).map(|v| (v.name.clone(), v.index, v.pipeline)).collect::<Vec<_>>();

		for (name, index, pipeline) in transparent_materials { // TODO: sort by distance to camera
			command_buffer_recording.start_region(&format!("Material: {}", name));
			// No need for sync here, as each thread across all invocations will write to a different pixel
			let compute_pipeline_command = command_buffer_recording.bind_compute_pipeline(&pipeline);
			compute_pipeline_command.bind_descriptor_sets(&self.material_evaluation_pipeline_layout, &[self.descriptor_set, self.visibility_passes_descriptor_set, self.material_evaluation_descriptor_set]);
			compute_pipeline_command.write_push_constant(&self.material_evaluation_pipeline_layout, 0, index);
			compute_pipeline_command.indirect_dispatch(&self.material_offset_pass.material_evaluation_dispatches, index as usize);
			command_buffer_recording.end_region();
		}

		command_buffer_recording.end_region();

		command_buffer_recording.end_region();
	}

	pub fn resize(&self, extent: Extent) {
		let mut ghi = self.ghi.write();
		ghi.resize_image(self.albedo, extent);
		ghi.resize_image(self.diffuse, extent);
		ghi.resize_image(self.depth_target, extent);
		ghi.resize_image(self.occlusion_map, extent);
		ghi.resize_image(self.primitive_index, extent);
		ghi.resize_image(self.instance_id, extent);

		self.pixel_mapping_pass.resize(extent, ghi.deref_mut());
	}

	pub fn get_transfer_synchronizer(&self) -> ghi::SynchronizerHandle {
		self.transfer_synchronizer
	}
}

impl EntitySubscriber<camera::Camera> for VisibilityWorldRenderDomain {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<camera::Camera>, camera: &camera::Camera) -> utils::BoxedFuture<()> {
		self.camera = Some(handle);
		Box::pin(async move {})
	}
}

#[derive(Copy, Clone)]
#[repr(C)]
struct ShaderMeshletData {
	/// Base index into the vertex indices buffer
	/// ```glsl
	/// vertex_index = mesh.base_vertex_index + vertex_indices[meshlet.vertex_offset + gl_LocalInvocationID.x];
	/// ```
	primitive_offset: u16,
	/// Base index into the primitive/triangle indices buffer
	/// This is stored as index / 3, as the meshlet contains 3 indices per triangle
	/// ```glsl
	/// triangle_index = primitive_indices.primitive_indices[(meshlet.triangle_offset + gl_LocalInvocationID.x) * 3 + 0..2]
	/// ```
	triangle_offset: u16,
	// The number of primitives in the meshlet
	primitive_count: u8,
	// The number of triangles in the meshlet
	triangle_count: u8,
}

#[repr(C)]
struct ShaderMesh {
	model: Mat4f,
	material_index: u32,
	/// The position into the vertex components data (positions, normals, uvs, ..) buffer this instance's data starts
	/// Also, the position into the vertex indices buffer this instance's data starts
	base_vertex_index: u32,
	base_primitive_index: u32,
	base_triangle_index: u32,
	base_meshlet_index: u32,
}

#[repr(C)]
pub struct LightingData {
	pub count: u32,
	pub lights: [LightData; MAX_LIGHTS],
}

#[repr(C)]
#[derive(Copy, Clone)]
struct ShaderCameraData {
	view_matrix: maths_rs::Mat4f,
	projection_matrix: maths_rs::Mat4f,
	view_projection_matrix: maths_rs::Mat4f,
	inverse_view_matrix: maths_rs::Mat4f,
	inverse_projection_matrix: maths_rs::Mat4f,
	inverse_view_projection_matrix: maths_rs::Mat4f,
	fov: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LightData {
	pub view_matrix: Mat4f,
	pub projection_matrix: Mat4f,
	pub vp_matrix: Mat4f,
	pub position: Vector3,
	pub color: Vector3,
	pub light_type: u8,
}

#[repr(C)]
struct MaterialData {
	textures: [u32; 16],
}

impl EntitySubscriber<dyn mesh::RenderEntity> for VisibilityWorldRenderDomain {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<dyn mesh::RenderEntity>, mesh: &'a dyn mesh::RenderEntity) -> utils::BoxedFuture<'a, ()> {
		Box::pin(async move {
		let mesh_id = self.create_mesh_resources(mesh.get_resource_id()).await.unwrap().to_string(); // I had to use to_string here because I couldn't solve the lifetime issue

		let mut ghi = self.ghi.write();

		let meshes_data_slice = ghi.get_mut_buffer_slice(self.meshes_data_buffer);

		let mesh_data = self.meshes.get(&mesh_id).expect("Mesh not loaded");

		let meshes_data_slice = unsafe { std::slice::from_raw_parts_mut(meshes_data_slice.as_mut_ptr() as *mut ShaderMesh, MAX_INSTANCES) };

		self.render_info.instances.extend(mesh_data.primitives.iter().map(|p| Instance {
			meshlet_count: p.meshlet_count,
		}));

		let instance_base_index = self.visibility_info.instance_count as usize;

		for (i, p) in mesh_data.primitives.iter().enumerate() {
			let shader_mesh_data = ShaderMesh {
				model: mesh.get_transform(),
				material_index: p.material_index,
				base_vertex_index: mesh_data.vertex_offset + p.vertex_offset, // Add the mesh relative vertex offset and the primitive relative vertex offset to get the absolute vertex offset
				base_primitive_index: mesh_data.primitive_offset + p.primitive_offset, // Add the mesh relative primitive offset and the primitive relative primitive offset to get the absolute primitive offset
				base_triangle_index: mesh_data.triangle_offset + p.triangle_offset, // Add the mesh relative triangle offset and the primitive relative triangle offset to get the absolute triangle offset
				base_meshlet_index: mesh_data.meshlet_offset + p.meshlet_offset, // Add the mesh relative meshlet offset and the primitive relative meshlet offset to get the absolute meshlet offset
			};

			meshes_data_slice[instance_base_index as usize + i] = shader_mesh_data;
		}

		if let (Some(ray_tracing), Some(acceleration_structure)) = (Option::<RayTracing>::None, mesh_data.acceleration_structure) {
			let mesh_transform = mesh.get_transform();

			let transform = [
				[mesh_transform[0], mesh_transform[1], mesh_transform[2], mesh_transform[3]],
				[mesh_transform[4], mesh_transform[5], mesh_transform[6], mesh_transform[7]],
				[mesh_transform[8], mesh_transform[9], mesh_transform[10], mesh_transform[11]],
			];

			ghi.write_instance(ray_tracing.instances_buffer, self.visibility_info.instance_count as usize, transform, self.visibility_info.instance_count as u16, 0xFF, 0, acceleration_structure);
		}

		self.visibility_info.instance_count += mesh_data.primitives.len() as u32;

		self.render_entities.push(((instance_base_index, instance_base_index + mesh_data.primitives.len()), handle));

		assert!((self.visibility_info.meshlet_count as usize) < MAX_MESHLETS, "Meshlet count exceeded");
		assert!((self.visibility_info.instance_count as usize) < MAX_INSTANCES, "Instance count exceeded");
		assert!((self.visibility_info.vertex_count as usize) < MAX_VERTICES, "Vertex count exceeded");
		assert!((self.visibility_info.vertex_count as usize) < MAX_PRIMITIVE_TRIANGLES, "Primitive triangle count exceeded");
		assert!((self.visibility_info.triangle_count as usize) < MAX_TRIANGLES, "Triangle count exceeded");
		})
	}
}

impl EntitySubscriber<directional_light::DirectionalLight> for VisibilityWorldRenderDomain {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<directional_light::DirectionalLight>, light: &directional_light::DirectionalLight) -> utils::BoxedFuture<()> {
		let mut ghi = self.ghi.write();

		let lighting_data = unsafe { (ghi.get_mut_buffer_slice(self.light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };

		let light_index = lighting_data.count as usize;

		let x = 4f32;
		let light_projection_matrix = math::orthographic_matrix(x, x, -5f32, 5f32);

		let direction = light.direction;
		let light_view_matrix = math::from_normal(direction);

		let vp_matrix = light_projection_matrix * light_view_matrix;

		lighting_data.lights[light_index].light_type = 'D' as u8;
		lighting_data.lights[light_index].view_matrix = light_view_matrix;
		lighting_data.lights[light_index].projection_matrix = light_projection_matrix;
		lighting_data.lights[light_index].vp_matrix = vp_matrix;
		lighting_data.lights[light_index].position = light.direction;
		lighting_data.lights[light_index].color = light.color;

		lighting_data.count += 1;

		self.lights.push(lighting_data.lights[light_index]);

		assert!(lighting_data.count < MAX_LIGHTS as u32, "Light count exceeded");

		Box::pin(async move { })
	}
}

impl EntitySubscriber<point_light::PointLight> for VisibilityWorldRenderDomain {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<point_light::PointLight>, light: &point_light::PointLight) -> utils::BoxedFuture<()> {
		let mut ghi = self.ghi.write();

		let lighting_data = unsafe { (ghi.get_mut_buffer_slice(self.light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };

		let light_index = lighting_data.count as usize;

		lighting_data.lights[light_index].light_type = 'P' as u8;
		lighting_data.lights[light_index].view_matrix = maths_rs::Mat4f::identity();
		lighting_data.lights[light_index].projection_matrix = maths_rs::Mat4f::identity();
		lighting_data.lights[light_index].vp_matrix = maths_rs::Mat4f::identity();
		lighting_data.lights[light_index].position = light.position;
		lighting_data.lights[light_index].color = light.color;

		lighting_data.count += 1;

		assert!(lighting_data.count < MAX_LIGHTS as u32, "Light count exceeded");

		Box::pin(async move { })
	}
}

impl Entity for VisibilityWorldRenderDomain {}

impl WorldRenderDomain for VisibilityWorldRenderDomain {
	fn get_descriptor_set_template(&self) -> ghi::DescriptorSetTemplateHandle {
		self.descriptor_set_layout
	}

	fn get_descriptor_set(&self) -> ghi::DescriptorSetHandle {
		self.descriptor_set
	}

	fn get_result_image(&self) -> ghi::ImageHandle {
		self.albedo
	}

	fn get_view_depth_image(&self) -> ghi::ImageHandle {
		self.depth_target
	}

	fn get_view_occlusion_image(&self) -> ghi::ImageHandle {
		self.occlusion_map
	}

	fn get_visibility_info(&self) -> VisibilityInfo {
		self.visibility_info
	}
}

struct VisibilityPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_pipeline: ghi::PipelineHandle,
}

impl VisibilityPass {
	pub fn new(ghi_instance: &mut ghi::GHI, pipeline_layout_handle: ghi::PipelineLayoutHandle, descriptor_set: ghi::DescriptorSetHandle, primitive_index: ghi::ImageHandle, instance_id: ghi::ImageHandle, depth_target: ghi::ImageHandle) -> Self {
		let visibility_pass_mesh_shader = ghi_instance.create_shader(Some("Visibility Pass Mesh Shader"), ghi::ShaderSource::GLSL(get_visibility_pass_mesh_source()), ghi::ShaderTypes::Mesh,
			&[
				CAMERA_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				VERTEX_POSITIONS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				VERTEX_NORMALS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				// ghi::ShaderBindingDescriptor::new(0, 4, ghi::AccessPolicies::READ),
				VERTEX_UV_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				VERTEX_INDICES_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			]
		).expect("Failed to create shader");

		let visibility_pass_fragment_shader = ghi_instance.create_shader(Some("Visibility Pass Fragment Shader"), ghi::ShaderSource::GLSL(VISIBILITY_PASS_FRAGMENT_SOURCE.to_string()), ghi::ShaderTypes::Fragment, &[]).expect("Failed to create shader");

		let visibility_pass_shaders = &[
			ghi::ShaderParameter::new(&visibility_pass_mesh_shader, ghi::ShaderTypes::Mesh),
			ghi::ShaderParameter::new(&visibility_pass_fragment_shader, ghi::ShaderTypes::Fragment),
		];

		let attachments = [
			ghi::AttachmentInformation::new(primitive_index,ghi::Formats::U32,ghi::Layouts::RenderTarget,ghi::ClearValue::Integer(!0u32, 0, 0, 0),false,true,),
			ghi::AttachmentInformation::new(instance_id,ghi::Formats::U32,ghi::Layouts::RenderTarget,ghi::ClearValue::Integer(!0u32, 0, 0, 0),false,true,),
			ghi::AttachmentInformation::new(depth_target,ghi::Formats::Depth32,ghi::Layouts::RenderTarget,ghi::ClearValue::Depth(0f32),false,true,),
		];

		let vertex_layout = [
			ghi::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0),
			ghi::VertexElement::new("NORMAL", ghi::DataTypes::Float3, 1),
		];

		let visibility_pass_pipeline = ghi_instance.create_raster_pipeline(&[
			ghi::PipelineConfigurationBlocks::Layout { layout: &pipeline_layout_handle },
			ghi::PipelineConfigurationBlocks::Shaders { shaders: visibility_pass_shaders },
			ghi::PipelineConfigurationBlocks::RenderTargets { targets: &attachments },
		]);

		VisibilityPass {
			pipeline_layout: pipeline_layout_handle,
			descriptor_set,
			visibility_pass_pipeline,
		}
	}

	pub fn render(&self, command_buffer_recording: &mut impl ghi::CommandBufferRecording, visibility_info: &VisibilityInfo, instances: &[Instance], primitive_index: ghi::ImageHandle, instance_id: ghi::ImageHandle, depth_target: ghi::ImageHandle, extent: Extent) {
		command_buffer_recording.start_region("Visibility Buffer");

		let attachments = [
			ghi::AttachmentInformation::new(primitive_index,ghi::Formats::U32,ghi::Layouts::RenderTarget,ghi::ClearValue::Integer(!0u32, 0, 0, 0),false,true,),
			ghi::AttachmentInformation::new(instance_id,ghi::Formats::U32,ghi::Layouts::RenderTarget,ghi::ClearValue::Integer(!0u32, 0, 0, 0),false,true,),
			ghi::AttachmentInformation::new(depth_target,ghi::Formats::Depth32,ghi::Layouts::RenderTarget,ghi::ClearValue::Depth(0f32),false,true,),
		];

		let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);
		render_pass_command.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
		let pipeline_bind = render_pass_command.bind_raster_pipeline(&self.visibility_pass_pipeline);

		for (i, instance) in instances.iter().enumerate() {
			pipeline_bind.write_push_constant(&self.pipeline_layout, 0, i as u32); // TODO: use actual instance indeces, not loaded meshes indices
			pipeline_bind.dispatch_meshes(instance.meshlet_count, 1, 1);
		}

		render_pass_command.end_render_pass();

		command_buffer_recording.end_region();
	}

	fn resize(&self, _: Extent) {}
}

struct MaterialCountPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	material_count_buffer: ghi::BaseBufferHandle,
	pipeline: ghi::PipelineHandle,
}

impl MaterialCountPass {
	fn new(ghi_instance: &mut ghi::GHI, pipeline_layout: ghi::PipelineLayoutHandle, descriptor_set: ghi::DescriptorSetHandle, visibility_pass_descriptor_set: ghi::DescriptorSetHandle, visibility_pass: &VisibilityPass) -> Self {
		let material_count_shader = ghi_instance.create_shader(Some("Material Count Pass Compute Shader"), ghi::ShaderSource::GLSL(get_material_count_source()), ghi::ShaderTypes::Compute,
			&[
				MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				MATERIAL_COUNT_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ | ghi::AccessPolicies::WRITE),
				INSTANCE_ID_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
			]
		).expect("Failed to create shader");

		let material_count_pipeline = ghi_instance.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&material_count_shader, ghi::ShaderTypes::Compute));

		let material_count_buffer = ghi_instance.create_buffer(Some("Material Count"), std::mem::size_of::<[u32; MAX_MATERIALS]>(), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

		MaterialCountPass {
			pipeline_layout,
			descriptor_set,
			material_count_buffer,
			visibility_pass_descriptor_set,
			pipeline: material_count_pipeline,
		}
	}

	fn render(&self, command_buffer_recording: &mut impl ghi::CommandBufferRecording, extent: Extent) {
		let pipeline_layout = self.pipeline_layout;
		let descriptor_set = self.descriptor_set;
		let visibility_pass_descriptor_set = self.visibility_pass_descriptor_set;
		let pipeline = self.pipeline;

		command_buffer_recording.start_region("Material Count");

		command_buffer_recording.clear_buffers(&[self.material_count_buffer]);

		command_buffer_recording.bind_descriptor_sets(&pipeline_layout, &[descriptor_set, visibility_pass_descriptor_set]);
		let compute_pipeline_command = command_buffer_recording.bind_compute_pipeline(&pipeline);
		compute_pipeline_command.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));

		command_buffer_recording.end_region();
	}

	fn get_material_count_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_count_buffer
    }
}

struct MaterialOffsetPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	material_offset_buffer: ghi::BaseBufferHandle,
	material_offset_scratch_buffer: ghi::BaseBufferHandle,
	material_evaluation_dispatches: ghi::BaseBufferHandle,
	material_offset_pipeline: ghi::PipelineHandle,
}

impl MaterialOffsetPass {
	fn new(ghi_instance: &mut ghi::GHI, pipeline_layout: ghi::PipelineLayoutHandle, descriptor_set: ghi::DescriptorSetHandle, visibility_pass_descriptor_set: ghi::DescriptorSetHandle) -> Self {
		let material_offset_shader = ghi_instance.create_shader(Some("Material Offset Pass Compute Shader"), ghi::ShaderSource::GLSL(get_material_offset_source()), ghi::ShaderTypes::Compute,
			&[
				MATERIAL_COUNT_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
				MATERIAL_OFFSET_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
				MATERIAL_OFFSET_SCRATCH_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
				MATERIAL_EVALUATION_DISPATCHES_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
			]
		).expect("Failed to create shader");

		let material_offset_pipeline = ghi_instance.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&material_offset_shader, ghi::ShaderTypes::Compute,));

		let material_evaluation_dispatches = ghi_instance.create_buffer(Some("Material Evaluation Dipatches"), std::mem::size_of::<[[u32; 3]; MAX_MATERIALS]>(), ghi::Uses::Storage | ghi::Uses::TransferDestination | ghi::Uses::Indirect, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
		let material_offset_buffer = ghi_instance.create_buffer(Some("Material Offset"), std::mem::size_of::<[u32; MAX_MATERIALS]>(), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
		let material_offset_scratch_buffer = ghi_instance.create_buffer(Some("Material Offset Scratch"), std::mem::size_of::<[u32; MAX_MATERIALS]>(), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

		MaterialOffsetPass {
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
			pipeline_layout,
			descriptor_set,
			visibility_pass_descriptor_set,
			material_offset_pipeline,
		}
	}

	fn render(&self, command_buffer_recording: &mut impl ghi::CommandBufferRecording) {
		let pipeline_layout = self.pipeline_layout;
		let descriptor_set = self.descriptor_set;
		let visibility_passes_descriptor_set = self.visibility_pass_descriptor_set;
		let pipeline = self.material_offset_pipeline;

		command_buffer_recording.start_region("Material Offset");

		command_buffer_recording.clear_buffers(&[self.material_offset_buffer, self.material_offset_scratch_buffer, self.material_evaluation_dispatches]);

		command_buffer_recording.bind_descriptor_sets(&pipeline_layout, &[descriptor_set, visibility_passes_descriptor_set]);
		let compute_pipeline_command = command_buffer_recording.bind_compute_pipeline(&pipeline);
		compute_pipeline_command.dispatch(ghi::DispatchExtent::new(Extent::line(1), Extent::line(1)));
		command_buffer_recording.end_region();
	}

	fn get_material_offset_buffer(&self) -> ghi::BaseBufferHandle {
        self.material_offset_buffer
    }

	fn get_material_offset_scratch_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_offset_scratch_buffer
	}
}

struct PixelMappingPass {
	material_xy: ghi::BaseBufferHandle,

	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_passes_descriptor_set: ghi::DescriptorSetHandle,
	pixel_mapping_pipeline: ghi::PipelineHandle,
}

impl PixelMappingPass {
	fn new(ghi_instance: &mut ghi::GHI, pipeline_layout: ghi::PipelineLayoutHandle, descriptor_set: ghi::DescriptorSetHandle, visibility_passes_descriptor_set: ghi::DescriptorSetHandle) -> Self {
		let pixel_mapping_shader = ghi_instance.create_shader(Some("Pixel Mapping Pass Compute Shader"), ghi::ShaderSource::GLSL(get_pixel_mapping_source()), ghi::ShaderTypes::Compute,
			&[
				MESH_DATA_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				MATERIAL_OFFSET_SCRATCH_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ | ghi::AccessPolicies::WRITE),
				INSTANCE_ID_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::READ),
				MATERIAL_XY_BINDING.into_shader_binding_descriptor(1, ghi::AccessPolicies::WRITE),
			]
		).expect("Failed to create shader");

		let pixel_mapping_pipeline = ghi_instance.create_compute_pipeline(&pipeline_layout, ghi::ShaderParameter::new(&pixel_mapping_shader, ghi::ShaderTypes::Compute,));

		let material_xy = ghi_instance.create_buffer(Some("Material XY"), 0, ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

		PixelMappingPass {
			material_xy,
			pipeline_layout,
			descriptor_set,
			visibility_passes_descriptor_set,
			pixel_mapping_pipeline,
		}
	}

	fn render(&self, command_buffer_recording: &mut impl ghi::CommandBufferRecording, extent: Extent) {
		let pipeline_layout = self.pipeline_layout;
		let descriptor_set = self.descriptor_set;
		let pipeline = self.pixel_mapping_pipeline;
		let visibility_passes_descriptor_set = self.visibility_passes_descriptor_set;

		command_buffer_recording.start_region("Pixel Mapping");

		command_buffer_recording.clear_buffers(&[self.material_xy,]);

		command_buffer_recording.bind_descriptor_sets(&pipeline_layout, &[descriptor_set, visibility_passes_descriptor_set]);
		let compute_pipeline_command = command_buffer_recording.bind_compute_pipeline(&pipeline);
		compute_pipeline_command.dispatch(ghi::DispatchExtent::new(extent, Extent::square(32)));

		command_buffer_recording.end_region();
	}

	fn resize(&self, extent: Extent, ghi: &mut ghi::GHI) {
		ghi.resize_buffer(self.material_xy, (extent.width() * extent.height() * 4) as usize);
	}
}

struct MaterialEvaluationPass {
}

impl MaterialEvaluationPass {
	fn new(ghi_instance: &mut ghi::GHI, visibility_pass: &VisibilityPass, material_count_pass: &MaterialCountPass, material_offset_pass: &MaterialOffsetPass, pixel_mapping_pass: &PixelMappingPass) -> Self {
		MaterialEvaluationPass {}
	}
}

const VERTEX_COUNT: u32 = 64;
const TRIANGLE_COUNT: u32 = 126;

const MAX_MESHLETS: usize = 1024 * 4;
const MAX_INSTANCES: usize = 1024;
const MAX_MATERIALS: usize = 1024;
const MAX_LIGHTS: usize = 16;
const MAX_TRIANGLES: usize = 65536 * 4;
const MAX_PRIMITIVE_TRIANGLES: usize = 65536 * 4;
const MAX_VERTICES: usize = 65536 * 4;

pub fn get_visibility_pass_mesh_source() -> String {
	let shader_generator = {
		let common_shader_generator = CommonShaderGenerator::new();
		common_shader_generator
	};

	let main_code = r#"
	Camera camera = camera.camera;
	process_meshlet(push_constant.instance_index, camera.view_projection);
	"#;

	let main = besl::parser::Node::function("main", Vec::new(), "void", vec![besl::parser::Node::glsl(main_code, &["camera", "push_constant", "process_meshlet"], Vec::new())]);

	let root_node = besl::parser::Node::root();

	let mut root = shader_generator.transform(root_node, &object! {});

	let push_constant = besl::parser::Node::push_constant(vec![besl::parser::Node::member("instance_index", "u32")]);

	root.add(vec![push_constant, main]);

	let root_node = besl::lex(root).unwrap();

	let main_node = root_node.borrow().get_main().unwrap();

	let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&ShaderGenerationSettings::mesh(64, 126, Extent::line(128)), &main_node);

	glsl
}

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

pub fn get_material_count_source() -> String {
	let shader_generator = {
		let common_shader_generator = CommonShaderGenerator::new_with_params(false, true, false, true, false, true, false, false);
		common_shader_generator
	};

	let main_code = r#"
	// If thread is out of bound respect to the material_id texture, return
	ivec2 extent = imageSize(instance_index_render_target);
	if (gl_GlobalInvocationID.x >= extent.x || gl_GlobalInvocationID.y >= extent.y) { return; }

	uint pixel_instance_index = imageLoad(instance_index_render_target, ivec2(gl_GlobalInvocationID.xy)).r;

	if (pixel_instance_index == 0xFFFFFFFF) { return; }

	uint material_index = meshes.meshes[pixel_instance_index].material_index;

	atomicAdd(material_count.material_count[material_index], 1);
	"#;

	let main = besl::parser::Node::function("main", Vec::new(), "void", vec![besl::parser::Node::glsl(main_code, &["meshes", "material_count", "instance_index_render_target"], Vec::new())]);

	let root_node = besl::parser::Node::root();

	let mut root = shader_generator.transform(root_node, &object! {});

	root.add(vec![main]);

	let root_node = besl::lex(root).unwrap();

	let main_node = root_node.borrow().get_main().unwrap();

	let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&ShaderGenerationSettings::compute(Extent::square(32)), &main_node);

	glsl
}

pub fn get_material_offset_source() -> String {
	let shader_generator = {
		let common_shader_generator = CommonShaderGenerator::new_with_params(true, false, false, true, false, true, false, false);
		common_shader_generator
	};

	let main_code = r#"
	uint sum = 0;

	for (uint i = 0; i < 1024; i++) { /* 1024 is the maximum number of materials */
		material_offset.material_offset[i] = sum;
		material_offset_scratch.material_offset_scratch[i] = sum;
		material_evaluation_dispatches.material_evaluation_dispatches[i] = uvec3((material_count.material_count[i] + 127) / 128, 1, 1);
		sum += material_count.material_count[i];
	}
	"#;

	let main = besl::parser::Node::function("main", Vec::new(), "void", vec![besl::parser::Node::glsl(main_code, &["material_offset", "material_offset_scratch", "material_count", "material_evaluation_dispatches",], Vec::new())]);

	let root_node = besl::parser::Node::root();

	let mut root = shader_generator.transform(root_node, &object! {});

	root.add(vec![main]);

	let root_node = besl::lex(root).unwrap();

	let main_node = root_node.borrow().get_main().unwrap();

	let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&ShaderGenerationSettings::compute(Extent::square(1)), &main_node);

	glsl
}

pub fn get_pixel_mapping_source() -> String {
	let shader_generator = {
		let common_shader_generator = CommonShaderGenerator::new_with_params(false, false, false, false, false, true, false, true);
		common_shader_generator
	};

	let main_code = r#"
	ivec2 extent = imageSize(instance_index_render_target);
	// If thread is out of bound respect to the material_id texture, return
	if (gl_GlobalInvocationID.x >= extent.x || gl_GlobalInvocationID.y >= extent.y) { return; }

	uint pixel_instance_index = imageLoad(instance_index_render_target, ivec2(gl_GlobalInvocationID.xy)).r;

	if (pixel_instance_index == 0xFFFFFFFF) { return; }

	uint material_index = meshes.meshes[pixel_instance_index].material_index;

	uint offset = atomicAdd(material_offset_scratch.material_offset_scratch[material_index], 1);

	pixel_mapping.pixel_mapping[offset] = u16vec2(gl_GlobalInvocationID.xy);
	"#;

	let main = besl::parser::Node::function("main", Vec::new(), "void", vec![besl::parser::Node::glsl(main_code, &["meshes", "material_offset_scratch", "pixel_mapping", "instance_index_render_target",], Vec::new())]);

	let root_node = besl::parser::Node::root();

	let mut root = shader_generator.transform(root_node, &object! {});

	root.add(vec![main]);

	let root_node = besl::lex(root).unwrap();

	let main_node = root_node.borrow().get_main().unwrap();

	let glsl = ShaderGenerator::new().compilation().generate_glsl_shader(&ShaderGenerationSettings::compute(Extent::square(32)), &main_node);

	glsl
}
