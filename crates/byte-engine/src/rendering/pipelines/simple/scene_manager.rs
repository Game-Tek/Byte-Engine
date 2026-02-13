//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

use std::{collections::{hash_map::Entry, VecDeque}, sync::Arc};

use besl::ParserNode;
use ghi::{command_buffer::{BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecordable as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _}, device::Device as _, frame::Frame, Device};
use math::Matrix4;
use resource_management::{asset::material_asset_handler::ProgramGenerator, shader_generator::ShaderGenerationSettings, spirv_shader_generator::SPIRVShaderGenerator};
use utils::{hash::{HashMap, HashMapExt}, json::{self, JsonContainerTrait as _, JsonValueTrait as _}, sync::RwLock, Box, Extent};

use crate::{camera::Camera, core::{Entity, EntityHandle, channel::DefaultChannel, entity::{self}, factory::CreateMessage, listener::{DefaultListener, Listener}}, gameplay::Transformable, rendering::{RenderableMesh, Viewport, common_shader_generator::CommonShaderScope, lights::{Light, Lights}, make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor, pipelines::simple::{CameraShaderData, RenderPass, render_pass}, render_pass::{FramePrepare, RenderPassBuilder, RenderPassFunction, RenderPassReturn}, renderable::mesh::MeshSource, utils::{InstanceBatch, MeshBuffersStats, MeshStats}, view::View}};

pub struct SceneManager {
	/// Buffer containing all vertex positions for meshes.
	pub(super) vertex_positions_buffer: ghi::BufferHandle<[(f32, f32, f32); 1024 * 1024]>,
	pub(super) indeces_buffer: ghi::BufferHandle<[u16; 1024 * 1024]>,
	pub(super) instance_data_buffer: ghi::DynamicBufferHandle<[InstanceShaderData; 1024]>,
	pub(super) camera_data_buffer: ghi::DynamicBufferHandle<[CameraShaderData; 8]>,
	pub(super) mesh_buffers_stats: MeshBuffersStats<EntityHandle<dyn Transformable>>,
	pub(super) descriptor_set_template: ghi::DescriptorSetTemplateHandle,
	pub(super) pipeline_layout: ghi::PipelineLayoutHandle,
	pub(super) pipeline: ghi::PipelineHandle,
	views: Vec<RenderPass>,

	renderable_meshes_channel: DefaultListener<CreateMessage<EntityHandle<dyn RenderableMesh>>>,
}

const VERTEX_LAYOUT: [ghi::VertexElement; 1] = [
	ghi::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0),
];

impl SceneManager {
	pub fn new(device: &mut ghi::Device, renderable_meshes_channel: DefaultListener<CreateMessage<EntityHandle<dyn RenderableMesh>>>) -> Self {
		let vertex_positions_buffer = device.create_buffer(Some("Vertex Positions"), ghi::Uses::Vertex, ghi::DeviceAccesses::HostToDevice);
		let indeces_buffer = device.create_buffer(Some("Indeces"), ghi::Uses::Index, ghi::DeviceAccesses::HostToDevice);

		let camera_data_buffer = device.create_dynamic_buffer(Some("Camera Data Buffer"), ghi::Uses::Storage, ghi::DeviceAccesses::HostToDevice);
		let instance_data_buffer = device.create_dynamic_buffer(Some("Instance Data Buffer"), ghi::Uses::Storage, ghi::DeviceAccesses::HostToDevice);

		let camera_data_binding_template = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::VERTEX);
		let instance_data_binding_template = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::VERTEX);

		let descriptor_set_template = device.create_descriptor_set_template(None, &[
			camera_data_binding_template.clone(),
			instance_data_binding_template.clone(),
		]);

		let pipeline_layout = device.create_pipeline_layout(&[descriptor_set_template], &[ghi::PushConstantRange::new(0, 4)]);

		let mut shader_generator = SPIRVShaderGenerator::new();

		let generated_vertex_shader = {
			let main_code = r#"
			Camera camera = cameras.cameras[0];
			uint instance_index = gl_InstanceIndex;
			Instance instance = instances.instances[instance_index];

			gl_Position = camera.view_projection * instance.transform * vec4(in_position, 1.0);
			out_instance_index = instance_index;
			"#.trim();

			let main = besl::ParserNode::main_function(vec![besl::ParserNode::glsl(main_code, &["cameras", "instances", "push_constant", "in_position", "out_instance_index"], &[])]);

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

			let main = besl::ParserNode::main_function(vec![besl::ParserNode::glsl(main_code, &["in_instance_index", "out_albedo", "get_debug_color"], &[])]);

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

		let pipeline = device.create_raster_pipeline(
			ghi::raster_pipeline::Builder::new(
				pipeline_layout,
				&VERTEX_LAYOUT,
				&[ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex), ghi::ShaderParameter::new(&fragment_shader, ghi::ShaderTypes::Fragment)],
				&[
					ghi::PipelineAttachmentInformation::new(ghi::Formats::RGBA16F),
					ghi::PipelineAttachmentInformation::new(ghi::Formats::Depth32),
				],
			)
		);

		Self {
			vertex_positions_buffer,
			indeces_buffer,

			mesh_buffers_stats: MeshBuffersStats::default(),

			instance_data_buffer,
			camera_data_buffer,

			descriptor_set_template,
			pipeline_layout,
			pipeline,

			views: Vec::with_capacity(4),

			renderable_meshes_channel,
		}
	}
}

