use ::core::slice::SlicePattern;
use std::borrow::Borrow;
use std::cell::{OnceCell, RefCell};
use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::ops::{Deref, DerefMut};
use std::sync::OnceLock;

use crate::core::{Entity, EntityHandle};
use crate::rendering::common_shader_generator::{CommonShaderGenerator, CommonShaderScope};
use crate::rendering::lights::{DirectionalLight, Light, Lights, PointLight};
use crate::rendering::mesh::generator::MeshGenerator;
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::pipelines::visibility::render_pass::VisibilityPipelineRenderPass;
use crate::rendering::pipelines::visibility::{
	INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING, MATERIAL_EVALUATION_DISPATCHES_BINDING, MATERIAL_OFFSET_BINDING,
	MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING, MAX_INSTANCES, MAX_LIGHTS, MAX_MATERIALS, MAX_MESHLETS,
	MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES, MESHLET_DATA_BINDING, MESH_DATA_BINDING, PRIMITIVE_INDICES_BINDING,
	SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION, TEXTURES_BINDING, TRIANGLE_INDEX_BINDING, VERTEX_INDICES_BINDING,
	VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
};
use crate::rendering::render_pass::{FramePrepare, RenderPass, RenderPassBuilder, RenderPassFunction, RenderPassReturn};
use crate::rendering::renderable::mesh::MeshSource;
use crate::rendering::scene_manager::SceneManager;
use crate::rendering::texture_manager::TextureManager;
use crate::rendering::view::View;
use ghi::device::{Device as _, DeviceCreate as _};
use ghi::frame::Frame as _;
use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
		CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	graphics_hardware_interface,
};
use log::{error, warn};
use math::{mat::MatInverse as _, Matrix4, Vector3};
use resource_management::asset::bema_asset_handler::ProgramGenerator;
use resource_management::glsl_shader_generator::GLSLShaderGenerator;
use resource_management::msl_shader_generator::MSLShaderGenerator;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resources::image::Image as ResourceImage;
use resource_management::resources::material::Variant as ResourceVariant;
use resource_management::resources::material::{Material as ResourceMaterial, Parameter, Shader, Value, VariantVariable};
use resource_management::resources::mesh::{Mesh as ResourceMesh, Primitive};
use resource_management::shader_generator::{ShaderGenerationSettings, ShaderGenerator};
use resource_management::spirv_shader_generator::SPIRVShaderGenerator;
use resource_management::types::{IndexStreamTypes, IntegralTypes, ShaderTypes};
use resource_management::{glsl, Reference};
use utils::hash::{HashMap, HashMapExt};
use utils::json::{self, object};
use utils::sync::{Arc, Rc, RwLock};
use utils::{Box, Extent, RGBA};

use super::shader_generator::{VisibilityShaderGenerator, VisibilityShaderScope};
use crate::rendering::{
	csm, make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor, mesh, world_render_domain,
	RenderableMesh, Viewport,
};
use crate::resource_management::{self};
use crate::space::Transformable as _;

const diffuse_binding_template: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const specular_binding_template: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const lighting_data_binding_template: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(4, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
const materials_data_binding_template: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(5, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);
const ao_map_binding_template: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	10,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const shadow_map_binding_template: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new_array(
	11,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
	1,
);
const visibility_depth_binding_template: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	12,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const ibl_cubemap_binding_template: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	13,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);

/// This the visibility buffer implementation of the world render domain.
pub struct VisibilityWorldRenderDomain {
	/// Tracks buffer offsets and counts for various resources.
	visibility_info: VisibilityInfo,
	/// Render entities that will be rendered in the scene.
	render_entities: Vec<(EntityHandle<dyn RenderableMesh>, ShaderMesh)>,
	/// Loaded mesh resources.
	meshes: Vec<MeshData>,
	/// Mapping from resource ID to mesh index.
	meshes_by_resource: HashMap<String, usize>,
	/// Mapping from generated mesh hash to mesh index.
	meshes_by_generated_hash: HashMap<u64, usize>,
	/// Loaded images.
	images: HashMap<String, Image>,
	/// Texture manager.
	texture_manager: TextureManager,
	/// Pipeline manager.
	pipeline_manager: PipelineManager,
	/// Mapping from mesh resource ID to mesh index.
	mesh_resources: HashMap<String, u32>,
	/// Material evaluation materials.
	material_evaluation_materials: HashMap<String, Arc<OnceLock<RenderDescription>>>,
	/// Vertex positions buffer for rendered meshes.
	vertex_positions_buffer: ghi::BufferHandle<[(f32, f32, f32); MAX_VERTICES]>,
	/// Vertex normals buffer for rendered meshes.
	vertex_normals_buffer: ghi::BufferHandle<[(f32, f32, f32); MAX_VERTICES]>,
	/// Vertex UVs buffer for rendered meshes.
	vertex_uvs_buffer: ghi::BufferHandle<[(f32, f32); MAX_VERTICES]>,
	/// Indices laid out as indices into the vertex buffers
	vertex_indices_buffer: ghi::BufferHandle<[u16; MAX_PRIMITIVE_TRIANGLES]>,
	/// Indices laid out as indices into the `vertex_indices_buffer`
	primitive_indices_buffer: ghi::BufferHandle<[[u8; 3]; MAX_TRIANGLES]>,
	/// Views data buffer.
	views_data_buffer_handle: ghi::DynamicBufferHandle<[ShaderViewData; 8]>,
	///  Materials data buffer.
	materials_data_buffer_handle: ghi::BufferHandle<[MaterialData; MAX_MATERIALS]>,
	/// Base descriptor set layout.
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	textures_binding: ghi::DescriptorSetBindingHandle,
	/// Handle to the buffer where each instance's data is stored.
	meshes_data_buffer: ghi::DynamicBufferHandle<[ShaderMesh; MAX_INSTANCES]>,
	/// Handle to the buffer where each meshlet's data is stored.
	meshlets_data_buffer: ghi::BufferHandle<[ShaderMeshletData; MAX_MESHLETS]>,
	material_evaluation_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
	/// Buffer containing lighting data.
	light_data_buffer: ghi::BufferHandle<LightingData>,
	/// Lights in the scene.
	lights: Vec<Lights>,
	/// Information about the current render.
	render_info: RenderInfo,
	/// Views
	views: Vec<(usize, VisibilityPipelineRenderPass)>,
	visibility_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	resource_manager: EntityHandle<ResourceManager>,
}

