//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
		CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
	},
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};

use crate::{
	core::Entity,
	rendering::{
		Sink,
		pipelines::simple::{CameraShaderData, PipelineManager},
		render_pass::RenderPassFunction,
		utils::InstanceBatch,
	},
};

pub struct RenderPass {
	pub(super) index: usize,
	descriptor_set: ghi::DescriptorSetHandle,
	background: Option<crate::rendering::render_pass::SceneBackground>,
}

impl RenderPass {
	pub fn new(
		context: &mut ghi::implementation::Context,
		camera_data_buffer: ghi::BaseBufferHandle,
		instance_data_buffer: ghi::BaseBufferHandle,
		index: usize,
		background: Option<crate::rendering::render_pass::SceneBackground>,
	) -> Self {
		let descriptor_set = context.create_descriptor_set(None);

		context.write(&[
			ghi::DescriptorWrite::buffer(descriptor_set, ghi::ResourceSlot::new(0), camera_data_buffer),
			ghi::DescriptorWrite::buffer(descriptor_set, ghi::ResourceSlot::new(1), instance_data_buffer),
		]);

		Self {
			index,
			descriptor_set,
			background,
		}
	}
}

impl Entity for RenderPass {}

impl RenderPass {
	pub(super) fn prepare<'a>(
		&self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		sm: &PipelineManager,
		pipeline: ghi::PipelineHandle,
		instance_batches: &'a [InstanceBatch],
		frame_allocator: &'a bumpalo::Bump,
	) -> impl RenderPassFunction + 'a {
		// The simple model has no transparent surfaces, so its background is drawn after every surface.
		let background = self
			.background
			.as_ref()
			.and_then(|background| background.prepare(frame, sink, frame_allocator));

		let camera_data_buffer = sm.camera_data_buffer;

		let camera_data_buffer = frame.get_mut_dynamic_buffer_slice(camera_data_buffer);

		camera_data_buffer[sink.index()] = CameraShaderData {
			vp: sink.view_projection().into(),
		};

		let vertex_buffer = sm.vertex_positions_buffer;

		let index_buffer = sm.indices_buffer;

		let descriptor_set = self.descriptor_set;

		let extent = sink.extent();

		move |c, t| {
			c.bind_vertex_buffers(&[vertex_buffer.into()]);

			c.bind_index_buffer(&ghi::BufferDescriptor::new(index_buffer).index_type(ghi::DataTypes::U16));

			let render_pass = c.start_render_pass(extent, t);

			let render_pass = render_pass.bind_raster_pipeline(pipeline);

			render_pass.bind_descriptor_sets(&[descriptor_set]);

			for batch in instance_batches.iter() {
				render_pass.write_push_constant(0, batch.base_instance() as u32);

				render_pass.draw_indexed(
					batch.index_count() as u32,
					batch.instance_count() as u32,
					batch.base_index() as _,
					batch.base_vertex() as _,
					batch.base_instance() as _,
				);
			}

			render_pass.end_render_pass();

			if let Some(background) = background {
				background(c, t);
			}
		}
	}
}
