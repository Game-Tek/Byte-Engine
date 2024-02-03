use std::collections::HashMap;
use std::ops::DerefMut;
use std::rc::Rc;
use std::sync::RwLock;

use log::error;
use maths_rs::mat::{MatProjection, MatRotate3D};
use maths_rs::{prelude::MatTranslate, Mat4f};

use crate::core::entity::EntityBuilder;
use crate::core::listener::{Listener, EntitySubscriber};
use crate::core::{self, Entity, EntityHandle};
use crate::rendering::shadow_render_pass::{self, ShadowRenderingPass};
use crate::{ghi, utils, RGBA, shader_generator};
use crate::rendering::{mesh, directional_light, point_light};
use crate::rendering::world_render_domain::WorldRenderDomain;
use crate::resource_management::resource_manager::ResourceManager;
use crate::{resource_management::{self, mesh_resource_handler, material_resource_handler::{Shader, Material, Variant}, texture_resource_handler}, Extent, core::orchestrator::{self, OrchestratorReference}, Vector3, camera::{self}, math};

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
}

impl VisibilityWorldRenderDomain {
	pub fn new<'a>(listener: &'a impl Listener, ghi: Rc<RwLock<dyn ghi::GraphicsHardwareInterface>>, resource_manager_handle: EntityHandle<ResourceManager>) -> EntityBuilder<'a, Self> {
		EntityBuilder::new_from_function(move || {
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
			let diffuse;
			let depth_target;
			let camera_data_buffer_handle;
			let meshes_data_buffer;
			let meshlets_data_buffer;
			let visibility_descriptor_set_layout;
			let visibility_pass_pipeline_layout;
			let visibility_passes_descriptor_set;
			let material_evaluation_descriptor_set_layout;
			let material_evaluation_descriptor_set;
			let material_evaluation_pipeline_layout;
			let primitive_index;
			let instance_id;
			let light_data_buffer;
			let materials_data_buffer_handle;

			let visibility_pass;
			let material_count_pass;
			let material_offset_pass;
			let pixel_mapping_pass;

			let shadow_render_pass;

			let shadow_map_binding;

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

				albedo = ghi_instance.create_image(Some("albedo"), Extent::new(1920, 1080, 1), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), None, ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
				diffuse = ghi_instance.create_image(Some("diffuse"), Extent::new(1920, 1080, 1), ghi::Formats::RGBA16(ghi::Encodings::UnsignedNormalized), None, ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
				depth_target = ghi_instance.create_image(Some("depth_target"), Extent::new(1920, 1080, 1), ghi::Formats::Depth32, None, ghi::Uses::DepthStencil | ghi::Uses::Image, ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

				camera_data_buffer_handle = ghi_instance.create_buffer(Some("Visibility Camera Data"), 16 * 4 * 4, ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

				meshes_data_buffer = ghi_instance.create_buffer(Some("Visibility Meshes Data"), std::mem::size_of::<[ShaderInstanceData; MAX_INSTANCES]>(), ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);
				meshlets_data_buffer = ghi_instance.create_buffer(Some("Visibility Meshlets Data"), std::mem::size_of::<[ShaderMeshletData; MAX_MESHLETS]>(), ghi::Uses::Storage, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

				ghi_instance.write(&[
					ghi::DescriptorWrite::buffer(camera_data_binding, camera_data_buffer_handle),
					ghi::DescriptorWrite::buffer(meshes_data_binding, meshes_data_buffer),
					ghi::DescriptorWrite::buffer(vertex_positions_binding, vertex_positions_buffer_handle),
					ghi::DescriptorWrite::buffer(vertex_normals_binding, vertex_normals_buffer_handle),
					ghi::DescriptorWrite::buffer(vertex_indices_binding, vertex_indices_buffer_handle),
					ghi::DescriptorWrite::buffer(primitive_indices_binding, primitive_indices_buffer_handle),
					ghi::DescriptorWrite::buffer(meshlets_data_binding, meshlets_data_buffer),
				]);

				primitive_index = ghi_instance.create_image(Some("primitive index"), Extent::rectangle(1920, 1080), ghi::Formats::U32, None, ghi::Uses::RenderTarget | ghi::Uses::Storage, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);
				instance_id = ghi_instance.create_image(Some("instance_id"), Extent::rectangle(1920, 1080), ghi::Formats::U32, None, ghi::Uses::RenderTarget | ghi::Uses::Storage, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::DYNAMIC);

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

				visibility_pass = VisibilityPass::new(ghi_instance.deref_mut(), pipeline_layout_handle, descriptor_set, primitive_index, instance_id, depth_target);
				material_count_pass = MaterialCountPass::new(ghi_instance.deref_mut(), visibility_pass_pipeline_layout, descriptor_set, visibility_passes_descriptor_set, &visibility_pass);
				material_offset_pass = MaterialOffsetPass::new(ghi_instance.deref_mut(), visibility_pass_pipeline_layout, descriptor_set, visibility_passes_descriptor_set);
				pixel_mapping_pass = PixelMappingPass::new(ghi_instance.deref_mut(), visibility_pass_pipeline_layout, descriptor_set, visibility_passes_descriptor_set,);

				ghi_instance.write(&[
					ghi::DescriptorWrite::buffer(material_count_binding, material_count_pass.get_material_count_buffer()),
					ghi::DescriptorWrite::buffer(material_offset_binding, material_offset_pass.get_material_offset_buffer()),
					ghi::DescriptorWrite::buffer(material_offset_scratch_binding, material_offset_pass.get_material_offset_scratch_buffer()),
					ghi::DescriptorWrite::buffer(material_evaluation_dispatches_binding, material_offset_pass.material_evaluation_dispatches),
					ghi::DescriptorWrite::buffer(material_xy_binding, pixel_mapping_pass.material_xy),
					ghi::DescriptorWrite::image(vertex_id_binding, primitive_index, ghi::Layouts::General),
					ghi::DescriptorWrite::image(instance_id_binding, instance_id, ghi::Layouts::General),
				]);

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
					ghi::DescriptorSetBindingTemplate::new(11, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE),
				];

				material_evaluation_descriptor_set_layout = ghi_instance.create_descriptor_set_template(Some("Material Evaluation Set Layout"), &bindings);
				material_evaluation_descriptor_set = ghi_instance.create_descriptor_set(Some("Material Evaluation Descriptor Set"), &material_evaluation_descriptor_set_layout);

				let albedo_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[0]);
				let camera_data_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[1]);
				let diffuse_target_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[2]);
				let light_data_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[4]);
				let materials_data_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[5]);
				let occlussion_texture_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[6]);
				shadow_map_binding = ghi_instance.create_descriptor_binding(material_evaluation_descriptor_set, &bindings[7]);

				shadow_render_pass = core::spawn(ShadowRenderingPass::new(ghi_instance.deref_mut(), &descriptor_set_layout, &depth_target));

				let sampler = ghi_instance.create_sampler(ghi::FilteringModes::Linear, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32);
				occlusion_map = ghi_instance.create_image(Some("Occlusion Map"), Extent::new(1920, 1080, 1), ghi::Formats::R8(ghi::Encodings::UnsignedNormalized), None, ghi::Uses::Storage | ghi::Uses::Image, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

				let shadow_map_image = {
					shadow_render_pass.read_sync().get_shadow_map_image()
				};

				ghi_instance.write(&[
					ghi::DescriptorWrite::image(albedo_binding, albedo, ghi::Layouts::General),
					ghi::DescriptorWrite::image(diffuse_target_binding, diffuse, ghi::Layouts::General),
					ghi::DescriptorWrite::buffer(camera_data_binding, camera_data_buffer_handle,),
					ghi::DescriptorWrite::buffer(light_data_binding, light_data_buffer,),
					ghi::DescriptorWrite::buffer(materials_data_binding, materials_data_buffer_handle,),
					ghi::DescriptorWrite::combined_image_sampler(occlussion_texture_binding, occlusion_map, sampler, ghi::Layouts::Read),
					ghi::DescriptorWrite::combined_image_sampler(shadow_map_binding, shadow_map_image, sampler, ghi::Layouts::Read),
				]);

				material_evaluation_pipeline_layout = ghi_instance.create_pipeline_layout(&[descriptor_set_layout, visibility_descriptor_set_layout, material_evaluation_descriptor_set_layout], &[ghi::PushConstantRange{ offset: 0, size: 4 }]);

				transfer_synchronizer = ghi_instance.create_synchronizer(Some("Transfer Synchronizer"), false);
				transfer_command_buffer = ghi_instance.create_command_buffer(Some("Transfer"));
			}

			Self {
				ghi,

				resource_manager: resource_manager_handle,

				visibility_info:  VisibilityInfo{ triangle_count: 0, instance_count: 0, meshlet_count:0, vertex_count:0, },

				visibility_pass,
				material_count_pass,
				material_offset_pass,
				pixel_mapping_pass,

				shadow_render_pass,

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
			}
		})
			// .add_post_creation_function(Box::new(Self::load_needed_assets))
			.listen_to::<camera::Camera>(listener)
			.listen_to::<mesh::Mesh>(listener)
			.listen_to::<directional_light::DirectionalLight>(listener)
			.listen_to::<point_light::PointLight>(listener)
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

					let new_texture = ghi.create_image(Some(&resource_document.url), texture.extent, ghi::Formats::RGBA8(ghi::Encodings::UnsignedNormalized), compression, ghi::Uses::Image | ghi::Uses::TransferDestination, ghi::DeviceAccesses::CpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

					ghi.get_texture_slice_mut(new_texture).copy_from_slice(&buffer[resource_document.offset as usize..(resource_document.offset + resource_document.size) as usize]);
					
					let sampler = ghi.create_sampler(ghi::FilteringModes::Linear, ghi::FilteringModes::Linear, ghi::SamplerAddressingModes::Clamp, None, 0f32, 0f32); // TODO: use actual sampler

					ghi.write(&[ghi::DescriptorWrite::combined_image_sampler(self.textures_binding, new_texture, sampler, ghi::Layouts::Read),]);

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

					let new_shader = ghi.create_shader(Some(&resource_document.url), ghi::ShaderSource::SPIRV(&buffer[offset..(offset + size)]), stage, &[
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
						ghi::ShaderBindingDescriptor::new(2, 11, ghi::AccessPolicies::READ),
					]).expect("Failed to create shader");

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

						let mut specialization_constants: Vec<ghi::SpecializationMapEntry> = vec![];

						for (i, variable) in variant.variables.iter().enumerate() {
							// TODO: use actual variable type

							let value = match variable.value.as_str() {
								"White" => { [1f32, 1f32, 1f32, 1f32] }
								"Red" => { [1f32, 0f32, 0f32, 1f32] }
								"Green" => { [0f32, 1f32, 0f32, 1f32] }
								"Blue" => { [0f32, 0f32, 1f32, 1f32] }
								"Purple" => { [1f32, 0f32, 1f32, 1f32] }
								"Yellow" => { [1f32, 1f32, 0f32, 1f32] }
								"Black" => { [0f32, 0f32, 0f32, 1f32] }
								_ => {
									error!("Unknown variant value: {}", variable.value);

									[0f32, 0f32, 0f32, 1f32]
								}
							};

							specialization_constants.push(ghi::SpecializationMapEntry::new(i as u32, "vec4f".to_string(), value));
						}

						let pipeline = ghi.create_compute_pipeline(&self.material_evaluation_pipeline_layout, (&shaders[0].0, ghi::ShaderTypes::Compute, &specialization_constants));
						
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
										let pipeline = ghi.create_compute_pipeline(&self.material_evaluation_pipeline_layout, (&shaders[0].0, ghi::ShaderTypes::Compute, &[ghi::SpecializationMapEntry::new(0, "vec4f".to_string(), [0f32, 1f32, 0f32, 1f32])]));
										
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

	fn get_transform(&self) -> Mat4f { Mat4f::identity() }
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

	pub fn render_a(&mut self, ghi: &dyn ghi::GraphicsHardwareInterface, command_buffer_recording: &mut dyn ghi::CommandBufferRecording) {
		let camera_handle = if let Some(camera_handle) = &self.camera { camera_handle } else { return; };

		{
			let mut command_buffer_recording = ghi.create_command_buffer_recording(self.transfer_command_buffer, None);

			command_buffer_recording.transfer_textures(&self.pending_texture_loads);

			self.pending_texture_loads.clear();

			command_buffer_recording.execute(&[], &[], self.transfer_synchronizer);
		}

		ghi.wait(self.transfer_synchronizer); // Bad

		let camera_data_buffer = ghi.get_mut_buffer_slice(self.camera_data_buffer_handle);

		let (camera_position, camera_orientation) = camera_handle.map(|camera| { let camera = camera.read_sync(); (camera.get_position(), camera.get_orientation()) });

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

		command_buffer_recording.start_region("Visibility Render Model");

		self.visibility_pass.render(command_buffer_recording, &self.visibility_info, self.primitive_index, self.instance_id, self.depth_target);
		self.material_count_pass.render(command_buffer_recording);
		self.material_offset_pass.render(command_buffer_recording);
		self.pixel_mapping_pass.render(command_buffer_recording);

		command_buffer_recording.end_region();
	}

	pub fn render_b(&mut self, ghi: &dyn ghi::GraphicsHardwareInterface, command_buffer_recording: &mut dyn ghi::CommandBufferRecording) {
		{
			let shadow_render_pass = self.shadow_render_pass.read_sync();
			
			let mut directional_lights: Vec<&LightData> = self.lights.iter().filter(|l| l.light_type == 'D').collect();
			directional_lights.sort_by(|a, b| maths_rs::length(a.color).partial_cmp(&maths_rs::length(b.color)).unwrap()); // Sort by intensity

			if let Some(most_significant_light) = directional_lights.get(0) {
				let normal = most_significant_light.view_matrix;

				shadow_render_pass.prepare(ghi, normal);
				shadow_render_pass.render(command_buffer_recording, self);
			} else {
				command_buffer_recording.clear_images(&[(shadow_render_pass.get_shadow_map_image(), ghi::ClearValue::Depth(1f32)),]);
			}
		}

		command_buffer_recording.start_region("Material Evaluation");
		command_buffer_recording.clear_images(&[(self.albedo, ghi::ClearValue::Color(crate::RGBA::black())),]);
		for (_, (i, pipeline)) in self.material_evaluation_materials.iter() {
			// No need for sync here, as each thread across all invocations will write to a different pixel
			let compute_pipeline_command = command_buffer_recording.bind_compute_pipeline(pipeline);
			compute_pipeline_command.bind_descriptor_sets(&self.material_evaluation_pipeline_layout, &[self.descriptor_set, self.visibility_passes_descriptor_set, self.material_evaluation_descriptor_set]);
			compute_pipeline_command.write_to_push_constant(&self.material_evaluation_pipeline_layout, 0, unsafe {
				std::slice::from_raw_parts(&(*i as u32) as *const u32 as *const u8, std::mem::size_of::<u32>())
			});
			compute_pipeline_command.indirect_dispatch(&self.material_offset_pass.material_evaluation_dispatches, *i as usize);
		}
		command_buffer_recording.end_region();

		// ghi.wait(self.transfer_synchronizer); // Wait for buffers to be copied over to the GPU, or else we might overwrite them on the CPU before they are copied over
	}
}