impl VisibilityWorldRenderDomain {
	pub fn new(
		device: &mut ghi::implementation::Device,
		texture_manager: TextureManager,
		resource_manager: EntityHandle<ResourceManager>,
	) -> Self {
		// Initialize the extent to 0 to allocate memory lazily.
		let extent = Extent::square(0);

		let vertex_positions_buffer_handle = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Vertex Positions Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let vertex_normals_buffer_handle = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Vertex Normals Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let vertex_uv_buffer_handle = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Vertex UV Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let vertex_indices_buffer_handle = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Index Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let primitive_indices_buffer_handle = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage)
				.name("Visibility Primitive Indices Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let meshlets_data_buffer = device.build_buffer::<[ShaderMeshletData; MAX_MESHLETS]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Meshlets Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let views_data_buffer_handle = device.build_dynamic_buffer::<[ShaderViewData; 8]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Views Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let meshes_data_buffer = device.build_dynamic_buffer::<[ShaderMesh; MAX_INSTANCES]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Meshes Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let bindings = [
			VIEWS_DATA_BINDING,
			MESH_DATA_BINDING,
			VERTEX_POSITIONS_BINDING,
			VERTEX_NORMALS_BINDING,
			VERTEX_UV_BINDING,
			VERTEX_INDICES_BINDING,
			PRIMITIVE_INDICES_BINDING,
			MESHLET_DATA_BINDING,
			TEXTURES_BINDING,
		];

		let descriptor_set_layout = device.create_descriptor_set_template(Some("Base Set Layout"), &bindings);

		let descriptor_set = device.create_descriptor_set(Some("Base Descriptor Set"), &descriptor_set_layout);

