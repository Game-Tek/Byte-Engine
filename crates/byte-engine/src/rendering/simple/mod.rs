//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

use core::slice::SlicePattern;
use std::{collections::{hash_map::Entry, VecDeque}, sync::Arc};

use besl::ParserNode;
use ghi::{command_buffer::{BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecordable as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _}, device::Device as _, frame::Frame, Device};
use math::Matrix4;
use resource_management::{asset::material_asset_handler::ProgramGenerator, shader_generator::ShaderGenerationSettings, spirv_shader_generator::SPIRVShaderGenerator};
use utils::{hash::{HashMap, HashMapExt}, json::{self, JsonContainerTrait as _, JsonValueTrait as _}, sync::RwLock, Box, Extent};

use crate::{camera::Camera, core::{entity::{self, EntityBuilder}, listener::{CreateEvent, Listener}, Entity, EntityHandle}, rendering::{common_shader_generator::CommonShaderScope, make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor, mesh::{MeshSource, RenderEntity}, render_pass::{RenderPass, RenderPassBuilder, RenderPassCommand}}};

pub struct SimpleRenderModel {
	meshes: Vec<u32>,
	camera: Option<EntityHandle<Camera>>,

	vertex_positions_buffer: ghi::BufferHandle<[(f32, f32, f32); 1024 * 1024]>,
	indeces_buffer: ghi::BufferHandle<[u16; 1024 * 1024]>,

	instance_data_buffer: ghi::DynamicBufferHandle<[InstanceShaderData; 1024]>,
	camera_data_buffer: ghi::DynamicBufferHandle<[CameraShaderData; 8]>,

	mesh_buffers_stats: MeshBuffersStats,

	descriptor_set: ghi::DescriptorSetHandle,

	pipeline_layout: ghi::PipelineLayoutHandle,
	pipeline: ghi::PipelineHandle,

	pending_entities: VecDeque<EntityHandle<dyn RenderEntity>>,
}

const VERTEX_LAYOUT: [ghi::VertexElement; 1] = [
	ghi::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0),
];