impl EntitySubscriber<camera::Camera> for VisibilityWorldRenderDomain {
	async fn on_create<'a>(&'a mut self, handle: EntityHandle<camera::Camera>, camera: &camera::Camera) {
		self.camera = Some(handle);
	}

	async fn on_update(&'static mut self, handle: EntityHandle<camera::Camera>, params: &camera::Camera) {
		
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
#[derive(Copy, Clone)]
struct LightData {
	view_matrix: Mat4f,
	projection_matrix: Mat4f,
	vp_matrix: Mat4f,
	position: Vector3,
	color: Vector3,
	light_type: char,
}

#[repr(C)]
struct MaterialData {
	textures: [u32; 16],
}

impl EntitySubscriber<mesh::Mesh> for VisibilityWorldRenderDomain {
	async fn on_create<'a>(&'a mut self, handle: EntityHandle<mesh::Mesh>, mesh: &mesh::Mesh) {
		
		if !self.material_evaluation_materials.contains_key(mesh.get_material_id()) {
			let response_and_data = {
				let resource_manager = self.resource_manager.read().await;
				resource_manager.get(mesh.get_material_id()).await.unwrap()
			};

			self.load_material(response_and_data,);
		}

		if !self.mesh_resources.contains_key(mesh.get_resource_id()) { // Load only if not already loaded
			let mut ghi = self.ghi.write().unwrap();

			let resource_request = {
				let resource_manager = self.resource_manager.read().await;
				resource_manager.request_resource(mesh.get_resource_id()).await
			};

			let resource_request = if let Some(resource_info) = resource_request { resource_info } else { return; };

			let mut vertex_positions_buffer = ghi.get_splitter(self.vertex_positions_buffer, self.visibility_info.vertex_count as usize * std::mem::size_of::<Vector3>());
			let mut vertex_normals_buffer = ghi.get_splitter(self.vertex_normals_buffer, self.visibility_info.vertex_count as usize * std::mem::size_of::<Vector3>());
			let mut triangle_indices_buffer = ghi.get_splitter(self.vertex_indices_buffer, self.visibility_info.triangle_count as usize * 3 * std::mem::size_of::<u16>());
			let mut vertex_indices_buffer = ghi.get_splitter(self.vertex_indices_buffer, self.visibility_info.vertex_count as usize * std::mem::size_of::<u16>());
			let mut primitive_indices_buffer = ghi.get_splitter(self.primitive_indices_buffer, self.visibility_info.triangle_count as usize * 3 * std::mem::size_of::<u8>());

			let mut meshlet_stream_buffer = vec![0u8; 1024 * 8];

			let mut buffer_allocator = utils::BufferAllocator::new(&mut meshlet_stream_buffer);

			let resources = resource_request.resources.into_iter().map(|resource| {
				match resource.class.as_str() {
					"Mesh" => {
						let mesh_resource = resource.resource.downcast_ref::<mesh_resource_handler::Mesh>().unwrap();

						let triangle_stream = mesh_resource.index_streams.iter().find(|is| is.stream_type == mesh_resource_handler::IndexStreamTypes::Triangles).unwrap();
						let vertex_indices_stream = mesh_resource.index_streams.iter().find(|is| is.stream_type == mesh_resource_handler::IndexStreamTypes::Vertices).unwrap();
						let primitive_indices_stream = mesh_resource.index_streams.iter().find(|is| is.stream_type == mesh_resource_handler::IndexStreamTypes::Meshlets).unwrap();

						let meshlet_stream = mesh_resource.meshlet_stream.as_ref().unwrap();
						
						let vertex_positions_buffer = vertex_positions_buffer.take(mesh_resource.vertex_count as usize * std::mem::size_of::<Vector3>());
						let vertex_normals_buffer = vertex_normals_buffer.take(mesh_resource.vertex_count as usize * std::mem::size_of::<Vector3>());
						let triangle_indices_buffer = triangle_indices_buffer.take(triangle_stream.count as usize * std::mem::size_of::<u16>());
						let vertex_indices_buffer = vertex_indices_buffer.take(vertex_indices_stream.count as usize * std::mem::size_of::<u16>());
						let primitive_indices_buffer = primitive_indices_buffer.take(primitive_indices_stream.count as usize * std::mem::size_of::<u8>());
						let meshlet_stream_buffer = buffer_allocator.take(meshlet_stream.count as usize * 2usize);

						let streams = vec![
							resource_management::Stream{ buffer: vertex_positions_buffer, name: "Vertex.Position".to_string() },
							resource_management::Stream{ buffer: vertex_normals_buffer, name: "Vertex.Normal".to_string() },
							resource_management::Stream{ buffer: triangle_indices_buffer, name: "TriangleIndices".to_string() },
							resource_management::Stream{ buffer: vertex_indices_buffer, name: "VertexIndices".to_string() },
							resource_management::Stream{ buffer: primitive_indices_buffer, name: "MeshletIndices".to_string() },
							resource_management::Stream{ buffer: meshlet_stream_buffer , name: "Meshlets".to_string() },
						];

						resource_management::LoadResourceRequest::new(resource).streams(streams)
					}
					_ => { resource_management::LoadResourceRequest::new(resource) }
				}
			}).collect::<Vec<_>>();

			let resource = if let Ok(a) = {
				let resource_manager = self.resource_manager.read().await;
				let resource_load_request = resource_management::LoadRequest::new(resources);
				resource_manager.load_resource(resource_load_request,).await
			} { a } else { return; };

			let response = resource.0;

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
									vertex_position_encoding: ghi::Encodings::FloatingPoint,
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

	async fn on_update(&'static mut self, handle: EntityHandle<mesh::Mesh>, params: &mesh::Mesh) {
		
	}
}

impl EntitySubscriber<directional_light::DirectionalLight> for VisibilityWorldRenderDomain {
	async fn on_create<'a>(&'a mut self, handle: EntityHandle<directional_light::DirectionalLight>, light: &directional_light::DirectionalLight) {
		let ghi = self.ghi.write().unwrap();

		let lighting_data = unsafe { (ghi.get_mut_buffer_slice(self.light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };

		let light_index = lighting_data.count as usize;
		
		let x = 4f32;
		let light_projection_matrix = math::orthographic_matrix(x, x, -5f32, 5f32);

		let normal = light.direction;
		let light_view_matrix = math::from_normal(-normal);

		let vp_matrix = light_projection_matrix * light_view_matrix;

		lighting_data.lights[light_index].light_type = 'D';
		lighting_data.lights[light_index].view_matrix = light_view_matrix;
		lighting_data.lights[light_index].projection_matrix = light_projection_matrix;
		lighting_data.lights[light_index].vp_matrix = vp_matrix;
		lighting_data.lights[light_index].position = light.direction;
		lighting_data.lights[light_index].color = light.color;
		
		lighting_data.count += 1;

		self.lights.push(lighting_data.lights[light_index]);

		assert!(lighting_data.count < MAX_LIGHTS as u32, "Light count exceeded");
	}

	async fn on_update(&'static mut self, handle: EntityHandle<directional_light::DirectionalLight>, params: &directional_light::DirectionalLight) {
		
	}
}

impl EntitySubscriber<point_light::PointLight> for VisibilityWorldRenderDomain {
	async fn on_create<'a>(&'a mut self, handle: EntityHandle<point_light::PointLight>, light: &point_light::PointLight) {
		let ghi = self.ghi.write().unwrap();

		let lighting_data = unsafe { (ghi.get_mut_buffer_slice(self.light_data_buffer).as_mut_ptr() as *mut LightingData).as_mut().unwrap() };

		let light_index = lighting_data.count as usize;

		lighting_data.lights[light_index].light_type = 'P';
		lighting_data.lights[light_index].view_matrix = maths_rs::Mat4f::identity();
		lighting_data.lights[light_index].projection_matrix = maths_rs::Mat4f::identity();
		lighting_data.lights[light_index].vp_matrix = maths_rs::Mat4f::identity();
		lighting_data.lights[light_index].position = light.position;
		lighting_data.lights[light_index].color = light.color;
		
		lighting_data.count += 1;

		assert!(lighting_data.count < MAX_LIGHTS as u32, "Light count exceeded");
	}

	async fn on_update(&'static mut self, handle: EntityHandle<point_light::PointLight>, params: &point_light::PointLight) {
		
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
}

struct VisibilityPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_pipeline: ghi::PipelineHandle,
}

impl VisibilityPass {
	pub fn new(ghi_instance: &mut dyn ghi::GraphicsHardwareInterface, pipeline_layout_handle: ghi::PipelineLayoutHandle, descriptor_set: ghi::DescriptorSetHandle, primitive_index: ghi::ImageHandle, instance_id: ghi::ImageHandle, depth_target: ghi::ImageHandle) -> Self {
		let visibility_pass_mesh_shader = ghi_instance.create_shader(Some("Visibility Pass Mesh Shader"), ghi::ShaderSource::GLSL(get_visibility_pass_mesh_source()), ghi::ShaderTypes::Mesh,
			&[
				ghi::ShaderBindingDescriptor::new(0, 0, ghi::AccessPolicies::READ),
				ghi::ShaderBindingDescriptor::new(0, 1, ghi::AccessPolicies::READ),
				ghi::ShaderBindingDescriptor::new(0, 2, ghi::AccessPolicies::READ),
				ghi::ShaderBindingDescriptor::new(0, 3, ghi::AccessPolicies::READ),
				ghi::ShaderBindingDescriptor::new(0, 4, ghi::AccessPolicies::READ),
				ghi::ShaderBindingDescriptor::new(0, 5, ghi::AccessPolicies::READ),
				ghi::ShaderBindingDescriptor::new(0, 6, ghi::AccessPolicies::READ),
			]
		).expect("Failed to create shader");

		let visibility_pass_fragment_shader = ghi_instance.create_shader(Some("Visibility Pass Fragment Shader"), ghi::ShaderSource::GLSL(VISIBILITY_PASS_FRAGMENT_SOURCE.to_string()), ghi::ShaderTypes::Fragment, &[]).expect("Failed to create shader");

		let visibility_pass_shaders: &[(&ghi::ShaderHandle, ghi::ShaderTypes, &[ghi::SpecializationMapEntry])] = &[
			(&visibility_pass_mesh_shader, ghi::ShaderTypes::Mesh, &[]),
			(&visibility_pass_fragment_shader, ghi::ShaderTypes::Fragment, &[]),
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

	pub fn render(&self, command_buffer_recording: &mut dyn ghi::CommandBufferRecording, visibility_info: &VisibilityInfo, primitive_index: ghi::ImageHandle, instance_id: ghi::ImageHandle, depth_target: ghi::ImageHandle) {
		command_buffer_recording.start_region("Visibility Buffer");

		let attachments = [
			ghi::AttachmentInformation::new(primitive_index,ghi::Formats::U32,ghi::Layouts::RenderTarget,ghi::ClearValue::Integer(!0u32, 0, 0, 0),false,true,),
			ghi::AttachmentInformation::new(instance_id,ghi::Formats::U32,ghi::Layouts::RenderTarget,ghi::ClearValue::Integer(!0u32, 0, 0, 0),false,true,),
			ghi::AttachmentInformation::new(depth_target,ghi::Formats::Depth32,ghi::Layouts::RenderTarget,ghi::ClearValue::Depth(0f32),false,true,),
		];

		let render_pass_command = command_buffer_recording.start_render_pass(Extent::rectangle(1920, 1080), &attachments);
		render_pass_command.bind_descriptor_sets(&self.pipeline_layout, &[self.descriptor_set]);
		render_pass_command.bind_raster_pipeline(&self.visibility_pass_pipeline).dispatch_meshes(visibility_info.meshlet_count, 1, 1);
		render_pass_command.end_render_pass();

		command_buffer_recording.end_region();
	}
}

struct MaterialCountPass {
	pipeline_layout: ghi::PipelineLayoutHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	material_count_buffer: ghi::BaseBufferHandle,
	pipeline: ghi::PipelineHandle,
}

impl MaterialCountPass {
	fn new(ghi_instance: &mut dyn ghi::GraphicsHardwareInterface, pipeline_layout: ghi::PipelineLayoutHandle, descriptor_set: ghi::DescriptorSetHandle, visibility_pass_descriptor_set: ghi::DescriptorSetHandle, visibility_pass: &VisibilityPass) -> Self {
		let material_count_shader = ghi_instance.create_shader(Some("Material Count Pass Compute Shader"), ghi::ShaderSource::GLSL(get_material_count_source()), ghi::ShaderTypes::Compute,
			&[
				ghi::ShaderBindingDescriptor::new(0, 0, ghi::AccessPolicies::READ),
				ghi::ShaderBindingDescriptor::new(1, 0, ghi::AccessPolicies::READ | ghi::AccessPolicies::WRITE),
				ghi::ShaderBindingDescriptor::new(1, 7, ghi::AccessPolicies::READ),
			]
		).expect("Failed to create shader");

		let material_count_pipeline = ghi_instance.create_compute_pipeline(&pipeline_layout, (&material_count_shader, ghi::ShaderTypes::Compute, &[]));

		let material_count_buffer = ghi_instance.create_buffer(Some("Material Count"), std::mem::size_of::<[u32; MAX_MATERIALS]>(), ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

		MaterialCountPass {
			pipeline_layout,
			descriptor_set,
			material_count_buffer,
			visibility_pass_descriptor_set,
			pipeline: material_count_pipeline,
		}
	}

	fn render(&self, command_buffer_recording: &mut dyn ghi::CommandBufferRecording) {
		let pipeline_layout = self.pipeline_layout;
		let descriptor_set = self.descriptor_set;
		let visibility_pass_descriptor_set = self.visibility_pass_descriptor_set;
		let pipeline = self.pipeline;

		command_buffer_recording.start_region("Material Count");

		command_buffer_recording.clear_buffers(&[self.material_count_buffer]);

		command_buffer_recording.bind_descriptor_sets(&pipeline_layout, &[descriptor_set, visibility_pass_descriptor_set]);
		let compute_pipeline_command = command_buffer_recording.bind_compute_pipeline(&pipeline);
		compute_pipeline_command.dispatch(ghi::DispatchExtent::new(Extent::rectangle(1920, 1080), Extent::square(32)));

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
	fn new(ghi_instance: &mut dyn ghi::GraphicsHardwareInterface, pipeline_layout: ghi::PipelineLayoutHandle, descriptor_set: ghi::DescriptorSetHandle, visibility_pass_descriptor_set: ghi::DescriptorSetHandle) -> Self {
		let material_offset_shader = ghi_instance.create_shader(Some("Material Offset Pass Compute Shader"), ghi::ShaderSource::GLSL(MATERIAL_OFFSET_SOURCE.to_string()), ghi::ShaderTypes::Compute,
			&[
				ghi::ShaderBindingDescriptor::new(1, 0, ghi::AccessPolicies::READ),
				ghi::ShaderBindingDescriptor::new(1, 1, ghi::AccessPolicies::WRITE),
				ghi::ShaderBindingDescriptor::new(1, 2, ghi::AccessPolicies::WRITE),
				ghi::ShaderBindingDescriptor::new(1, 3, ghi::AccessPolicies::WRITE),
			]
		).expect("Failed to create shader");

		let material_offset_pipeline = ghi_instance.create_compute_pipeline(&pipeline_layout, (&material_offset_shader, ghi::ShaderTypes::Compute, &[]));

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

	fn render(&self, command_buffer_recording: &mut dyn ghi::CommandBufferRecording) {
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
	fn new(ghi_instance: &mut dyn ghi::GraphicsHardwareInterface, pipeline_layout: ghi::PipelineLayoutHandle, descriptor_set: ghi::DescriptorSetHandle, visibility_passes_descriptor_set: ghi::DescriptorSetHandle) -> Self {
		let pixel_mapping_shader = ghi_instance.create_shader(Some("Pixel Mapping Pass Compute Shader"), ghi::ShaderSource::GLSL(get_pixel_mapping_source()), ghi::ShaderTypes::Compute,
			&[
				ghi::ShaderBindingDescriptor::new(0, 1, ghi::AccessPolicies::READ),
				ghi::ShaderBindingDescriptor::new(1, 2, ghi::AccessPolicies::READ | ghi::AccessPolicies::WRITE),
				ghi::ShaderBindingDescriptor::new(1, 4, ghi::AccessPolicies::WRITE),
				ghi::ShaderBindingDescriptor::new(1, 7, ghi::AccessPolicies::READ),
			]
		).expect("Failed to create shader");

		let pixel_mapping_pipeline = ghi_instance.create_compute_pipeline(&pipeline_layout, (&pixel_mapping_shader, ghi::ShaderTypes::Compute, &[]));

		let material_xy = ghi_instance.create_buffer(Some("Material XY"), 1920 * 1080 * 2 * 2, ghi::Uses::Storage | ghi::Uses::TransferDestination, ghi::DeviceAccesses::GpuWrite | ghi::DeviceAccesses::GpuRead, ghi::UseCases::STATIC);

		PixelMappingPass {
			material_xy,
			pipeline_layout,
			descriptor_set,
			visibility_passes_descriptor_set,
			pixel_mapping_pipeline,
		}
	}

	fn render(&self, command_buffer_recording: &mut dyn ghi::CommandBufferRecording,) {
		let pipeline_layout = self.pipeline_layout;
		let descriptor_set = self.descriptor_set;
		let pipeline = self.pixel_mapping_pipeline;
		let visibility_passes_descriptor_set = self.visibility_passes_descriptor_set;

		command_buffer_recording.start_region("Pixel Mapping");

		command_buffer_recording.clear_buffers(&[self.material_xy,]);

		command_buffer_recording.bind_descriptor_sets(&pipeline_layout, &[descriptor_set, visibility_passes_descriptor_set]);
		let compute_pipeline_command = command_buffer_recording.bind_compute_pipeline(&pipeline);
		compute_pipeline_command.dispatch(ghi::DispatchExtent::new(Extent::rectangle(1920, 1080), Extent::square(32)));

		command_buffer_recording.end_region();
	}
}

struct MaterialEvaluationPass {
}

impl MaterialEvaluationPass {
	fn new(ghi_instance: &mut dyn ghi::GraphicsHardwareInterface, visibility_pass: &VisibilityPass, material_count_pass: &MaterialCountPass, material_offset_pass: &MaterialOffsetPass, pixel_mapping_pass: &PixelMappingPass) -> Self {
		MaterialEvaluationPass {}
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

pub fn get_visibility_pass_mesh_source() -> String {
	let mut string = shader_generator::generate_glsl_header_block(&json::object! { "glsl": { "version": "450" }, "stage": "Mesh" });
	string.push_str(CAMERA_STRUCT_GLSL);
	string.push_str(MESH_STRUCT_GLSL);
	string.push_str(MESHLET_STRUCT_GLSL);
	string.push_str("layout(location=0) perprimitiveEXT out uint out_instance_index[126];
	layout(location=1) perprimitiveEXT out uint out_primitive_index[126];
	
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
	}");
	string
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
	let mut string = shader_generator::generate_glsl_header_block(&json::object! { "glsl": { "version": "450" }, "stage": "Compute" });
	string.push_str(MESH_STRUCT_GLSL);
	string.push_str("	
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
	}");
	string
}

const MATERIAL_OFFSET_SOURCE: &'static str = r#"
#version 450
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_explicit_arithmetic_types : enable

layout(set=1,binding=0,scalar) buffer readonly MaterialCount {
	uint material_count[];
};

layout(set=1,binding=1,scalar) buffer writeonly MaterialOffset {
	uint material_offset[];
};

layout(set=1,binding=2,scalar) buffer writeonly MaterialOffsetScratch {
	uint material_offset_scratch[];
};

layout(set=1,binding=3,scalar) buffer writeonly MaterialEvaluationDispatches {
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

pub fn get_pixel_mapping_source() -> String {
	let mut string = shader_generator::generate_glsl_header_block(&json::object! { "glsl": { "version": "450" }, "stage": "Compute" });
	string.push_str(MESH_STRUCT_GLSL);
	string.push_str(MESHLET_STRUCT_GLSL);
	string.push_str("	
	layout(set=0,binding=1,scalar) buffer MeshesBuffer {
		Mesh meshes[];
	};
	
	layout(set=1,binding=2,scalar) buffer MaterialOffsetScratch {
		uint material_offset_scratch[];
	};
	
	layout(set=1,binding=4,scalar) buffer writeonly PixelMapping {
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
	");
	string

}

pub const LIGHT_STRUCT_GLSL: &'static str = "struct Light {
	mat4 view_matrix;
	mat4 projection_matrix;
	mat4 vp_matrix;
	vec3 position;
	vec3 color;
	uint8_t light_type;
};";

pub const LIGHTING_DATA_STRUCT_GLSL: &'static str = "struct LightingData {
	uint light_count;
	Light lights[16];
};";

pub const MATERIAL_STRUCT_GLSL: &'static str = "struct Material {
	uint textures[16];
};";

pub const CAMERA_STRUCT_GLSL: &'static str = "struct Camera {
	mat4 view;
	mat4 projection_matrix;
	mat4 view_projection;
};";

pub const MESHLET_STRUCT_GLSL: &'static str = "struct Meshlet {
	uint32_t instance_index;
	uint16_t vertex_offset;
	uint16_t triangle_offset;
	uint8_t vertex_count;
	uint8_t triangle_count;
};";

pub const MESH_STRUCT_GLSL: &'static str = "struct Mesh {
	mat4 model;
	uint material_index;
	uint32_t base_vertex_index;
};";