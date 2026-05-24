//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

use std::{
	collections::{hash_map::Entry, VecDeque},
	sync::Arc,
};

use besl::ParserNode;
use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
		CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	context::{Context as _, ContextCreate as _},
	device::Device as _,
	frame::Frame,
};
use math::{Matrix4, ShaderMatrix4};
use resource_management::{
	asset::bema_asset_handler::ProgramGenerator, msl_shader_generator::MSLShaderGenerator,
	shader_generator::ShaderGenerationSettings, spirv_shader_generator::SPIRVShaderGenerator,
};
use utils::{
	hash::{HashMap, HashMapExt},
	json::{self, JsonContainerTrait as _, JsonValueTrait as _},
	sync::RwLock,
	Box, Extent,
};

use crate::{
	core::{
		channel::DefaultChannel,
		entity::{self},
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
		Entity, EntityHandle,
	},
	gameplay::transform::TransformationUpdate,
	rendering::Camera,
	rendering::{
		lights::{Light, Lights},
		make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor,
		pipelines::simple::{render_pass, CameraShaderData, RenderPass},
		render_pass::{FramePrepare, RenderPassBuilder, RenderPassFunction, RenderPassReturn},
		renderable::mesh::MeshSource,
		utils::{InstanceBatch, MeshBuffersStats, MeshStats},
		view::View,
		RenderableMesh, Sink,
	},
};

pub struct PipelineManager {
	/// Buffer containing all vertex positions for meshes.
	pub(super) vertex_positions_buffer: ghi::BufferHandle<[(f32, f32, f32); 1024 * 1024]>,
	pub(super) indeces_buffer: ghi::BufferHandle<[u16; 1024 * 1024]>,
	pub(super) instance_data_buffer: ghi::DynamicBufferHandle<[InstanceShaderData; 1024]>,
	pub(super) camera_data_buffer: ghi::DynamicBufferHandle<[CameraShaderData; 8]>,
	pub(super) mesh_buffers_stats: MeshBuffersStats<Handle>,
	pub(super) descriptor_set_template: ghi::DescriptorSetTemplateHandle,
	pub(super) pipeline: ghi::PipelineHandle,
	sinks: Vec<RenderPass>,
}

const VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 1] =
	[ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0)];

const SIMPLE_FRAGMENT_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct VertexOutput {
	float4 position [[position]];
	uint out_instance_index [[flat]] [[user(locn0)]];
};

static float4 debug_color(uint index) {
	const float3 palette[8] = {
		float3(0.90, 0.20, 0.20),
		float3(0.20, 0.70, 0.95),
		float3(0.35, 0.85, 0.35),
		float3(0.95, 0.75, 0.20),
		float3(0.75, 0.35, 0.95),
		float3(0.95, 0.45, 0.20),
		float3(0.25, 0.90, 0.75),
		float3(0.85, 0.85, 0.90),
	};
	return float4(palette[index % 8], 1.0);
}

fragment float4 besl_main(VertexOutput in [[stage_in]]) {
	return debug_color(in.out_instance_index);
}
"#;

