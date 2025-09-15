//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

use core::slice::SlicePattern;
use std::{collections::VecDeque, sync::Arc};

use besl::ParserNode;
use ghi::{frame::Frame, graphics_hardware_interface::Device, BoundRasterizationPipelineMode, CommandBufferRecordable, RasterizationRenderPassMode};
use math::Matrix4;
use resource_management::{asset::material_asset_handler::ProgramGenerator, shader_generator::ShaderGenerationSettings, spirv_shader_generator::SPIRVShaderGenerator};
use utils::{json::{self, JsonContainerTrait as _, JsonValueTrait as _}, sync::RwLock, Box, Extent};

use crate::{camera::Camera, core::{entity::{self, EntityBuilder}, listener::{CreateEvent, Listener}, Entity, EntityHandle}, rendering::{common_shader_generator::CommonShaderScope, make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor, mesh::{MeshSource, RenderEntity}, render_pass::{RenderPass, RenderPassBuilder, RenderPassCommand}}};

pub struct SimpleRenderModel {
	meshes: Vec<ghi::MeshHandle>,
	camera: Option<EntityHandle<Camera>>,

	instance_data_buffer: ghi::DynamicBufferHandle<[InstanceShaderData; 1024]>,
	camera_data_buffer: ghi::DynamicBufferHandle<[CameraShaderData; 8]>,

	pipeline: ghi::PipelineHandle,

	pending_entities: VecDeque<EntityHandle<dyn RenderEntity>>,
}

const VERTEX_LAYOUT: [ghi::VertexElement; 1] = [
	ghi::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0),
];

impl SimpleRenderModel {
	pub fn new<'a>(render_pass_builder: &mut RenderPassBuilder<'a>) -> Self {
		let device = render_pass_builder.device();

		let instance_data_buffer = device.create_dynamic_buffer(Some("Instance Data Buffer"), ghi::Uses::Storage, ghi::DeviceAccesses::HostToDevice);
		let camera_data_buffer = device.create_dynamic_buffer(Some("Camera Data Buffer"), ghi::Uses::Storage, ghi::DeviceAccesses::HostToDevice);

		let pipeline_layout = device.create_pipeline_layout(&[], &[]);

		let mut shader_generator = SPIRVShaderGenerator::new();

		let generated_vertex_shader = {
			let main_code = r#"
			Camera camera = cameras.cameras[0];
			Instance instance = instances.instances[instance_index];

			gl_Position = camera.view_projection * instance.transform * vec4(position, 1.0);
			out_instance_index = instance_index;
			"#.trim();

			let main = besl::ParserNode::main_function(vec![besl::ParserNode::glsl(main_code, &["cameras", "instances"], Vec::new())]);

			let mut root = besl::ParserNode::root();

			let camera = ParserNode::r#struct("Camera", vec![ParserNode::member("view_projection", "mat4f")]);
			let instance = ParserNode::r#struct("Instance", vec![ParserNode::member("transform", "mat4f")]);

			let cameras_binding = ParserNode::binding("cameras", ParserNode::buffer("CamerasBuffer", vec![ParserNode::member("cameras", "Camera[8]")]), 0, 0, true, false);
			let instances_binding = ParserNode::binding("instances", ParserNode::buffer("InstancesBuffer", vec![ParserNode::member("instances", "Instance[8]")]), 1, 0, true, false);

			let instance_index_output = ParserNode::output("out_instance_index", "u32", 0);

			let shader = besl::ParserNode::scope("Shader", vec![camera, instance, cameras_binding, instances_binding, instance_index_output, main]);

			root.add(vec![CommonShaderScope::new(), shader]);

			let root_node = besl::lex(root).unwrap();

			let main_node = root_node.get_main().unwrap();

			let generated = shader_generator.generate(&ShaderGenerationSettings::vertex(), &main_node).unwrap();

			generated
		};

		let generated_fragment_shader = {
			let main_code = r#"
			uint instance_index = in_instance_index;
			output = get_debug_color(instance_index);
			"#.trim();

			let main = besl::ParserNode::main_function(vec![besl::ParserNode::glsl(main_code, &["get_debug_color"], Vec::new())]);

			let mut root = besl::ParserNode::root();

			let instance_index_input = ParserNode::input("in_instance_index", "u32", 0);

			let shader = besl::ParserNode::scope("Shader", vec![instance_index_input, main]);

			root.add(vec![CommonShaderScope::new(), shader]);

			let root_node = besl::lex(root).unwrap();

			let main_node = root_node.get_main().unwrap();

			let generated = shader_generator.generate(&ShaderGenerationSettings::fragment(), &main_node).unwrap();

			generated
		};

		let vertex_shader = device.create_shader(Some("Vertex Shader"), ghi::ShaderSource::SPIRV(generated_vertex_shader.binary()), ghi::ShaderTypes::Vertex, generated_vertex_shader.bindings().iter().map(map_shader_binding_to_shader_binding_descriptor)).unwrap();
		let fragment_shader = device.create_shader(Some("Fragment Shader"), ghi::ShaderSource::SPIRV(generated_fragment_shader.binary()), ghi::ShaderTypes::Fragment, generated_fragment_shader.bindings().iter().map(map_shader_binding_to_shader_binding_descriptor)).unwrap();

		let pipeline = device.create_raster_pipeline(ghi::raster_pipeline::Builder::new(pipeline_layout, &VERTEX_LAYOUT, &[ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex), ghi::ShaderParameter::new(&fragment_shader, ghi::ShaderTypes::Fragment)], &[ghi::PipelineAttachmentInformation::new(ghi::Formats::RGBu11u11u10)]));

		Self {
			meshes: Vec::new(),
			camera: None,

			instance_data_buffer,
			camera_data_buffer,

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

						let vertex_count = vertices.len();
						let index_count = indices.len();

						let mesh = frame.device().add_mesh_from_vertices_and_indices(vertex_count as u32, index_count as u32, unsafe { std::mem::transmute(vertices.as_slice()) }, unsafe { std::mem::transmute(indices.as_slice()) }, &VERTEX_LAYOUT);

						mesh
					}
					_ => {
						log::warn!("SimpleRenderModel does not support non-generated meshes");
						continue;
					}
				};

				self.meshes.push(mesh);
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

		let meshes = self.meshes.clone();
		let pipeline = self.pipeline.clone();

		Some(Box::new(move |c, t| {
			let render_pass = c.start_render_pass(extent, t);
			let pipeline = render_pass.bind_raster_pipeline(&pipeline);
			for mesh in &meshes {
				pipeline.draw_mesh(&mesh);
			}
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
