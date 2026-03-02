use ::core::slice::SlicePattern;
use std::borrow::Borrow;
use std::cell::{OnceCell, RefCell};
use std::collections::VecDeque;
use std::mem::transmute;
use std::num::NonZeroU32;
use std::ops::{Deref, DerefMut};
use std::sync::OnceLock;

use crate::core::listener::Listener;
use crate::core::{Entity, EntityHandle};
use crate::rendering::common_shader_generator::{CommonShaderGenerator, CommonShaderScope};
use crate::rendering::lights::{DirectionalLight, Light, PointLight};
use crate::rendering::mesh::generator::MeshGenerator;
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::pipelines::visibility::render_pass::VisibilityPipelineRenderPass;
use crate::rendering::pipelines::visibility::{
	INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING, MATERIAL_EVALUATION_DISPATCHES_BINDING, MATERIAL_OFFSET_BINDING,
	MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING, MAX_INSTANCES, MAX_LIGHTS, MAX_MATERIALS, MAX_MESHLETS,
	MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES, MESHLET_DATA_BINDING, MESH_DATA_BINDING, PRIMITIVE_INDICES_BINDING,
	TEXTURES_BINDING, TRIANGLE_INDEX_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING,
	VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
};
use crate::rendering::render_pass::{FramePrepare, RenderPass, RenderPassBuilder, RenderPassFunction, RenderPassReturn};
use crate::rendering::renderable::mesh::MeshSource;
use crate::rendering::scene_manager::SceneManager;
use crate::rendering::texture_manager::TextureManager;
use crate::rendering::view::View;
use ghi::device::Device as _;
use ghi::frame::Frame as _;
use ghi::{
	command_buffer::{
		BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
		CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	graphics_hardware_interface, raster_pipeline, ImageHandle,
};
use log::error;
use math::{Matrix4, Vector3};
use resource_management::asset::material_asset_handler::ProgramGenerator;
use resource_management::glsl_shader_generator::GLSLShaderGenerator;
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
use crate::gameplay::transform::TransformationUpdate;
use crate::gameplay::Transformable as _;
use crate::rendering::{
	csm, make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor, mesh, world_render_domain,
	RenderableMesh, Viewport,
};
use crate::{
	camera::{self},
	resource_management::{self},
};

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
	/// Loaded images.
	images: RwLock<HashMap<String, Image>>,
	/// Texture manager.
	texture_manager: Arc<RwLock<TextureManager>>,
	/// Pipeline manager.
	pipeline_manager: PipelineManager,
	/// Mapping from mesh resource ID to mesh index.
	mesh_resources: HashMap<String, u32>,
	/// Material evaluation materials.
	material_evaluation_materials: RwLock<HashMap<String, Arc<OnceLock<RenderDescription>>>>,
	/// Base pipeline layout handle.
	base_pipeline_layout: ghi::PipelineLayoutHandle,
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
	/// Pipeline layout for the visibility pass.
	visibility_pass_pipeline_layout: ghi::PipelineLayoutHandle,
	/// Descriptor set for the visibility pass.
	visibility_passes_descriptor_set: ghi::DescriptorSetHandle,
	material_evaluation_descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
	material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
	material_evaluation_pipeline_layout: ghi::PipelineLayoutHandle,
	/// Buffer containing lighting data.
	light_data_buffer: ghi::BufferHandle<LightingData>,
	/// Lights in the scene.
	lights: Vec<EntityHandle<dyn Light>>,
	/// Information about the current render.
	render_info: RenderInfo,
	/// Queue of render entities pending to be processed.
	pending_render_entities: VecDeque<EntityHandle<dyn RenderableMesh>>,
	/// Views
	views: Vec<(usize, VisibilityPipelineRenderPass)>,
}

