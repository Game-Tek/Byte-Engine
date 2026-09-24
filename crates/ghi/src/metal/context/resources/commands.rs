use super::super::*;

impl Context {
	/// Selects the frame's retained arena, or the transient arena for recordings outside any frame.
	pub(crate) fn upload_arena_index(&self, frame_key: Option<graphics_hardware_interface::FrameKey>) -> usize {
		frame_key.map_or(self.frames as usize, |key| key.sequence_index as usize)
	}

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

	/// Creates a recording that belongs to no frame, for transfers submitted outside the render loop.
	pub fn create_command_buffer_recording<'a>(
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
		// SAFETY: Detached recordings create and drain the pool on their owning thread.
		let autorelease_pool = frame_key.is_none().then(|| unsafe { NSAutoreleasePool::new() });
		let sequence_index = frame_key.map(|key| key.sequence_index).unwrap_or(0);
		let (queue_handle, command_buffer_name) = {
			let command_buffer = &self.command_buffers[command_buffer_handle.0 as usize];
			let name = self.settings.debug_labels.then(|| command_buffer.name.clone()).flatten();
			(command_buffer.queue_handle, name)
		};

		// Detached recordings have no completion point that could recycle pages, so they start from empty pages.
		let arena_index = self.upload_arena_index(frame_key);
		if frame_key.is_none() {
			self.upload_arenas[arena_index].discard();
		}
		// Same-queue uploads stay asynchronous; a queue switch waits because pending writes have no public queue owner.
		self.synchronize_internal_upload_queue(queue_handle);
		self.flush_pending_uploads(queue_handle, sequence_index, arena_index);

		let mtl_command_buffer = self.create_metal_command_buffer(queue_handle, command_buffer_name.as_deref());

		let recording_device = super::super::command_buffer::RecordingDevice {
			metal_device: self.device.as_ref(),
			buffers: &self.buffers,
			images: &self.images,
			samplers: &self.samplers,
			acceleration_structures: &self.acceleration_structures,
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
			descriptor_sets: &mut self.descriptor_sets,
			upload_arena: &mut self.upload_arenas[arena_index],
			argument_tables: &mut self.argument_tables,
		};

		super::super::CommandBufferRecording::new(
			recording_device,
			commit,
			command_buffer_handle,
			mtl_command_buffer,
			frame_key,
			autorelease_pool,
			allocator,
		)
	}
}