		let views_data_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VIEWS_DATA_BINDING, views_data_buffer_handle.into()),
		);
		let meshes_data_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&MESH_DATA_BINDING, meshes_data_buffer.into()),
		);
		let vertex_positions_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_POSITIONS_BINDING, vertex_positions_buffer_handle.into()),
		);
		let vertex_normals_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_NORMALS_BINDING, vertex_normals_buffer_handle.into()),
		);
		let vertex_uv_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_UV_BINDING, vertex_uv_buffer_handle.into()),
		);
		let vertex_indices_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&VERTEX_INDICES_BINDING, vertex_indices_buffer_handle.into()),
		);
		let primitive_indices_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&PRIMITIVE_INDICES_BINDING, primitive_indices_buffer_handle.into()),
		);
		let meshlets_data_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::buffer(&MESHLET_DATA_BINDING, meshlets_data_buffer.into()),
		);
		let textures_binding = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::combined_image_sampler_array(&TEXTURES_BINDING),
		);

		let bindings = [
			MATERIAL_COUNT_BINDING,
			MATERIAL_OFFSET_BINDING,
			MATERIAL_OFFSET_SCRATCH_BINDING,
			MATERIAL_EVALUATION_DISPATCHES_BINDING,
			MATERIAL_XY_BINDING,
			TRIANGLE_INDEX_BINDING,
			INSTANCE_ID_BINDING,
		];

		let visibility_descriptor_set_layout = device.create_descriptor_set_template(Some("Visibility Set Layout"), &bindings);

		let light_data_buffer = device.build_buffer::<LightingData>(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Light Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let lighting_data = device.get_mut_buffer_slice(light_data_buffer);

		lighting_data.count = 0; // Initially, no lights

		let materials_data_buffer_handle = device.build_buffer::<[MaterialData; MAX_MATERIALS]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Materials Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let bindings = [
			diffuse_binding_template,
			ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
			specular_binding_template,
			ghi::DescriptorSetBindingTemplate::new(3, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
			lighting_data_binding_template,
			materials_data_binding_template,
			ao_map_binding_template,
			shadow_map_binding_template,
			visibility_depth_binding_template,
			ibl_cubemap_binding_template,
		];

		let sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let depth_sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);

		let material_evaluation_descriptor_set_layout =
			device.create_descriptor_set_template(Some("Material Evaluation Set Layout"), &bindings);
		let material_evaluation_descriptor_set = device.create_descriptor_set(
			Some("Material Evaluation Descriptor Set"),
			&material_evaluation_descriptor_set_layout,
		);

		Self {
			render_entities: Vec::with_capacity(512),

			visibility_info: VisibilityInfo::default(),

			meshes: Vec::with_capacity(1024),
			meshes_by_resource: HashMap::with_capacity(1024),
			meshes_by_generated_hash: HashMap::with_capacity(128),

			images: HashMap::with_capacity(1024),

			texture_manager,
			pipeline_manager: PipelineManager::new(),

			mesh_resources: HashMap::new(),

			material_evaluation_materials: HashMap::new(),

			vertex_positions_buffer: vertex_positions_buffer_handle,
			vertex_normals_buffer: vertex_normals_buffer_handle,
			vertex_uvs_buffer: vertex_uv_buffer_handle,

			vertex_indices_buffer: vertex_indices_buffer_handle,
			primitive_indices_buffer: primitive_indices_buffer_handle,

			descriptor_set_layout,
			descriptor_set,

			visibility_descriptor_set_layout,

			textures_binding,

			views_data_buffer_handle,

			meshes_data_buffer,
			meshlets_data_buffer,

			material_evaluation_descriptor_set_layout,
			material_evaluation_descriptor_set,

			light_data_buffer,
			materials_data_buffer_handle,

			lights: Vec::new(),

			render_info: RenderInfo {
				instances: Vec::with_capacity(4096),
			},

			views: Vec::with_capacity(4),

			resource_manager,
		}
	}

	pub fn create_renderable_mesh(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		mesh_source: EntityHandle<dyn RenderableMesh>,
	) {
		let renderable = mesh_source.clone();
		let mesh_source = mesh_source.get_mesh();

		match mesh_source {
			MeshSource::Resource(urid) => {
				if let Ok(idx) = self.create_mesh_resources(urid, frame) {
					let model = renderable.transform().get_matrix();
					let mesh = &self.meshes[idx];

					for primitive in &mesh.primitives {
						self.render_entities.push((
							renderable.clone(),
							ShaderMesh {
								model,
								material_index: primitive.material_index,
								base_vertex_index: mesh.vertex_offset + primitive.vertex_offset,
								base_primitive_index: mesh.primitive_offset + primitive.primitive_offset,
								base_triangle_index: mesh.triangle_offset + primitive.triangle_offset,
								base_meshlet_index: mesh.meshlet_offset + primitive.meshlet_offset,
							},
						));
						self.render_info.instances.push(Instance {
							meshlet_count: primitive.meshlet_count,
						});
					}
				}
			}
			MeshSource::Generated(generator) => {
				if let Ok(idx) = self.create_mesh_from_generator(generator.as_ref(), frame) {
					let model = renderable.transform().get_matrix();
					let mesh = &self.meshes[idx];

					for primitive in &mesh.primitives {
						self.render_entities.push((
							renderable.clone(),
							ShaderMesh {
								model,
								material_index: primitive.material_index,
								base_vertex_index: mesh.vertex_offset + primitive.vertex_offset,
								base_primitive_index: mesh.primitive_offset + primitive.primitive_offset,
								base_triangle_index: mesh.triangle_offset + primitive.triangle_offset,
								base_meshlet_index: mesh.meshlet_offset + primitive.meshlet_offset,
							},
						));
						self.render_info.instances.push(Instance {
							meshlet_count: primitive.meshlet_count,
						});
					}
				}
			}
		}
	}

	/// Creates the needed GHI resource for the given mesh.
	/// Does nothing if the mesh has already been loaded.
	fn create_mesh_resources<'a, 's: 'a>(
		&'s mut self,
		id: &'a str,
		device: &mut ghi::implementation::Frame,
	) -> Result<usize, ()> {
		if let Some(entry) = self.meshes_by_resource.get(id) {
			return Ok(*entry);
		}

		let mut meshlet_stream_buffer = vec![0u8; 1024 * 8];

		let mut resource_request: Reference<ResourceMesh> = {
			let resource_manager = &self.resource_manager;
			let Ok(resource_request) = resource_manager.request(id) else {
				log::error!("Failed to load mesh resource {}", id);
				return Err(());
			};
			resource_request
		};

		let mesh_resource = resource_request.resource();

		let Some(positions_stream) = mesh_resource.position_stream() else {
			log::error!("Mesh resource does not contain vertex position stream");
			return Err(());
		};

		let Some(normals_stream) = mesh_resource.normal_stream() else {
			log::error!("Mesh resource does not contain vertex normal stream");
			return Err(());
		};

		let Some(uvs_stream) = mesh_resource.uv_stream() else {
			log::error!("Mesh resource does not contain vertex uv stream");
			return Err(());
		};

		let Some(vertex_indices_stream) = mesh_resource.vertex_indices_stream() else {
			log::error!("Mesh resource does not contain vertex index stream");
			return Err(());
		};

		let Some(triangle_indices_stream) = mesh_resource.triangle_indices_stream() else {
			log::error!("Mesh resource does not contain triangle index stream");
			return Err(());
		};

		// let triangle_indices_stream: Option<resource_management::types::Stream> = None;

		let Some(meshlet_indices_stream) = mesh_resource.meshlet_indices_stream() else {
			log::error!("Mesh resource does not contain meshlet index stream");
			return Err(());
		};

		let Some(meshlets_stream) = mesh_resource.meshlets_stream() else {
			log::error!("Mesh resource does not contain meshlet stream");
			return Err(());
		};

		assert_eq!(meshlet_indices_stream.stride, 1, "Meshlet index stream is not u8");
		assert_eq!(vertex_indices_stream.stride, 2, "Vertex index stream is not u16");
		assert_eq!(meshlets_stream.stride, 2, "Meshlet stream stride is not of size 2");
		assert_eq!(
			meshlet_indices_stream.count() % 3,
			0,
			"Meshlet index stream does not contain complete triangles"
		);

		let vertex_offset = self.visibility_info.vertex_count as usize;
		let primitive_offset = self.visibility_info.primitives_count as usize;
		let triangle_offset = self.visibility_info.triangle_count as usize;
		let vertex_count = positions_stream.count();
		let primitive_count = vertex_indices_stream.count();
		let triangle_count = meshlet_indices_stream.count() / 3;

		let vertex_positions_buffer = device.get_mut_buffer_slice(self.vertex_positions_buffer);
		let vertex_normals_buffer = device.get_mut_buffer_slice(self.vertex_normals_buffer);
		let vertex_uv_buffer = device.get_mut_buffer_slice(self.vertex_uvs_buffer);
		let vertex_indices_buffer = device.get_mut_buffer_slice(self.vertex_indices_buffer);
		let primitive_indices_buffer = device.get_mut_buffer_slice(self.primitive_indices_buffer);

		let mut buffer_allocator = utils::BufferAllocator::new(&mut meshlet_stream_buffer);

		let streams = vec![
			resource_management::stream::StreamMut::new(
				"Vertex.Position",
				&mut vertex_positions_buffer[vertex_offset..][..vertex_count],
			),
			resource_management::stream::StreamMut::new(
				"Vertex.Normal",
				&mut vertex_normals_buffer[vertex_offset..][..normals_stream.count()],
			),
			resource_management::stream::StreamMut::new(
				"Vertex.UV",
				&mut vertex_uv_buffer[vertex_offset..][..uvs_stream.count()],
			),
			resource_management::stream::StreamMut::new(
				"VertexIndices",
				&mut vertex_indices_buffer[primitive_offset..][..primitive_count],
			),
			resource_management::stream::StreamMut::new(
				"MeshletIndices",
				&mut primitive_indices_buffer[triangle_offset..][..triangle_count],
			),
			resource_management::stream::StreamMut::new("Meshlets", buffer_allocator.take(meshlets_stream.size)),
		];

		let Ok(load_target) = resource_request.load(streams.into()) else {
			log::warn!("Failed to load mesh data");
			return Err(());
		};

		let Reference {
			resource: ResourceMesh {
				vertex_components,
				streams,
				primitives,
			},
			..
		} = resource_request;

		let vcps = primitives
			.iter()
			.scan(0, |state, p| {
				let offset = *state;
				*state += p.vertex_count;
				offset.into()
			})
			.collect::<Vec<_>>();

		self.mesh_resources
			.insert(id.to_string(), self.visibility_info.triangle_count);

		let total_meshlet_count = meshlets_stream.count();

		struct Meshlet {
			primitive_count: u8,
			triangle_count: u8,
		}

		let meshlets_per_primitive = primitives
			.into_iter()
			.zip(vcps.iter())
			.scan(
				(0, 0, 0),
				|(mesh_primitive_counter, mesh_triangle_counter, mesh_meshlet_counter), (primitive, vcps)| {
					let vertex_offset = *vcps;
					let primitive_offset = *mesh_primitive_counter;
					let triangle_offset = *mesh_triangle_counter;
					let meshlet_offset = *mesh_meshlet_counter;

					let meshlets = if let Some(stream) = primitive.meshlet_stream() {
						let m = load_target.stream("Meshlets").unwrap();

						let meshlet_stream = unsafe {
							std::slice::from_raw_parts(
								m.buffer().as_ptr().byte_add(stream.offset) as *const Meshlet,
								stream.count(),
							)
						};

						meshlet_stream
							.iter()
							.scan(
								(0, 0),
								|(primitive_primitive_counter, primitive_triangle_counter), meshlet| {
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
									}
									.into()
								},
							)
							.collect::<Vec<_>>()
					} else {
						panic!();
					};

					(
						MeshPrimitive {
							material_index: 0,
							meshlet_count: meshlets.len() as u32,
							meshlet_offset,
							vertex_offset,
							primitive_offset,
							triangle_offset,
						},
						meshlets,
						primitive,
					)
						.into()
				},
			)
			.collect::<Vec<_>>();

		let meshlets_per_primitive = meshlets_per_primitive
			.into_iter()
			.map(|(mp, meshlets, primitive)| {
				let variant = self.create_variant_resources(primitive.material, device).unwrap();
				(
					MeshPrimitive {
						material_index: variant,
						..mp
					},
					meshlets,
				)
			})
			.collect::<Vec<_>>();

		let meshlets_data_slice = device.get_mut_buffer_slice(self.meshlets_data_buffer);

		for (i, (primitive, meshlets)) in meshlets_per_primitive.iter().enumerate() {
			for (j, meshlet) in meshlets.iter().enumerate() {
				meshlets_data_slice[self.visibility_info.meshlet_count as usize + primitive.meshlet_offset as usize + j] =
					*meshlet;
			}
		}

		let primitives = meshlets_per_primitive.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>();

		let meshlet_offset = self.visibility_info.meshlet_count;

		let acceleration_structure = if let Some(triangle_indices_stream) = None as Option<resource_management::types::Stream> {
			let index_format = match triangle_indices_stream.stride {
				2 => ghi::DataTypes::U16,
				4 => ghi::DataTypes::U32,
				_ => panic!("Unsupported index format"),
			};

			let bottom_level_acceleration_structure =
				device.create_bottom_level_acceleration_structure(&ghi::BottomLevelAccelerationStructure {
					description: ghi::BottomLevelAccelerationStructureDescriptions::Mesh {
						vertex_count: positions_stream.count() as u32,
						vertex_position_encoding: ghi::Encodings::FloatingPoint,
						triangle_count: triangle_indices_stream.count() as u32 / 3,
						index_format,
					},
				});

			// ray_tracing.pending_meshes.push(MeshState::Build { mesh_handle: mesh.resource_id.to_string() });

			Some(bottom_level_acceleration_structure)
		} else {
			None
		};

		device.sync_buffer(self.vertex_positions_buffer);
		device.sync_buffer(self.vertex_normals_buffer);
		device.sync_buffer(self.vertex_uvs_buffer);
		device.sync_buffer(self.vertex_indices_buffer);
		device.sync_buffer(self.primitive_indices_buffer);
		device.sync_buffer(self.meshlets_data_buffer);
		device.sync_buffer(self.meshes_data_buffer);

		let mesh_id = self.meshes.len();

		self.meshes.push(MeshData {
			vertex_offset: self.visibility_info.vertex_count,
			primitive_offset: self.visibility_info.primitives_count,
			triangle_offset: self.visibility_info.triangle_count,
			meshlet_offset,
			acceleration_structure,
			primitives,
		});

		self.meshes_by_resource.insert(id.to_string(), mesh_id);

		self.visibility_info.vertex_count += vertex_count as u32;
		self.visibility_info.primitives_count += primitive_count as u32;
		self.visibility_info.triangle_count += triangle_count as u32;
		self.visibility_info.meshlet_count += total_meshlet_count as u32;

		Ok(mesh_id)
	}

	fn create_mesh_from_generator<'a>(
		&'a mut self,
		generator: &dyn MeshGenerator,
		device: &mut ghi::implementation::Frame,
	) -> Result<usize, ()> {
		let mesh_hash = generator.hash();

		if let Some(mesh_id) = self.meshes_by_generated_hash.get(&mesh_hash) {
			return Ok(*mesh_id);
		}

		let positions = generator.positions();
		let normals = generator.normals();
		let uvs = generator.uvs();
		let indices = generator.indices().iter().map(|&index| index as u16).collect::<Vec<_>>();

		if positions.len() != normals.len() || positions.len() != uvs.len() {
			log::error!(
				"Generated mesh attributes are inconsistent. The most likely cause is that the mesh generator returned mismatched vertex attribute counts."
			);
			return Err(());
		}

		let (vertex_indices, primitive_indices, meshlets) = Self::build_generated_meshlets(&indices)?;

		let vertex_offset = self.visibility_info.vertex_count as usize;
		let primitive_offset = self.visibility_info.primitives_count as usize;
		let triangle_offset = self.visibility_info.triangle_count as usize;
		let meshlet_offset = self.visibility_info.meshlet_count as usize;

		let vertex_positions_buffer = device.get_mut_buffer_slice(self.vertex_positions_buffer);
		vertex_positions_buffer[vertex_offset..][..positions.len()].copy_from_slice(&positions);

		let vertex_normals_buffer = device.get_mut_buffer_slice(self.vertex_normals_buffer);
		vertex_normals_buffer[vertex_offset..][..normals.len()].copy_from_slice(&normals);

		let vertex_uv_buffer = device.get_mut_buffer_slice(self.vertex_uvs_buffer);
		vertex_uv_buffer[vertex_offset..][..uvs.len()].copy_from_slice(&uvs);

		let indices_buffer = device.get_mut_buffer_slice(self.vertex_indices_buffer);
		indices_buffer[primitive_offset..][..vertex_indices.len()].copy_from_slice(&vertex_indices);

		let primitive_indices_buffer = device.get_mut_buffer_slice(self.primitive_indices_buffer);
		primitive_indices_buffer[triangle_offset..][..primitive_indices.len()].copy_from_slice(&primitive_indices);

		let meshlets_data_slice = device.get_mut_buffer_slice(self.meshlets_data_buffer);

		for (index, meshlet) in meshlets.iter().enumerate() {
			meshlets_data_slice[meshlet_offset + index] = *meshlet;
		}

		{
			let (index, v) = {
				let material_evaluation_materials = &mut self.material_evaluation_materials;
				let i = material_evaluation_materials.len() as u32;
				(
					i,
					material_evaluation_materials
						.entry("heyyy".to_string())
						.or_insert_with(|| Arc::new(OnceLock::new()))
						.clone(),
				)
			};

			let material = v.get_or_try_init(|| {
				let materials_buffer_slice = device.get_mut_buffer_slice(self.materials_data_buffer_handle);

				let material_data = materials_buffer_slice[index as usize];

				let root = besl::parse(
					&"main: fn () -> void {
	albedo = vec4f(1.0, 1.0, 1.0, 1.0);
}",
				)
				.unwrap();

				let shader_generator = VisibilityShaderGenerator::new(true, false, true, false, false, false, true, false);

				let object = json::object! { "variables": [] };

				let root = shader_generator.transform(root, &object);

				let root = besl::lex(root).unwrap();

				let main_node = root.get_main().ok_or(())?;

				let settings = ShaderGenerationSettings::compute(Extent::line(128));

				let fshader = if cfg!(target_os = "macos") {
					let mut source_generator = MSLShaderGenerator::new();
					let source = source_generator.generate(&settings, &main_node).map_err(|_| {
						log::error!("Failed to generate Metal shader source for material evaluation");
						()
					})?;
					let reflected_shader = SPIRVShaderGenerator::new().generate(&settings, &main_node).map_err(|e| {
						log::error!("{}", e);
						()
					})?;
					let bindings = reflected_shader
						.bindings()
						.iter()
						.map(map_shader_binding_to_shader_binding_descriptor)
						.collect::<Vec<_>>();

					device
						.create_shader(
							None,
							ghi::shader::Sources::MTL {
								source: source.as_str(),
								entry_point: "besl_main",
							},
							ghi::ShaderTypes::Compute,
							bindings,
						)
						.unwrap()
				} else {
					let shader = SPIRVShaderGenerator::new().generate(&settings, &main_node).map_err(|e| {
						log::error!("{}", e);
						()
					})?;
					let bindings = shader
						.bindings()
						.iter()
						.map(map_shader_binding_to_shader_binding_descriptor)
						.collect::<Vec<_>>();

					device
						.create_shader(
							None,
							ghi::shader::Sources::SPIRV(shader.binary()),
							ghi::ShaderTypes::Compute,
							bindings,
						)
						.unwrap()
				};

				let pipeline = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
					&[
						self.descriptor_set_layout,
						self.visibility_descriptor_set_layout,
						self.material_evaluation_descriptor_set_layout,
					],
					&[ghi::pipelines::PushConstantRange::new(0, 4)],
					ghi::ShaderParameter::new(&fshader, ghi::ShaderTypes::Compute),
				));

				Ok(RenderDescription {
					name: "heyyy".to_string(),
					index,
					pipeline,
					alpha: false,
					variant: RenderDescriptionVariants::Material { shaders: vec![] },
				})
			})?;
		}

		let mesh_id = self.meshes.len();

		self.meshes.push(MeshData {
			vertex_offset: self.visibility_info.vertex_count,
			primitive_offset: self.visibility_info.primitives_count,
			triangle_offset: self.visibility_info.triangle_count,
			meshlet_offset: self.visibility_info.meshlet_count,
			acceleration_structure: None,
			primitives: vec![MeshPrimitive {
				material_index: 0,
				meshlet_count: meshlets.len() as u32,
				meshlet_offset: self.visibility_info.meshlet_count,
				vertex_offset: self.visibility_info.vertex_count,
				primitive_offset: self.visibility_info.primitives_count,
				triangle_offset: self.visibility_info.triangle_count,
			}],
		});

		self.meshes_by_generated_hash.insert(mesh_hash, mesh_id);

		let vertex_count = positions.len();
		let primitive_count = vertex_indices.len();
		let triangle_count = primitive_indices.len();
		let total_meshlet_count = meshlets.len();

		self.visibility_info.vertex_count += vertex_count as u32;
		self.visibility_info.primitives_count += primitive_count as u32;
		self.visibility_info.triangle_count += triangle_count as u32;
		self.visibility_info.meshlet_count += total_meshlet_count as u32;

		device.sync_buffer(self.vertex_positions_buffer);
		device.sync_buffer(self.vertex_normals_buffer);
		device.sync_buffer(self.vertex_uvs_buffer);
		device.sync_buffer(self.vertex_indices_buffer);
		device.sync_buffer(self.primitive_indices_buffer);
		device.sync_buffer(self.meshlets_data_buffer);

		Ok(mesh_id)
	}

	fn build_generated_meshlets(indices: &[u16]) -> Result<(Vec<u16>, Vec<[u8; 3]>, Vec<ShaderMeshletData>), ()> {
		if indices.len() % 3 != 0 {
			log::error!(
				"Generated mesh indices are invalid. The most likely cause is that the mesh generator returned a triangle list whose index count is not divisible by three."
			);
			return Err(());
		}

		let mut vertex_indices = Vec::new();
		let mut primitive_indices = Vec::new();
		let mut meshlets = Vec::new();

		let mut meshlet_vertex_indices = Vec::<u16>::new();
		let mut meshlet_triangles = Vec::<[u8; 3]>::new();

		for triangle in indices.chunks_exact(3) {
			let unique_vertices = triangle
				.iter()
				.filter(|index| !meshlet_vertex_indices.contains(index))
				.count();

			if !meshlet_triangles.is_empty()
				&& (meshlet_vertex_indices.len() + unique_vertices > u8::MAX as usize
					|| meshlet_triangles.len() >= u8::MAX as usize)
			{
				Self::push_generated_meshlet(
					&mut vertex_indices,
					&mut primitive_indices,
					&mut meshlets,
					&mut meshlet_vertex_indices,
					&mut meshlet_triangles,
				)?;
			}

			let mut local_triangle = [0u8; 3];

			for (slot, index) in triangle.iter().enumerate() {
				let local_index = if let Some(existing) = meshlet_vertex_indices.iter().position(|value| value == index) {
					existing
				} else {
					meshlet_vertex_indices.push(*index);
					meshlet_vertex_indices.len() - 1
				};

				local_triangle[slot] = local_index as u8;
			}

			meshlet_triangles.push(local_triangle);
		}

		Self::push_generated_meshlet(
			&mut vertex_indices,
			&mut primitive_indices,
			&mut meshlets,
			&mut meshlet_vertex_indices,
			&mut meshlet_triangles,
		)?;

		Ok((vertex_indices, primitive_indices, meshlets))
	}

	fn push_generated_meshlet(
		vertex_indices: &mut Vec<u16>,
		primitive_indices: &mut Vec<[u8; 3]>,
		meshlets: &mut Vec<ShaderMeshletData>,
		meshlet_vertex_indices: &mut Vec<u16>,
		meshlet_triangles: &mut Vec<[u8; 3]>,
	) -> Result<(), ()> {
		if meshlet_triangles.is_empty() {
			return Ok(());
		}

		let primitive_offset = u16::try_from(vertex_indices.len()).map_err(|_| {
			log::error!(
				"Generated mesh exceeds primitive index limits. The most likely cause is that the visibility pipeline buffers are too small for the generated mesh data."
			);
		})?;
		let triangle_offset = u16::try_from(primitive_indices.len()).map_err(|_| {
			log::error!(
				"Generated mesh exceeds triangle index limits. The most likely cause is that the visibility pipeline buffers are too small for the generated mesh data."
			);
		})?;
		let primitive_count = u8::try_from(meshlet_vertex_indices.len()).map_err(|_| {
			log::error!(
				"Generated meshlet exceeds vertex limits. The most likely cause is that too many unique vertices were packed into a single meshlet."
			);
		})?;
		let triangle_count = u8::try_from(meshlet_triangles.len()).map_err(|_| {
			log::error!(
				"Generated meshlet exceeds triangle limits. The most likely cause is that too many triangles were packed into a single meshlet."
			);
		})?;

		vertex_indices.extend(meshlet_vertex_indices.iter().copied());
		primitive_indices.extend(meshlet_triangles.iter().copied());
		meshlets.push(ShaderMeshletData {
			primitive_offset,
			triangle_offset,
			primitive_count,
			triangle_count,
		});

		meshlet_vertex_indices.clear();
		meshlet_triangles.clear();

		Ok(())
	}

	fn create_material_resources<'a>(
		&'a mut self,
		resource: &mut resource_management::Reference<ResourceMaterial>,
		device: &mut ghi::implementation::Frame,
	) -> Result<u32, ()> {
		let (index, v) = {
			let material_evaluation_materials = &mut self.material_evaluation_materials;
			let i = material_evaluation_materials.len() as u32;
			(
				i,
				material_evaluation_materials
					.entry(resource.id().to_string())
					.or_insert_with(|| Arc::new(OnceLock::new()))
					.clone(),
			)
		};

		let material = v.get_or_try_init(|| {
			let material_id = resource.id().to_string();

			let shader_names = resource
				.resource()
				.shaders()
				.iter()
				.map(|shader| shader.id().to_string())
				.collect::<Vec<_>>();

			let parameters = &mut resource.resource_mut().parameters;

			let textures_indices = parameters
				.iter_mut()
				.map(|parameter| match parameter.value {
					Value::Image(ref mut image) => {
						let texture_manager = &mut self.texture_manager;
						texture_manager.load(image, device)
					}
					_ => None,
				})
				.collect::<Vec<_>>();

			let textures_indices = textures_indices
				.into_iter()
				.map(|v| {
					if let Some((name, image, sampler)) = v {
						let texture_index = {
							let images = &mut self.images;
							let index = images.len() as u32;
							match images.entry(name) {
								std::collections::hash_map::Entry::Occupied(v) => v.get().index,
								std::collections::hash_map::Entry::Vacant(v) => {
									v.insert(Image { index });
									index
								}
							}
						};

						device.write(&[ghi::descriptors::Write::combined_image_sampler_array(
							self.textures_binding,
							image,
							sampler,
							ghi::Layouts::Read,
							texture_index,
						)]);

						Some(texture_index)
					} else {
						None
					}
				})
				.collect::<Vec<_>>();

			match resource.resource().model.name.as_str() {
				"Visibility" => match resource.resource().model.pass.as_str() {
					"MaterialEvaluation" => {
						let pipeline_handle = self
							.pipeline_manager
							.load_material(
								&[
									self.descriptor_set_layout,
									self.visibility_descriptor_set_layout,
									self.material_evaluation_descriptor_set_layout,
								],
								&[ghi::pipelines::PushConstantRange::new(0, 4)],
								resource,
								device,
							)
							.unwrap();

						let materials_buffer_slice = device.get_mut_buffer_slice(self.materials_data_buffer_handle);

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
				},
				_ => {
					error!("Unknown material model");
					Err(())
				}
			}
		})?;

		return Ok(material.index);
	}

	/// Creates the needed GHI resource for the given material.
	/// Does nothing if the material has already been loaded.
	fn create_variant_resources<'s, 'a>(
		&'s mut self,
		mut resource: resource_management::Reference<ResourceVariant>,
		frame: &mut ghi::implementation::Frame,
	) -> Result<u32, ()> {
		let (index, v) = {
			let material_evaluation_materials = &mut self.material_evaluation_materials;
			let i = material_evaluation_materials.len() as u32;
			(
				i,
				material_evaluation_materials
					.entry(resource.id().to_string())
					.or_insert_with(|| Arc::new(OnceLock::new()))
					.clone(),
			)
		};

		let material = v.get_or_try_init(|| {
			let variant_id = resource.id().to_string();

			let specialization_constants: Vec<ghi::pipelines::SpecializationMapEntry> = resource
				.resource_mut()
				.variables
				.iter()
				.enumerate()
				.filter_map(|(i, variable)| match &variable.value {
					Value::Scalar(scalar) => {
						ghi::pipelines::SpecializationMapEntry::new(i as u32, "f32".to_string(), *scalar).into()
					}
					Value::Vector3(value) => {
						ghi::pipelines::SpecializationMapEntry::new(i as u32, "vec3f".to_string(), *value).into()
					}
					Value::Vector4(value) => {
						ghi::pipelines::SpecializationMapEntry::new(i as u32, "vec4f".to_string(), *value).into()
					}
					_ => None,
				})
				.collect();

			let pipeline = self.pipeline_manager.load_variant(
				&[
					self.descriptor_set_layout,
					self.visibility_descriptor_set_layout,
					self.material_evaluation_descriptor_set_layout,
				],
				&[ghi::pipelines::PushConstantRange::new(0, 4)],
				&specialization_constants,
				&mut resource,
				frame,
			);

			let pipeline = pipeline.unwrap();

			let variant = resource.resource_mut();

			let material_id = variant.material.id().to_string();

			self.create_material_resources(&mut variant.material, frame)?;

			let textures_indices = {
				let texture_manager = &mut self.texture_manager;
				variant
					.variables
					.iter_mut()
					.map(|parameter| match parameter.value {
						Value::Image(ref mut image) => texture_manager.load(image, frame),
						_ => None,
					})
					.collect::<Vec<_>>()
			};

			let textures_indices = textures_indices
				.into_iter()
				.map(|v| {
					if let Some((name, image, sampler)) = v {
						let texture_index = {
							let images = &mut self.images;
							let index = images.len() as u32;
							match images.entry(name) {
								std::collections::hash_map::Entry::Occupied(v) => v.get().index,
								std::collections::hash_map::Entry::Vacant(v) => {
									v.insert(Image { index });
									index
								}
							}
						};

						frame.write(&[ghi::descriptors::Write::combined_image_sampler_array(
							self.textures_binding,
							image,
							sampler,
							ghi::Layouts::Read,
							texture_index,
						)]);

						Some(texture_index)
					} else {
						None
					}
				})
				.collect::<Vec<_>>();

			let alpha = variant.alpha_mode == resource_management::types::AlphaMode::Blend;

			let materials_buffer_slice = frame.get_mut_buffer_slice(self.materials_data_buffer_handle);

			frame.sync_buffer(self.materials_data_buffer_handle);

			let material_data = materials_buffer_slice.as_mut_ptr() as *mut MaterialData;

			let material_data = unsafe { material_data.add(index as usize).as_mut().unwrap() };

			for (i, e) in textures_indices.iter().enumerate() {
				material_data.textures[i] = e.unwrap_or(0xFFFFFFFFu32) as u32;
			}

			Ok(RenderDescription {
				name: variant_id,
				index,
				pipeline,
				alpha,
				variant: RenderDescriptionVariants::Variant {},
			})
		})?;

		return Ok(material.index);
	}

	pub fn create_light(&mut self, light: Lights) {
		self.lights.push(light);
	}

	/// Uploads the current scene lights to the GPU buffer used by material evaluation.
	fn write_light_data(&self, frame: &mut ghi::implementation::Frame, shadow_light_index: Option<usize>) {
		let lighting_data = frame.get_mut_buffer_slice(self.light_data_buffer);
		let light_count = self.lights.len().min(MAX_LIGHTS);

		if self.lights.len() > MAX_LIGHTS {
			warn!(
				"Too many lights for the visibility pipeline. The most likely cause is that the scene contains more lights than the GPU buffer can hold."
			);
		}

		lighting_data.count = light_count as u32;

		for (index, light) in self.lights.iter().take(light_count).enumerate() {
			lighting_data.lights[index] = Self::make_light_data(light, shadow_light_index == Some(index));
		}

		frame.sync_buffer(self.light_data_buffer);
	}

	fn make_light_data(light: &Lights, casts_shadow: bool) -> LightData {
		let mut cascades = [0; 8];

		if casts_shadow {
			for (index, cascade) in cascades.iter_mut().take(SHADOW_CASCADE_COUNT).enumerate() {
				*cascade = (index + 1) as u32;
			}
		}

		match light {
			Lights::Direction(light) => LightData {
				position: light.direction,
				color: light.color,
				light_type: 68,
				cascades,
			},
			Lights::Point(light) => LightData {
				position: light.position,
				color: light.color,
				light_type: 0,
				cascades: [0; 8],
			},
		}
	}

	fn make_shader_view_data(view: View) -> ShaderViewData {
		let view_projection = view.view_projection();

		ShaderViewData {
			view: view.view(),
			projection: view.projection(),
			view_projection,
			inverse_view: view.view().inverse(),
			inverse_projection: view.projection().inverse(),
			inverse_view_projection: view_projection.inverse(),
			fov: view.fov(),
			near: view.near(),
			far: view.far(),
		}
	}
}

