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

	/// Creates a recording that belongs to no frame, for transfers submitted outside the render loop.
	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> super::super::CommandBufferRecording<'a> {
		self.create_command_buffer_recording_with_frame_key_in(command_buffer_handle, None, &std::alloc::Global)
	}

	/// Creates a recording for one GHI command buffer after submitting the uploads it may depend on.
	pub(crate) fn create_command_buffer_recording_with_frame_key_in<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
		allocator: &'a dyn std::alloc::Allocator,
	) -> super::super::CommandBufferRecording<'a> {
		let (queue_handle, command_buffer_name) = {
			let command_buffer = &self.command_buffers[command_buffer_handle.0 as usize];
			let name = self.settings.debug_labels.then(|| command_buffer.name.clone()).flatten();
			(command_buffer.queue_handle, name)
		};

		// Detached recordings have no completion point that could recycle pages, so they start from empty pages and
		// release the upload submissions that finished since the last one.
		if frame_key.is_none() {
			// The transient arena follows the frames' retained arenas.
			self.upload_arenas[self.frames as usize].discard();
			self.retire_completed_internal_uploads();
		}
		// Same-queue uploads stay asynchronous; a queue switch waits for outstanding uploads on another queue because
		// pending writes have no public queue owner.
		for sequence_index in 0..self.internal_upload_queues.len() {
			if self.internal_upload_queues[sequence_index].is_some_and(|owner| owner != queue_handle) {
				self.retire_internal_uploads(sequence_index as u8);
			}
		}
		self.flush_pending_uploads(queue_handle, frame_key);

		super::super::CommandBufferRecording::new(self, queue_handle, command_buffer_name.as_deref(), frame_key, allocator)
	}
}