impl SimpleRenderModel {
	pub fn new<'a>(render_pass_builder: &mut RenderPassBuilder<'a>) -> Self {
		let render_to = render_pass_builder.render_to("main");

		let device = render_pass_builder.device();

		let vertex_positions_buffer = device.create_buffer(Some("Vertex Positions"), ghi::Uses::Vertex, ghi::DeviceAccesses::HostToDevice);
		let indeces_buffer = device.create_buffer(Some("Indeces"), ghi::Uses::Index, ghi::DeviceAccesses::HostToDevice);

		let instance_data_buffer = device.create_dynamic_buffer(Some("Instance Data Buffer"), ghi::Uses::Storage, ghi::DeviceAccesses::HostToDevice);
		let camera_data_buffer = device.create_dynamic_buffer(Some("Camera Data Buffer"), ghi::Uses::Storage, ghi::DeviceAccesses::HostToDevice);

		let instance_data_binding_template = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::VERTEX);
		let camera_data_binding_template = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::VERTEX);

		let descriptor_set_layout = device.create_descriptor_set_template(None, &[
			instance_data_binding_template.clone(),
			camera_data_binding_template.clone(),
		]);

		let descriptor_set = device.create_descriptor_set(None, &descriptor_set_layout);

		device.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&instance_data_binding_template, instance_data_buffer.into()));
		device.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&camera_data_binding_template, camera_data_buffer.into()));

		let pipeline_layout = device.create_pipeline_layout(&[descriptor_set_layout], &[ghi::PushConstantRange::new(0, 4)]);

		let mut shader_generator = SPIRVShaderGenerator::new();

		let generated_vertex_shader = {
			let main_code = r#"
			Camera camera = cameras.cameras[0];
			uint instance_index = gl_InstanceIndex;
			Instance instance = instances.instances[instance_index];

			gl_Position = camera.view_projection * instance.transform * vec4(in_position, 1.0);
			out_instance_index = instance_index;
			"#.trim();

			let main = besl::ParserNode::main_function(vec![besl::ParserNode::glsl(main_code, &["cameras", "instances", "push_constant", "in_position", "out_instance_index"], Vec::new())]);

			let mut root = besl::ParserNode::root();

			let push_constant = ParserNode::push_constant(vec![ParserNode::member("instance_index", "u32")]);

			let camera = ParserNode::r#struct("Camera", vec![ParserNode::member("view_projection", "mat4f")]);
			let instance = ParserNode::r#struct("Instance", vec![ParserNode::member("transform", "mat4f")]);

			let cameras_binding = ParserNode::binding("cameras", ParserNode::buffer("CamerasBuffer", vec![ParserNode::member("cameras", "Camera[8]")]), 0, 0, true, false);
			let instances_binding = ParserNode::binding("instances", ParserNode::buffer("InstancesBuffer", vec![ParserNode::member("instances", "Instance[8]")]), 0, 1, true, false);

			let position_input = ParserNode::input("in_position", "vec3f", 0);
			let instance_index_output = ParserNode::output("out_instance_index", "u32", 0);

			let shader = besl::ParserNode::scope("Shader", vec![camera, instance, cameras_binding, instances_binding, position_input, instance_index_output, push_constant, main]);

			root.add(vec![CommonShaderScope::new(), shader]);

			let root_node = besl::lex(root).unwrap();

			let main_node = root_node.get_main().unwrap();

			let generated = shader_generator.generate(&ShaderGenerationSettings::vertex(), &main_node).unwrap();

			generated
		};

		let generated_fragment_shader = {
			let main_code = r#"
			uint instance_index = in_instance_index;
			out_albedo = get_debug_color(instance_index);
			"#.trim();

			let main = besl::ParserNode::main_function(vec![besl::ParserNode::glsl(main_code, &["in_instance_index", "out_albedo", "get_debug_color"], Vec::new())]);

			let mut root = besl::ParserNode::root();

			let instance_index_input = ParserNode::input("in_instance_index", "u32", 0);
			let albedo_output = ParserNode::output("out_albedo", "vec4f", 0);

			let shader = besl::ParserNode::scope("Shader", vec![instance_index_input, albedo_output, main]);

			root.add(vec![CommonShaderScope::new(), shader]);

			let root_node = besl::lex(root).unwrap();

			let main_node = root_node.get_main().unwrap();

			let generated = shader_generator.generate(&ShaderGenerationSettings::fragment(), &main_node).unwrap();

			generated
		};

		let vertex_shader = device.create_shader(Some("Vertex Shader"), ghi::ShaderSource::SPIRV(generated_vertex_shader.binary()), ghi::ShaderTypes::Vertex, generated_vertex_shader.bindings().iter().map(map_shader_binding_to_shader_binding_descriptor)).unwrap();
		let fragment_shader = device.create_shader(Some("Fragment Shader"), ghi::ShaderSource::SPIRV(generated_fragment_shader.binary()), ghi::ShaderTypes::Fragment, generated_fragment_shader.bindings().iter().map(map_shader_binding_to_shader_binding_descriptor)).unwrap();

		let pipeline = device.create_raster_pipeline(ghi::raster_pipeline::Builder::new(pipeline_layout, &VERTEX_LAYOUT, &[ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex), ghi::ShaderParameter::new(&fragment_shader, ghi::ShaderTypes::Fragment)], &[render_to.into()]));

		Self {
			meshes: Vec::new(),
			camera: None,

			vertex_positions_buffer,
			indeces_buffer,

			descriptor_set,

			mesh_buffers_stats: MeshBuffersStats::default(),

			instance_data_buffer,
			camera_data_buffer,

			pipeline_layout,
			pipeline,

			pending_entities: VecDeque::with_capacity(64),
		}
	}
}

impl Entity for SimpleRenderModel {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self).listen_to::<CreateEvent<dyn RenderEntity>>().listen_to::<CreateEvent<Camera>>()
	}
}

impl Listener<CreateEvent<Camera>> for SimpleRenderModel {
	fn handle(&mut self, event: &CreateEvent<Camera>) {
    	self.camera = Some(event.handle().clone());
	}
}

impl Listener<CreateEvent<dyn RenderEntity>> for SimpleRenderModel {
	fn handle(&mut self, event: &CreateEvent<dyn RenderEntity>) {
		let entity = event.handle();

		self.pending_entities.push_back(entity.clone());
	}
}