impl SceneManager for VisibilityWorldRenderDomain {
	fn prepare(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		viewports: &[Viewport],
	) -> Option<Vec<Box<dyn RenderPassFunction>>> {
		let main_viewport = viewports
			.iter()
			.find(|viewport| viewport.index() == 0)
			.copied()
			.or_else(|| viewports.first().copied());
		let shadow_light = self.lights.iter().enumerate().find_map(|(index, light)| match light {
			Lights::Direction(light) => Some((index, light.direction)),
			Lights::Point(_) => None,
		});
		let shadow_light_index = if main_viewport.is_some() {
			shadow_light.map(|(index, _)| index)
		} else {
			None
		};

		if let Some(main_viewport) = main_viewport {
			let main_view = main_viewport.view();
			let main_view_data = Self::make_shader_view_data(main_view);
			let views_data_buffer = frame.get_mut_dynamic_buffer_slice(self.views_data_buffer_handle);

			for view_data in views_data_buffer.iter_mut() {
				*view_data = main_view_data;
			}

			if let Some((_, light_direction)) = shadow_light {
				for (cascade_index, (cascade_view, cascade_far)) in
					csm::make_csm_views(main_view, light_direction, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION)
						.into_iter()
						.zip(
							csm::make_cascade_split_ranges(main_view, SHADOW_CASCADE_COUNT)
								.into_iter()
								.map(|(_, far)| far),
						)
						.enumerate()
				{
					let mut cascade_view_data = Self::make_shader_view_data(cascade_view);
					cascade_view_data.far = cascade_far;
					views_data_buffer[cascade_index + 1] = cascade_view_data;
				}
			}
		}

		let meshes_data_buffer = frame.get_mut_dynamic_buffer_slice(self.meshes_data_buffer);

		for (index, (entity, shader_mesh)) in self.render_entities.iter().enumerate() {
			meshes_data_buffer[index] = ShaderMesh {
				model: entity.transform().get_matrix(),
				..*shader_mesh
			};
		}

		self.write_light_data(frame, shadow_light_index);

		let opaque_materials = self
			.material_evaluation_materials
			.values()
			.filter_map(|v| v.get())
			.filter(|v| v.alpha == false)
			.map(|v| (v.name.clone(), v.index, v.pipeline))
			.collect::<Vec<_>>();
		let transparent_materials = self
			.material_evaluation_materials
			.values()
			.filter_map(|v| v.get())
			.filter(|v| v.alpha == true)
			.map(|v| (v.name.clone(), v.index, v.pipeline))
			.collect::<Vec<_>>();

		let viewport_x_rp = viewports.iter().map(|v| (v, &self.views[v.index()].1));

		let commands: Vec<Box<dyn RenderPassFunction>> = viewport_x_rp
			.map(|(v, r)| {
				Box::new(r.prepare(
					frame,
					v,
					&self.render_info.instances,
					&opaque_materials,
					&transparent_materials,
					shadow_light_index.is_some(),
				)) as Box<dyn RenderPassFunction>
			})
			.collect::<Vec<_>>();

		Some(commands)
	}