impl crate::rendering::scene_manager::SceneManager for SceneManager {
	fn prepare(&mut self, frame: &mut ghi::Frame, viewports: &[Viewport]) -> Option<Vec<Box<dyn RenderPassFunction>>> {
		for message in self.renderable_meshes_channel.iter() {
			let handle = message.into_data();
			let entity = handle.read();

			let mesh = entity.get_mesh();

			let mesh_id = match mesh {
				MeshSource::Generated(generator) => 'a: {
					let mesh_hash = generator.hash();

					if let Some(mesh_id) = self.mesh_buffers_stats.does_mesh_exist(mesh_hash) {
						break 'a mesh_id;
					}

					let positions = generator.positions();
					let indices = generator.indices();
					let indices = indices.iter().map(|&index| index as u16);

					let vertex_count = positions.len();
					let index_count = indices.len();

					let vertex_buffer = frame.device().get_mut_buffer_slice(self.vertex_positions_buffer);

					let mesh_ref = self.mesh_buffers_stats.add_mesh(MeshStats::new(vertex_count, index_count), mesh_hash);

					let vertex_buffer_offset = mesh_ref.vertex_offset();
					let index_buffer_offset = mesh_ref.index_offset();

					vertex_buffer[vertex_buffer_offset..][..vertex_count].copy_from_slice(&positions);

					let index_buffer = frame.device().get_mut_buffer_slice(self.indeces_buffer);

					index_buffer[index_buffer_offset..][..index_count].iter_mut().zip(indices).for_each(|(dst, src)| {
						*dst = src;
					});

					mesh_ref.id()
				}
				_ => {
					log::warn!("SimpleRenderModel does not support non-generated meshes");
					continue;
				}
			};

			drop(entity);

			self.mesh_buffers_stats.add_instance(mesh_id, handle);
		}

		let instance_data_buffer = frame.get_mut_dynamic_buffer_slice(self.instance_data_buffer);

		let instance_batches = self.mesh_buffers_stats.get_instance_batches();

		for batch in instance_batches.iter() {
			for (index, instance_data) in batch {
				instance_data_buffer[index] = InstanceShaderData { instance_transform: instance_data.read().transform().get_matrix() };
			}
		}

		let instance_batches = instance_batches.iter().into_vec();

		let commands = viewports.iter().filter_map(|viewport| {
			self.views.iter().find(|v| v.index == viewport.index()).map(|v| (viewport, v))
		}).map(|(viewport, v)| {
			Box::new(v.prepare(frame, viewport, &self, &instance_batches)) as Box<dyn RenderPassFunction>
		}).collect::<Vec<_>>();

		Some(commands)
	}

	fn create_view(&mut self, id: usize, render_pass_builder: &mut RenderPassBuilder) {
		let main = render_pass_builder.render_to("main");
		let depth = render_pass_builder.render_to("depth");
		self.views.push(RenderPass::new(render_pass_builder.device(), &self.descriptor_set_template, self.camera_data_buffer.into(), self.instance_data_buffer.into(), id))
	}
}

#[derive(Debug, Clone, Copy)]
pub(super) struct InstanceShaderData {
	instance_transform: Matrix4,
}
