//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

use core::slice::SlicePattern;
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
	frame::Frame,
};
use resource_management::{asset::bema_asset_handler::ProgramGenerator, shader::generator::ShaderGenerationSettings};
use utils::{
	hash::{HashMap, HashMapExt},
	json::{self, JsonContainerTrait as _, JsonValueTrait as _},
	sync::RwLock,
	Box, Extent,
};

use crate::{
	core::{
		entity::{self},
		listener::Listener,
		Entity, EntityHandle,
	},
	rendering::Camera,
	rendering::{
		common_shader_generator::CommonShaderScope,
		make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor,
		pipelines::simple::{CameraShaderData, PipelineManager},
		render_pass::{FramePrepare, RenderPassBuilder, RenderPassFunction, RenderPassReturn},
		renderable::mesh::MeshSource,
		utils::{InstanceBatch, MeshBuffersStats, MeshStats},
		view::View,
		RenderableMesh, Sink,
	},
	space::Transformable,
};

pub struct RenderPass {
	pub(super) index: usize,
	descriptor_set: ghi::DescriptorSetHandle,
}

const VERTEX_LAYOUT: [ghi::pipelines::VertexElement; 1] =
	[ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0)];

impl RenderPass {
	pub fn new(
		context: &mut ghi::implementation::Context,
		camera_data_buffer: ghi::BaseBufferHandle,
		instance_data_buffer: ghi::BaseBufferHandle,
		index: usize,
	) -> Self {
		let descriptor_set = context.create_descriptor_set(None);
		context.write(&[
			ghi::DescriptorWrite::buffer(descriptor_set, ghi::ResourceSlot::new(0), camera_data_buffer),
			ghi::DescriptorWrite::buffer(descriptor_set, ghi::ResourceSlot::new(1), instance_data_buffer),
		]);

		Self { index, descriptor_set }
	}
}

impl Entity for RenderPass {}

impl RenderPass {
	pub(super) fn prepare<'a>(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		sm: &PipelineManager,
		instance_batches: &'a [InstanceBatch],
	) -> impl RenderPassFunction + 'a {
		let camera_data_buffer = sm.camera_data_buffer;

		let camera_data_buffer = frame.get_mut_dynamic_buffer_slice(camera_data_buffer);

		camera_data_buffer[sink.index()] = CameraShaderData {
			vp: sink.view_projection().into(),
		};

		let vertex_buffer = sm.vertex_positions_buffer;
		let index_buffer = sm.indeces_buffer;
		let pipeline = sm.pipeline;
		let descriptor_set = self.descriptor_set;

		let extent = sink.extent();

		move |c, t| {
			c.bind_vertex_buffers(&[vertex_buffer.into()]);
			c.bind_index_buffer(&ghi::BufferDescriptor::new(index_buffer).index_type(ghi::DataTypes::U16));

			let c = c.start_render_pass(extent, t);

			let c = c.bind_raster_pipeline(pipeline);
			c.bind_descriptor_sets(&[descriptor_set]);

			for batch in instance_batches.iter() {
				c.write_push_constant(0, batch.base_instance() as u32);
				c.draw_indexed(
					batch.index_count() as u32,
					batch.instance_count() as u32,
					batch.base_index() as _,
					batch.base_vertex() as _,
					batch.base_instance() as _,
				);
			}

			c.end_render_pass();
		}
	}
}