impl PipelineManager {
	pub fn new(context: &mut ghi::implementation::Context) -> Self {
		let vertex_positions_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Vertex)
				.name("Vertex Positions")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let indeces_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Index)
				.name("Indeces")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let camera_data_buffer = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Camera Data Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let instance_data_buffer = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Instance Data Buffer")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);

		let camera_data_binding_template =
			ghi::DescriptorSetBindingTemplate::new(0, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::VERTEX);
		let instance_data_binding_template =
			ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::VERTEX);

		let descriptor_set_template = context.create_descriptor_set_template(
			None,
			&[camera_data_binding_template.clone(), instance_data_binding_template.clone()],
		);

		let mut spirv_shader_generator = SPIRVShaderGenerator::new();
		let mut msl_shader_generator = MSLShaderGenerator::new();

		let (generated_vertex_shader, generated_vertex_msl) = {
			let main_code = r#"
			Camera camera = cameras.cameras[0];
			uint instance_index = gl_InstanceIndex;
			Instance instance = instances.instances[instance_index];

			gl_Position = camera.view_projection * instance.transform * vec4(in_position, 1.0);
			out_instance_index = instance_index;
			"#
			.trim();
			let main_msl = r#"
			VertexOutput out;
			Camera camera = set0.cameras->cameras[0];
			Instance instance = set0.instances->instances[instance_index];

			out.position = camera.view_projection * instance.transform * float4(in.position, 1.0);
			out.out_instance_index = instance_index;
			return out;
			"#
			.trim();

			let main = besl::ParserNode::main_function(vec![besl::ParserNode::raw_code(
				Some(main_code.into()),
				None,
				Some(main_msl.into()),
				&["cameras", "instances", "push_constant", "in_position", "out_instance_index"],
				&[],
			)]);

			let mut root = besl::ParserNode::root();

			let push_constant = ParserNode::push_constant(vec![ParserNode::member("instance_index", "u32")]);

			let camera = ParserNode::r#struct("Camera", vec![ParserNode::member("view_projection", "mat4f")]);
			let instance = ParserNode::r#struct("Instance", vec![ParserNode::member("transform", "mat4f")]);

			let cameras_binding = ParserNode::binding(
				"cameras",
				ParserNode::buffer("CamerasBuffer", vec![ParserNode::member("cameras", "Camera[8]")]),
				0,
				0,
				true,
				false,
			);
			let instances_binding = ParserNode::binding(
				"instances",
				ParserNode::buffer("InstancesBuffer", vec![ParserNode::member("instances", "Instance[8]")]),
				0,
				1,
				true,
				false,
			);

			let position_input = ParserNode::input("in_position", "vec3f", 0);
			let instance_index_output = ParserNode::output("out_instance_index", "u32", 0);

			let shader = besl::ParserNode::scope(
				"Shader",
				vec![
					camera,
					instance,
					cameras_binding,
					instances_binding,
					position_input,
					instance_index_output,
					push_constant,
					main,
				],
			);

			root.add(vec![shader]);

			let root_node = besl::lex(root).unwrap();

			let main_node = root_node.get_main().unwrap();

			let generated_spirv = spirv_shader_generator
				.generate(&ShaderGenerationSettings::vertex(), &main_node)
				.unwrap();

			let generated_msl = msl_shader_generator
				.generate(&ShaderGenerationSettings::vertex(), &main_node)
				.unwrap();

			(generated_spirv, generated_msl)
		};

		let (generated_fragment_shader, generated_fragment_msl) = {
			let main_code = r#"
			uint instance_index = in_instance_index;
			vec3 palette[8] = vec3[](
				vec3(0.90, 0.20, 0.20),
				vec3(0.20, 0.70, 0.95),
				vec3(0.35, 0.85, 0.35),
				vec3(0.95, 0.75, 0.20),
				vec3(0.75, 0.35, 0.95),
				vec3(0.95, 0.45, 0.20),
				vec3(0.25, 0.90, 0.75),
				vec3(0.85, 0.85, 0.90)
			);
			out_albedo = vec4(palette[instance_index % 8], 1.0);
			"#
			.trim();

			let main = besl::ParserNode::main_function(vec![besl::ParserNode::raw_code(
				Some(main_code.into()),
				None,
				Some(format!("// besl-full-source\n{SIMPLE_FRAGMENT_MSL}").into()),
				&["in_instance_index", "out_albedo"],
				&[],
			)]);

			let mut root = besl::ParserNode::root();

			let instance_index_input = ParserNode::input("in_instance_index", "u32", 0);
			let albedo_output = ParserNode::output("out_albedo", "vec4f", 0);

			let shader = besl::ParserNode::scope("Shader", vec![instance_index_input, albedo_output, main]);

			root.add(vec![shader]);

			let root_node = besl::lex(root).unwrap();

			let main_node = root_node.get_main().unwrap();

			let generated_spirv = spirv_shader_generator
				.generate(&ShaderGenerationSettings::fragment(), &main_node)
				.unwrap();

			let generated_msl = msl_shader_generator
				.generate(&ShaderGenerationSettings::fragment(), &main_node)
				.unwrap();

			(generated_spirv, generated_msl)
		};

		#[cfg(target_vendor = "apple")]
		let vertex_shader_source = ghi::shader::Sources::MTL {
			source: &generated_vertex_msl,
			entry_point: "besl_main",
		};
		#[cfg(not(target_vendor = "apple"))]
		let vertex_shader_source = ghi::shader::Sources::SPIRV(generated_vertex_shader.binary());

		let vertex_shader = context
			.create_shader(
				Some("Vertex Shader"),
				vertex_shader_source,
				ghi::ShaderTypes::Vertex,
				generated_vertex_shader
					.bindings()
					.iter()
					.map(map_shader_binding_to_shader_binding_descriptor),
			)
			.unwrap();
		#[cfg(target_vendor = "apple")]
		let fragment_shader_source = ghi::shader::Sources::MTL {
			source: &generated_fragment_msl,
			entry_point: "besl_main",
		};
		#[cfg(not(target_vendor = "apple"))]
		let fragment_shader_source = ghi::shader::Sources::SPIRV(generated_fragment_shader.binary());

		let fragment_shader = context
			.create_shader(
				Some("Fragment Shader"),
				fragment_shader_source,
				ghi::ShaderTypes::Fragment,
				generated_fragment_shader
					.bindings()
					.iter()
					.map(map_shader_binding_to_shader_binding_descriptor),
			)
			.unwrap();

		let pipeline = context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
			&[descriptor_set_template],
			&[ghi::pipelines::PushConstantRange::new(0, 4)],
			&VERTEX_LAYOUT,
			&[
				ghi::ShaderParameter::new(&vertex_shader, ghi::ShaderTypes::Vertex),
				ghi::ShaderParameter::new(&fragment_shader, ghi::ShaderTypes::Fragment),
			],
			&[
				ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::RGBA16UNORM),
				ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::Depth32),
			],
		));

		Self {
			vertex_positions_buffer,
			indeces_buffer,

			mesh_buffers_stats: MeshBuffersStats::default(),

			instance_data_buffer,
			camera_data_buffer,

			descriptor_set_template,
			pipeline,

			sinks: Vec::with_capacity(4),
		}
	}

	pub fn create_mesh(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		handle: Handle,
		entity: EntityHandle<dyn RenderableMesh>,
	) {
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

				let vertex_buffer = frame.get_mut_buffer_slice(self.vertex_positions_buffer);

				let mesh_ref = self
					.mesh_buffers_stats
					.add_mesh(MeshStats::new(vertex_count, index_count), mesh_hash);

				let vertex_buffer_offset = mesh_ref.vertex_offset();
				let index_buffer_offset = mesh_ref.index_offset();

				vertex_buffer[vertex_buffer_offset..][..vertex_count].copy_from_slice(&positions);
				frame.sync_buffer(self.vertex_positions_buffer);

				let index_buffer = frame.get_mut_buffer_slice(self.indeces_buffer);

				index_buffer[index_buffer_offset..][..index_count]
					.iter_mut()
					.zip(indices)
					.for_each(|(dst, src)| {
						*dst = src;
					});

				frame.sync_buffer(self.indeces_buffer);

				mesh_ref.id()
			}
			_ => {
				log::warn!("SimpleRenderModel does not support non-generated meshes");
				return;
			}
		};

		let instace_id = self.mesh_buffers_stats.add_instance(mesh_id, handle);

		let instance_data_buffer = frame.get_mut_dynamic_buffer_slice(self.instance_data_buffer);

		let instance_batches = self.mesh_buffers_stats.get_instance_batches();

		instance_data_buffer[instace_id] = InstanceShaderData {
			instance_transform: entity.transform().get_matrix().into(),
		};
	}

	pub fn update_transform(&mut self, frame: &mut ghi::implementation::Frame, handle: Handle, transform: Matrix4) {
		let Some(idx) = self.mesh_buffers_stats.get_instance_id(handle) else {
			return;
		};

		let instance_data_buffer = frame.get_mut_dynamic_buffer_slice(self.instance_data_buffer);

		instance_data_buffer[idx] = InstanceShaderData {
			instance_transform: transform.into(),
		};
	}
}

impl crate::rendering::pipeline_manager::PipelineManager for PipelineManager {
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, sinks: &[Sink]) -> Option<Vec<Box<dyn RenderPassFunction>>> {
		let instance_batches = self.mesh_buffers_stats.get_instance_batches();

		let instance_batches = instance_batches.iter().into_vec();

		let commands = sinks
			.iter()
			.filter_map(|sink| {
				self.sinks
					.iter()
					.find(|sink_state| sink_state.index == sink.index())
					.map(|sink_state| (sink, sink_state))
			})
			.map(|(sink, sink_state)| {
				Box::new(sink_state.prepare(frame, sink, &self, &instance_batches)) as Box<dyn RenderPassFunction>
			})
			.collect::<Vec<_>>();

		Some(commands)
	}

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder) {
		let main = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				ghi::Formats::RGBA16UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage,
			)
			.name("main"),
		);
		let depth = render_pass_builder
			.create_render_target(ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::RenderTarget).name("depth"));
		self.sinks.push(RenderPass::new(
			render_pass_builder.context(),
			&self.descriptor_set_template,
			self.camera_data_buffer.into(),
			self.instance_data_buffer.into(),
			sink_id,
		))
	}
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct InstanceShaderData {
	instance_transform: ShaderMatrix4,
}
