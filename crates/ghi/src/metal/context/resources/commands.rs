use super::super::*;

impl Context {
	pub(crate) fn create_command_buffer(
		&mut self,
		name: Option<&str>,
		queue_handle: graphics_hardware_interface::QueueHandle,
	) -> graphics_hardware_interface::CommandBufferHandle {
		self.command_buffers.push(StoredCommandBuffer {
			queue_handle,
			name: crate::debug_name(name),
		});
		graphics_hardware_interface::CommandBufferHandle((self.command_buffers.len() - 1) as u64)
	}

	pub(crate) fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> super::super::CommandBufferRecording<'a> {
		self.create_command_buffer_recording_with_frame_key_in(command_buffer_handle, None, &std::alloc::Global)
	}

	pub(crate) fn create_command_buffer_recording_with_frame_key_in<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
		allocator: &'a dyn std::alloc::Allocator,
	) -> super::super::CommandBufferRecording<'a> {
		let autorelease_pool = frame_key.is_none().then(|| unsafe { NSAutoreleasePool::new() });
		let sequence_index = frame_key.map(|key| key.sequence_index).unwrap_or(0);
		let (queue_handle, command_buffer_name) = {
			let command_buffer = &self.command_buffers[command_buffer_handle.0 as usize];
			let name = self.settings.debug_labels.then(|| command_buffer.name.clone()).flatten();
			(command_buffer.queue_handle, name)
		};

		// Same-queue uploads stay asynchronous; a queue switch waits because pending writes have no public queue owner.
		self.synchronize_internal_upload_queue(queue_handle);
		self.flush_pending_uploads(queue_handle, sequence_index);

		let mtl_command_buffer = self.create_metal_command_buffer(queue_handle, command_buffer_name.as_deref());

		let recording_device = super::super::command_buffer::RecordingDevice {
			metal_device: self.device.as_ref(),
			buffers: &self.buffers,
			images: &self.images,
			samplers: &self.samplers,
			acceleration_structures: &self.acceleration_structures,
			pipeline_layouts: &self.pipeline_layouts,
			descriptor_sets: &self.descriptor_sets,
			meshes: &self.meshes,
			pipelines: &self.pipelines,
			swapchains: &self.swapchains,
			debug_labels: self.settings.debug_labels,
		};
		let commit = super::super::command_buffer::RecordingCommit {
			queue_handle,
			queue: &mut self.queues[queue_handle.0 as usize],
			synchronizers: &mut self.synchronizers,
			texture_readbacks: &mut self.texture_readbacks,
		};

		super::super::CommandBufferRecording::new(
			recording_device,
			commit,
			command_buffer_handle,
			mtl_command_buffer,
			frame_key,
			Vec::new_in(allocator),
			autorelease_pool,
			allocator,
		)
	}
}