impl RenderPass for SimpleRenderModel {
	fn get_read_attachments() -> Vec<&'static str> where Self: Sized {
		Vec::new()
	}

	fn get_write_attachments() -> Vec<&'static str> where Self: Sized {
		Vec::new()
	}

	fn prepare(&mut self, frame: &mut ghi::Frame, extent: utils::Extent) -> Option<RenderPassCommand> {
		{
			let pending_entities = self.pending_entities.drain(..);

			for entity in pending_entities {
				let entity = entity.read();

				let mesh = entity.get_mesh();

				let mesh = match mesh {
					MeshSource::Generated(generator) => {
						let vertices = generator.vertices();
						let indices = generator.indices();
						let indices = indices.iter().map(|&index| index as u16);

						let vertex_count = vertices.len();
						let index_count = indices.len();

						let vertex_buffer = frame.device().get_mut_buffer_slice(self.vertex_positions_buffer);

						let vertex_buffer_offset = self.mesh_buffers_stats.vertex_count;
						let index_buffer_offset = self.mesh_buffers_stats.index_count;

						vertex_buffer[vertex_buffer_offset..][..vertex_count].copy_from_slice(vertices.as_slice());

						let index_buffer = frame.device().get_mut_buffer_slice(self.indeces_buffer);

						index_buffer[index_buffer_offset..][..index_count].iter_mut().zip(indices).for_each(|(dst, src)| {
							*dst = src;
						});

						MeshStats::new(vertex_count, index_count, vertex_buffer_offset, index_buffer_offset)
					}
					_ => {
						log::warn!("SimpleRenderModel does not support non-generated meshes");
						continue;
					}
				};

				let mesh_id = self.mesh_buffers_stats.add(mesh);
				self.mesh_buffers_stats.add_instance(mesh_id);
			}
		}

		let Some(camera) = &self.camera else {
			log::warn!("SimpleRenderModel requires a camera to be set");
			return None;
		};

		let camera_data_buffer = frame.get_mut_dynamic_buffer_slice(self.camera_data_buffer);

		let view = make_perspective_view_from_camera(&camera.read(), extent);

		camera_data_buffer[0] = CameraShaderData { vp: view.view_projection() };

		let instance_data_buffer = frame.get_mut_dynamic_buffer_slice(self.instance_data_buffer);

		for (index, mesh) in self.meshes.iter().enumerate() {
			instance_data_buffer[index] = InstanceShaderData { instance_transform: Matrix4::identity() };
		}

		let instance_batches = self.mesh_buffers_stats.get_instance_batches();
		let pipeline_layout = self.pipeline_layout;
		let pipeline = self.pipeline.clone();
		let descriptor_set = self.descriptor_set;
		let vertex_buffer = self.vertex_positions_buffer;
		let index_buffer = self.indeces_buffer;

		Some(Box::new(move |c, t| {
			c.bind_vertex_buffers(&[vertex_buffer.into()]);
			c.bind_index_buffer(&index_buffer.into());
			let c = c.start_render_pass(extent, t);
			let c = c.bind_pipeline_layout(pipeline_layout);
			c.bind_descriptor_sets(&[descriptor_set]);
			let c = c.bind_raster_pipeline(pipeline);
			for batch in &instance_batches {
				c.write_push_constant(0, batch.base_instance as u32);
				c.draw_indexed(batch.index_count as u32, batch.instance_count as u32, batch.base_index as _, batch.base_vertex as _, batch.base_instance as _);
			}
			c.end_render_pass();
		}))
	}
}

#[derive(Debug, Clone, Copy)]
struct InstanceShaderData {
	instance_transform: Matrix4,
}

#[derive(Debug, Clone, Copy)]
struct CameraShaderData {
	vp: Matrix4,
}

struct MeshStats {
	vertex_count: usize,
	index_count: usize,
	base_index: usize,
	base_vertex: usize,
}

impl MeshStats {
	pub fn new(vertex_count: usize, index_count: usize, base_index: usize, base_vertex: usize) -> Self {
		Self {
			vertex_count,
			index_count,
			base_index,
			base_vertex,
		}
	}
}

struct MeshBuffersStats {
	vertex_count: usize,
	index_count: usize,

	meshes: Vec<MeshStats>,

	instances: Vec<usize>,
}

struct InstanceBatch {
	base_index: usize,
	base_vertex: usize,
	instance_count: usize,
	index_count: usize,
	base_instance: usize,
}

impl MeshBuffersStats {
	pub fn add(&mut self, other: MeshStats) -> usize {
		self.vertex_count += other.vertex_count;
		self.index_count += other.index_count;

		let mesh_id = self.meshes.len();
		self.meshes.push(other);

		mesh_id
	}

	pub fn add_instance(&mut self, mesh_id: usize) {
		self.instances.push(mesh_id);
	}

	pub fn get_instance_batches(&self) -> Vec<InstanceBatch> {
		let mut batches = HashMap::with_capacity(self.meshes.len());

		for (instance_id, &mesh_id) in self.instances.iter().enumerate() {
			let mesh = &self.meshes[mesh_id];

			match batches.entry(mesh_id) {
				Entry::Vacant(e) => {
					e.insert(InstanceBatch {
						index_count: mesh.index_count,
						instance_count: 1,
						base_vertex: mesh.base_vertex,
						base_index: mesh.base_index,
						base_instance: instance_id,
					});
				}
				Entry::Occupied(mut e) => {
					e.get_mut().instance_count += 1;
				}
			}
		}

		batches.into_values().collect::<Vec<_>>()
	}
}

impl Default for MeshBuffersStats {
	fn default() -> Self {
		Self {
			vertex_count: 0,
			index_count: 0,
			meshes: Vec::new(),
			instances: Vec::new(),
		}
	}
}