impl VisibilityWorldRenderDomain {
	pub fn new(device: &mut ghi::Device, texture_manager: Arc<RwLock<TextureManager>>) -> Self {
		// Initialize the extent to 0 to allocate memory lazily.
		let extent = Extent::square(0);

		let vertex_positions_buffer_handle = device.create_buffer(
			Some("Visibility Vertex Positions Buffer"),
			ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage,
			ghi::DeviceAccesses::HostToDevice,
		);
		let vertex_normals_buffer_handle = device.create_buffer(
			Some("Visibility Vertex Normals Buffer"),
			ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage,
			ghi::DeviceAccesses::HostToDevice,
		);
		let vertex_uv_buffer_handle = device.create_buffer(
			Some("Visibility Vertex UV Buffer"),
			ghi::Uses::Vertex | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage,
			ghi::DeviceAccesses::HostToDevice,
		);
		let vertex_indices_buffer_handle = device.create_buffer(
			Some("Visibility Index Buffer"),
			ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage,
			ghi::DeviceAccesses::HostToDevice,
		);
		let primitive_indices_buffer_handle = device.create_buffer(
			Some("Visibility Primitive Indices Buffer"),
			ghi::Uses::Index | ghi::Uses::AccelerationStructureBuild | ghi::Uses::Storage,
			ghi::DeviceAccesses::HostToDevice,
		);
		let meshlets_data_buffer = device.create_buffer::<[ShaderMeshletData; MAX_MESHLETS]>(
			Some("Visibility Meshlets Data"),
			ghi::Uses::Storage,
			ghi::DeviceAccesses::HostToDevice,
		);

		let views_data_buffer_handle = device.create_dynamic_buffer::<[ShaderViewData; 8]>(
			Some("Visibility Views Data"),
			ghi::Uses::Storage,
			ghi::DeviceAccesses::HostToDevice,
		);

		let meshes_data_buffer = device.create_dynamic_buffer::<[ShaderMesh; MAX_INSTANCES]>(
			Some("Visibility Meshes Data"),
			ghi::Uses::Storage,
			ghi::DeviceAccesses::HostToDevice,
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

		// Push constant:
		// 4 bytes for the view index
		// 4 bytes for the mesh index
		let pipeline_layout_handle =
			device.create_pipeline_layout(&[descriptor_set_layout], &[ghi::PushConstantRange::new(0, 4 + 4)]);

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
		let visibility_pass_pipeline_layout =
			device.create_pipeline_layout(&[descriptor_set_layout, visibility_descriptor_set_layout], &[]);
		let visibility_passes_descriptor_set =
			device.create_descriptor_set(Some("Visibility Descriptor Set"), &visibility_descriptor_set_layout);

		let light_data_buffer = device.create_buffer::<LightingData>(
			Some("Light Data"),
			ghi::Uses::Storage | ghi::Uses::TransferDestination,
			ghi::DeviceAccesses::HostToDevice,
		);

		let lighting_data = device.get_mut_buffer_slice(light_data_buffer);

		lighting_data.count = 0; // Initially, no lights

		let materials_data_buffer_handle = device.create_buffer::<[MaterialData; MAX_MATERIALS]>(
			Some("Materials Data"),
			ghi::Uses::Storage | ghi::Uses::TransferDestination,
			ghi::DeviceAccesses::HostToDevice,
		);

		let bindings = [
			ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
			ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
			ghi::DescriptorSetBindingTemplate::new(2, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
			ghi::DescriptorSetBindingTemplate::new(3, ghi::DescriptorType::StorageImage, ghi::Stages::COMPUTE),
			ghi::DescriptorSetBindingTemplate::new(4, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
			ghi::DescriptorSetBindingTemplate::new(5, ghi::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE),
			ghi::DescriptorSetBindingTemplate::new(10, ghi::DescriptorType::CombinedImageSampler, ghi::Stages::COMPUTE),
			ghi::DescriptorSetBindingTemplate::new_array(
				11,
				ghi::DescriptorType::CombinedImageSampler,
				ghi::Stages::COMPUTE,
				1,
			),
		];

		let sampler = device.create_sampler(
			ghi::FilteringModes::Linear,
			ghi::SamplingReductionModes::WeightedAverage,
			ghi::FilteringModes::Linear,
			ghi::SamplerAddressingModes::Clamp,
			None,
			0f32,
			0f32,
		);
		let depth_sampler = device.create_sampler(
			ghi::FilteringModes::Linear,
			ghi::SamplingReductionModes::WeightedAverage,
			ghi::FilteringModes::Linear,
			ghi::SamplerAddressingModes::Border {},
			None,
			0f32,
			0f32,
		);

		let material_evaluation_descriptor_set_layout =
			device.create_descriptor_set_template(Some("Material Evaluation Set Layout"), &bindings);
		let material_evaluation_descriptor_set = device.create_descriptor_set(
			Some("Material Evaluation Descriptor Set"),
			&material_evaluation_descriptor_set_layout,
		);

		let material_evaluation_pipeline_layout = device.create_pipeline_layout(
			&[
				descriptor_set_layout,
				visibility_descriptor_set_layout,
				material_evaluation_descriptor_set_layout,
			],
			&[ghi::PushConstantRange::new(0, 4 + 4)],
		);

		Self {
			render_entities: Vec::with_capacity(512),

			visibility_info: VisibilityInfo::default(),

			meshes: Vec::with_capacity(1024),
			meshes_by_resource: HashMap::with_capacity(1024),

			images: RwLock::new(HashMap::with_capacity(1024)),

			texture_manager,
			pipeline_manager: PipelineManager::new(),

			mesh_resources: HashMap::new(),

			material_evaluation_materials: RwLock::new(HashMap::new()),

			// Visibility
			base_pipeline_layout: pipeline_layout_handle,

			vertex_positions_buffer: vertex_positions_buffer_handle,
			vertex_normals_buffer: vertex_normals_buffer_handle,
			vertex_uvs_buffer: vertex_uv_buffer_handle,

			vertex_indices_buffer: vertex_indices_buffer_handle,
			primitive_indices_buffer: primitive_indices_buffer_handle,

			descriptor_set_layout,
			descriptor_set,

			textures_binding,

			views_data_buffer_handle,

			meshes_data_buffer,
			meshlets_data_buffer,

			visibility_pass_pipeline_layout,
			visibility_passes_descriptor_set,

			material_evaluation_descriptor_set_layout,
			material_evaluation_descriptor_set,
			material_evaluation_pipeline_layout,

			light_data_buffer,
			materials_data_buffer_handle,

			lights: Vec::new(),

			render_info: RenderInfo {
				instances: Vec::with_capacity(4096),
			},

			pending_render_entities: VecDeque::with_capacity(64),

			views: Vec::with_capacity(4),
		}
	}

	/// Creates the needed GHI resource for the given mesh.
	/// Does nothing if the mesh has already been loaded.
	fn create_mesh_resources<'a, 's: 'a>(&'s mut self, id: &'a str, device: &mut ghi::Device) -> Result<usize, ()> {
		if let Some(entry) = self.meshes_by_resource.get(id) {
			return Ok(*entry);
		}

		let meshlet_stream_buffer = vec![0u8; 1024 * 8];

		let mut resource_request: Reference<ResourceMesh> = {
			return Err(());
			// let resource_manager = self.resource_manager.read();
			// let Ok(resource_request) = resource_manager.request(id) else {
			// 	log::error!("Failed to load mesh resource {}", id);
			// 	return Err(());
			// };
			// resource_request
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

		let vertex_positions_buffer = device.get_mut_buffer_slice(self.vertex_positions_buffer);
		let vertex_normals_buffer = device.get_mut_buffer_slice(self.vertex_normals_buffer);
		let vertex_uv_buffer = device.get_mut_buffer_slice(self.vertex_uvs_buffer);
		let vertex_indices_buffer = device.get_mut_buffer_slice(self.vertex_indices_buffer);
		let primitive_indices_buffer = device.get_mut_buffer_slice(self.primitive_indices_buffer);

		let mut buffer_allocator = utils::BufferAllocator::new(&mut meshlet_stream_buffer);

		let streams = vec![
			resource_management::stream::StreamMut::new(
				"Vertex.Position",
				&mut vertex_positions_buffer[self.visibility_info.vertex_count as usize..positions_stream.count()],
			),
			resource_management::stream::StreamMut::new(
				"Vertex.Normal",
				&mut vertex_normals_buffer[self.visibility_info.vertex_count as usize..normals_stream.count()],
			),
			resource_management::stream::StreamMut::new(
				"Vertex.UV",
				&mut vertex_uv_buffer[self.visibility_info.vertex_count as usize..uvs_stream.count()],
			),
			resource_management::stream::StreamMut::new(
				"VertexIndices",
				&mut vertex_indices_buffer[self.visibility_info.primitives_count as usize..vertex_indices_stream.count()],
			),
			resource_management::stream::StreamMut::new(
				"MeshletIndices",
				&mut primitive_indices_buffer[self.visibility_info.triangle_count as usize..meshlet_indices_stream.count()],
			), // TODO: this might be wrong
			resource_management::stream::StreamMut::new("Meshlets", buffer_allocator.take(meshlets_stream.size)),
		];

		let Ok(load_target) = resource_request.load(streams.into()) else {
			log::warn!("Failed to load mesh resources");
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

		let acceleration_structure = if false {
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

		let vertex_count = positions_stream.count();
		let primitive_count = vertex_indices_stream.count();
		let triangle_count = triangle_indices_stream.count() / 3;

		self.visibility_info.vertex_count += vertex_count as u32;
		self.visibility_info.primitives_count += primitive_count as u32;
		self.visibility_info.triangle_count += triangle_count as u32;
		self.visibility_info.meshlet_count += total_meshlet_count as u32;

		Ok(mesh_id)
	}

	fn create_mesh_from_generator<'a>(
		&'a mut self,
		generator: &dyn MeshGenerator,
		device: &mut ghi::Device,
	) -> Result<usize, ()> {
		let positions = generator.positions();
		let normals = generator.normals();
		let uvs = generator.uvs();
		let indices = generator.indices().iter().map(|&i| i as u16).collect::<Vec<_>>();
		let Some(meshlet_indices) = generator.meshlet_indices() else {
			log::warn!("Need mesh to contain meshlet indices to be used with this render domain");
			return Err(());
		};

		if meshlet_indices.len() > 192 {
			log::warn!("Meshlet indices exceed maximum limit");
			return Err(());
		}

		let meshlet_indices = meshlet_indices
			.iter()
			.map_windows(|&[a, b, c]| [*a, *b, *c])
			.collect::<Vec<_>>();

		let vertex_positions_buffer = device.get_mut_buffer_slice(self.vertex_positions_buffer);
		let vertex_normals_buffer = device.get_mut_buffer_slice(self.vertex_normals_buffer);
		let vertex_uv_buffer = device.get_mut_buffer_slice(self.vertex_uvs_buffer);
		let indices_buffer = device.get_mut_buffer_slice(self.vertex_indices_buffer);
		let primitive_indices_buffer = device.get_mut_buffer_slice(self.primitive_indices_buffer);
		let meshlets_data_slice = device.get_mut_buffer_slice(self.meshlets_data_buffer);

		vertex_positions_buffer[self.visibility_info.vertex_count as usize..positions.len()].copy_from_slice(&positions);
		vertex_normals_buffer[self.visibility_info.vertex_count as usize..normals.len()].copy_from_slice(&normals);
		vertex_uv_buffer[self.visibility_info.vertex_count as usize..uvs.len()].copy_from_slice(&uvs);
		indices_buffer[self.visibility_info.vertex_count as usize..indices.len()].copy_from_slice(&indices);
		primitive_indices_buffer[self.visibility_info.primitives_count as usize..meshlet_indices.len()]
			.copy_from_slice(&meshlet_indices);

		let meshlets = [ShaderMeshletData {
			primitive_offset: 0,
			triangle_offset: 0,
			primitive_count: indices.len() as u8,
			triangle_count: (indices.len() / 3) as u8,
		}];

		meshlets_data_slice[self.visibility_info.meshlet_count as usize + 0] = meshlets[0];

		{
			let (index, v) = {
				let mut material_evaluation_materials = self.material_evaluation_materials.write();
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

				let shader = SPIRVShaderGenerator::new()
					.generate(&ShaderGenerationSettings::compute(Extent::line(128)), &main_node)
					.map_err(|e| {
						log::error!("{}", e);
						()
					})?;

				let bindings = shader.bindings().iter().map(map_shader_binding_to_shader_binding_descriptor);

				let fshader = device
					.create_shader(
						None,
						ghi::ShaderSource::SPIRV(&shader.binary()),
						ghi::ShaderTypes::Compute,
						bindings,
					)
					.unwrap();

				let pipeline = device.create_compute_pipeline(
					self.material_evaluation_pipeline_layout,
					ghi::ShaderParameter::new(&fshader, ghi::ShaderTypes::Compute),
				);

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
				meshlet_count: 1,
				meshlet_offset: self.visibility_info.meshlet_count,
				vertex_offset: self.visibility_info.vertex_count,
				primitive_offset: self.visibility_info.primitives_count,
				triangle_offset: self.visibility_info.triangle_count,
			}],
		});

		let vertex_count = positions.len();
		let primitive_count = indices.len();
		let triangle_count = primitive_count / 3;
		let total_meshlet_count = 1;

		self.visibility_info.vertex_count += vertex_count as u32;
		self.visibility_info.primitives_count += primitive_count as u32;
		self.visibility_info.triangle_count += triangle_count as u32;
		self.visibility_info.meshlet_count += total_meshlet_count as u32;

		Ok(mesh_id)
	}

	fn create_material_resources<'a>(
		&'a self,
		resource: &mut resource_management::Reference<ResourceMaterial>,
		device: &mut ghi::Device,
	) -> Result<u32, ()> {
		let (index, v) = {
			let mut material_evaluation_materials = self.material_evaluation_materials.write();
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
						let texture_manager = self.texture_manager.clone();
						let mut texture_manager = texture_manager.write();
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
							let mut images = self.images.write();
							let index = images.len() as u32;
							match images.entry(name) {
								std::collections::hash_map::Entry::Occupied(v) => v.get().index,
								std::collections::hash_map::Entry::Vacant(v) => {
									v.insert(Image { index });
									index
								}
							}
						};

						device.write(&[ghi::DescriptorWrite::combined_image_sampler_array(
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
							.load_material(self.material_evaluation_pipeline_layout, resource, device)
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
		&'s self,
		mut resource: resource_management::Reference<ResourceVariant>,
		device: &mut ghi::Device,
	) -> Result<u32, ()> {
		let (index, v) = {
			let mut material_evaluation_materials = self.material_evaluation_materials.write();
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

			let specialization_constants: Vec<ghi::SpecializationMapEntry> = resource
				.resource_mut()
				.variables
				.iter()
				.enumerate()
				.filter_map(|(i, variable)| match &variable.value {
					Value::Scalar(scalar) => ghi::SpecializationMapEntry::new(i as u32, "f32".to_string(), *scalar).into(),
					Value::Vector3(value) => ghi::SpecializationMapEntry::new(i as u32, "vec3f".to_string(), *value).into(),
					Value::Vector4(value) => ghi::SpecializationMapEntry::new(i as u32, "vec4f".to_string(), *value).into(),
					_ => None,
				})
				.collect();

			let pipeline = self.pipeline_manager.load_variant(
				self.material_evaluation_pipeline_layout,
				&specialization_constants,
				&mut resource,
				device,
			);

			let pipeline = pipeline.unwrap();

			let variant = resource.resource_mut();

			let material_id = variant.material.id().to_string();

			self.create_material_resources(&mut variant.material, device)?;

			let textures_indices = {
				let texture_manager = self.texture_manager.clone();
				variant
					.variables
					.iter_mut()
					.map(|parameter| match parameter.value {
						Value::Image(ref mut image) => {
							let mut texture_manager = texture_manager.write();
							texture_manager.load(image, device)
						}
						_ => None,
					})
					.collect::<Vec<_>>()
			};

			let textures_indices = textures_indices
				.into_iter()
				.map(|v| {
					if let Some((name, image, sampler)) = v {
						let texture_index = {
							let mut images = self.images.write();
							let index = images.len() as u32;
							match images.entry(name) {
								std::collections::hash_map::Entry::Occupied(v) => v.get().index,
								std::collections::hash_map::Entry::Vacant(v) => {
									v.insert(Image { index });
									index
								}
							}
						};

						device.write(&[ghi::DescriptorWrite::combined_image_sampler_array(
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

			let materials_buffer_slice = device.get_mut_buffer_slice(self.materials_data_buffer_handle);

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
}

impl SceneManager for VisibilityWorldRenderDomain {
	fn prepare(
		&mut self,
		frame: &mut ghi::Frame,
		viewports: &[Viewport],
		_transforms_listener: &mut dyn Listener<TransformationUpdate>,
	) -> Option<Vec<Box<dyn RenderPassFunction>>> {
		let opaque_materials = self
			.material_evaluation_materials
			.read()
			.values()
			.filter_map(|v| v.get())
			.filter(|v| v.alpha == false)
			.map(|v| (v.name.clone(), v.index, v.pipeline))
			.collect::<Vec<_>>();
		let transparent_materials = self
			.material_evaluation_materials
			.read()
			.values()
			.filter_map(|v| v.get())
			.filter(|v| v.alpha == true)
			.map(|v| (v.name.clone(), v.index, v.pipeline))
			.collect::<Vec<_>>();

		let viewport_x_rp = viewports.iter().map(|v| (v, &self.views[v.index()].1));

		let commands: Vec<Box<dyn RenderPassFunction>> = viewport_x_rp
			.map(|(v, r)| Box::new(r.prepare(frame, v, &[])) as Box<dyn RenderPassFunction>)
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

		let render_pass = VisibilityPipelineRenderPass::new(
			render_pass_builder.device(),
			self.base_pipeline_layout,
			self.visibility_pass_pipeline_layout,
			self.material_evaluation_pipeline_layout,
			self.descriptor_set,
			self.visibility_passes_descriptor_set,
			self.material_evaluation_descriptor_set,
			diffuse_target.into(),
			specular_target.into(),
			depth_target.into(),
			primitive_index.into(),
			instance_id.into(),
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