	fn create_view(&mut self, id: usize, render_pass_builder: &mut RenderPassBuilder) {
		let diffuse_target = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				ghi::Formats::RGBA16UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferDestination,
			)
			.name("Diffuse"),
		);
		let specular_target = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				ghi::Formats::RGBA16UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferDestination,
			)
			.name("Specular"),
		);
		let depth_target = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::DepthStencil | ghi::Uses::Image).name("Depth"),
		);
		let primitive_index = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Storage).name("primitive index"),
		);
		let instance_id = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Storage).name("instance_id"),
		);

		let device = render_pass_builder.device();

		let visibility_passes_descriptor_set =
			device.create_descriptor_set(Some("Visibility Descriptor Set"), &self.visibility_descriptor_set_layout);

		let material_count_buffer = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Count")
				.device_accesses(ghi::DeviceAccesses::HostOnly),
		);

		let material_xy = device.build_buffer(ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination));

		let material_evaluation_dispatches = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination | ghi::Uses::Indirect)
				.name("Material Evaluation Dipatches")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_offset_buffer = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Offset")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_offset_scratch_buffer = device.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Offset Scratch")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let ao_map = device.build_dynamic_image(
			ghi::image::Builder::new(
				ghi::Formats::R8UNORM,
				ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferDestination,
			)
			.name("Occlusion Map")
			.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let shadow_map = device.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name("Shadow Map")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.array_layers(NonZeroU32::new(SHADOW_CASCADE_COUNT as u32)),
		);
		let ibl_cubemap = device.build_image(
			ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name("IBL Cubemap")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.extent(Extent::square(1))
				.array_layers(NonZeroU32::new(6)),
		);
		let sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let depth_sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);

		let _ = device.create_descriptor_binding(
			self.material_evaluation_descriptor_set,
			ghi::BindingConstructor::image(&diffuse_binding_template, ghi::BaseImageHandle::from(diffuse_target)),
		);
		let _ = device.create_descriptor_binding(
			self.material_evaluation_descriptor_set,
			ghi::BindingConstructor::image(&specular_binding_template, ghi::BaseImageHandle::from(specular_target)),
		);
		let _ = device.create_descriptor_binding(
			self.material_evaluation_descriptor_set,
			ghi::BindingConstructor::buffer(&lighting_data_binding_template, self.light_data_buffer.into()),
		);
		let _ = device.create_descriptor_binding(
			self.material_evaluation_descriptor_set,
			ghi::BindingConstructor::buffer(&materials_data_binding_template, self.materials_data_buffer_handle.into()),
		);
		let _ = device.create_descriptor_binding(
			self.material_evaluation_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&ao_map_binding_template,
				ao_map,
				sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			self.material_evaluation_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&shadow_map_binding_template,
				shadow_map,
				depth_sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			self.material_evaluation_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&visibility_depth_binding_template,
				ghi::BaseImageHandle::from(depth_target),
				depth_sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			self.material_evaluation_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&ibl_cubemap_binding_template,
				ibl_cubemap,
				sampler.clone(),
				ghi::Layouts::Read,
			),
		);

		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_COUNT_BINDING, material_count_buffer.into()),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_OFFSET_BINDING, material_offset_buffer.into()),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_OFFSET_SCRATCH_BINDING, material_offset_scratch_buffer.into()),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_EVALUATION_DISPATCHES_BINDING, material_evaluation_dispatches.into()),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::buffer(&MATERIAL_XY_BINDING, material_xy.into()),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::image(&TRIANGLE_INDEX_BINDING, ghi::BaseImageHandle::from(primitive_index)),
		);
		let _ = device.create_descriptor_binding(
			visibility_passes_descriptor_set,
			ghi::BindingConstructor::image(&INSTANCE_ID_BINDING, ghi::BaseImageHandle::from(instance_id)),
		);

		render_pass_builder.alias("Depth", "depth");
		render_pass_builder.alias("Diffuse", "main");

		let render_pass = VisibilityPipelineRenderPass::new(
			render_pass_builder.device(),
			self.descriptor_set_layout,
			self.visibility_descriptor_set_layout,
			self.descriptor_set,
			visibility_passes_descriptor_set,
			self.material_evaluation_descriptor_set,
			material_count_buffer,
			ghi::BaseImageHandle::from(diffuse_target),
			ghi::BaseImageHandle::from(specular_target),
			ao_map.into(),
			shadow_map.into(),
			ibl_cubemap.into(),
			ghi::BaseImageHandle::from(depth_target),
			ghi::BaseImageHandle::from(primitive_index),
			ghi::BaseImageHandle::from(instance_id),
			material_xy,
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
		);

		self.views.push((id, render_pass));
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
	/// The number of primitives in the meshlet
	/// Primitives are meshlet local indices
	primitive_count: u8,
	// The number of triangles in the meshlet
	triangle_count: u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct ShaderMesh {
	model: Matrix4,
	material_index: u32,
	/// The position into the vertex components data (positions, normals, uvs, ..) buffer this instance's data starts
	/// Also, the position into the vertex indices buffer this instance's data starts
	base_vertex_index: u32,
	base_primitive_index: u32,
	base_triangle_index: u32,
	base_meshlet_index: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LightingData {
	pub count: u32,
	pub lights: [LightData; MAX_LIGHTS],
}

#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) struct ShaderViewData {
	pub(crate) view: Matrix4,
	pub(crate) projection: Matrix4,
	pub(crate) view_projection: Matrix4,
	pub(crate) inverse_view: Matrix4,
	pub(crate) inverse_projection: Matrix4,
	pub(crate) inverse_view_projection: Matrix4,
	pub(crate) fov: [f32; 2],
	pub(crate) near: f32,
	pub(crate) far: f32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct LightData {
	pub position: Vector3,
	pub color: Vector3,
	pub light_type: u8,
	pub cascades: [u32; 8],
}

#[repr(C)]
#[derive(Copy, Clone)]
struct MaterialData {
	textures: [u32; 16],
}

impl WorldRenderDomain for VisibilityWorldRenderDomain {
	fn get_descriptor_set_template(&self) -> ghi::DescriptorSetTemplateHandle {
		self.descriptor_set_layout
	}

	fn get_descriptor_set(&self) -> ghi::DescriptorSetHandle {
		self.descriptor_set
	}

	fn get_visibility_info(&self) -> VisibilityInfo {
		self.visibility_info
	}
}

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
	Build { mesh_handle: String },
	Update {},
}

