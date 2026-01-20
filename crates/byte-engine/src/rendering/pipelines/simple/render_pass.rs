//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

use core::slice::SlicePattern;
use std::{collections::{hash_map::Entry, VecDeque}, sync::Arc};

use besl::ParserNode;
use ghi::{command_buffer::{BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecordable as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _}, device::Device as _, frame::Frame, Device};
use math::Matrix4;
use resource_management::{asset::material_asset_handler::ProgramGenerator, shader_generator::ShaderGenerationSettings, spirv_shader_generator::SPIRVShaderGenerator};
use utils::{hash::{HashMap, HashMapExt}, json::{self, JsonContainerTrait as _, JsonValueTrait as _}, sync::RwLock, Box, Extent};

use crate::{camera::Camera, core::{Entity, EntityHandle, entity::{self, EntityBuilder}, listener::{CreateEvent, Listener}}, gameplay::Transformable, rendering::{RenderableMesh, Viewport, common_shader_generator::CommonShaderScope, make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor, pipelines::simple::{CameraShaderData, SceneManager}, render_pass::{FramePrepare, RenderPassBuilder, RenderPassFunction, RenderPassReturn}, renderable::mesh::MeshSource, utils::{InstanceBatch, MeshBuffersStats, MeshStats}, view::View}};

pub struct RenderPass {
	pub(super) index: usize,
	descriptor_set: ghi::DescriptorSetHandle,
}

const VERTEX_LAYOUT: [ghi::VertexElement; 1] = [
	ghi::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0),
];

impl RenderPass {
	pub fn new(device: &mut ghi::Device, descriptor_set_layout: &ghi::DescriptorSetTemplateHandle, camera_data_buffer: ghi::BaseBufferHandle, instance_data_buffer: ghi::BaseBufferHandle, index: usize) -> Self {
		let camera_data_binding_template = ghi::DescriptorSetBindingTemplate::new(0, ghi::DescriptorType::StorageBuffer, ghi::Stages::VERTEX);
		let instance_data_binding_template = ghi::DescriptorSetBindingTemplate::new(1, ghi::DescriptorType::StorageBuffer, ghi::Stages::VERTEX);

		let descriptor_set = device.create_descriptor_set(None, &descriptor_set_layout);

		device.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&camera_data_binding_template, camera_data_buffer.into()));
		device.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::buffer(&instance_data_binding_template, instance_data_buffer.into()));

		Self {
			index,
			descriptor_set,
		}
	}
}

impl Entity for RenderPass {
}

impl RenderPass {
	pub(super) fn prepare(&self, frame: &mut ghi::Frame, viewport: &Viewport, sm: &SceneManager, instance_batches: &[InstanceBatch]) -> impl RenderPassFunction {
		let camera_data_buffer = sm.camera_data_buffer;

		let camera_data_buffer = frame.get_mut_dynamic_buffer_slice(camera_data_buffer);

		camera_data_buffer[viewport.index()] = CameraShaderData { vp: viewport.view_projection() };

		let vertex_buffer = sm.vertex_positions_buffer;
		let index_buffer = sm.indeces_buffer;
		let pipeline_layout = sm.pipeline_layout;
		let pipeline = sm.pipeline.clone();
		let descriptor_set = self.descriptor_set;

		let extent = viewport.extent();
		let instance_batches = instance_batches.iter().copied().collect::<Vec<_>>();

		move |c, t| {
			c.bind_vertex_buffers(&[vertex_buffer.into()]);
			c.bind_index_buffer(&index_buffer.into());

			let c = c.start_render_pass(extent, t);

			let c = c.bind_pipeline_layout(pipeline_layout);
			c.bind_descriptor_sets(&[descriptor_set]);
			let c = c.bind_raster_pipeline(pipeline);

			for batch in &instance_batches {
				c.write_push_constant(0, batch.base_instance() as u32);
				c.draw_indexed(batch.index_count() as u32, batch.instance_count() as u32, batch.base_index() as _, batch.base_vertex() as _, batch.base_instance() as _);
			}

			c.end_render_pass();
		}
	}
}