struct RayTracing {
	top_level_acceleration_structure: ghi::TopLevelAccelerationStructureHandle,
	descriptor_set_template: ghi::DescriptorSetTemplateHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	pipeline: ghi::PipelineHandle,

	ray_gen_sbt_buffer: ghi::BaseBufferHandle,
	miss_sbt_buffer: ghi::BaseBufferHandle,
	hit_sbt_buffer: ghi::BaseBufferHandle,

	shadow_map_resolution: Extent,
	shadow_map: ghi::BaseImageHandle,

	instances_buffer: ghi::BaseBufferHandle,
	scratch_buffer: ghi::BaseBufferHandle,

	pending_meshes: Vec<MeshState>,
}

enum RenderDescriptionVariants {
	Material { shaders: Vec<String> },
	Variant {},
}

struct RenderDescription {
	index: u32,
	pipeline: ghi::PipelineHandle,
	name: String,
	alpha: bool,
	variant: RenderDescriptionVariants,
}

#[derive(Clone, Copy)]
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

use crate::ghi;

#[derive(Clone, Copy, Default)]
pub struct VisibilityInfo {
	pub instance_count: u32,
	pub triangle_count: u32,
	pub meshlet_count: u32,
	pub vertex_count: u32,
	pub primitives_count: u32,
}

pub trait WorldRenderDomain {
	fn get_descriptor_set_template(&self) -> ghi::DescriptorSetTemplateHandle;
	fn get_descriptor_set(&self) -> ghi::DescriptorSetHandle;
	fn get_visibility_info(&self) -> VisibilityInfo;
}
