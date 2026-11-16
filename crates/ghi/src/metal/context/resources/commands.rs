use super::super::*;

impl Context {
	pub(crate) fn create_command_buffer(
		&mut self,
		name: Option<&str>,
		queue_handle: graphics_hardware_interface::QueueHandle,
	) -> graphics_hardware_interface::CommandBufferHandle {
		self.command_buffers.push(StoredCommandBuffer {
			queue_handle,
			name: name.map(str::to_owned),
		});
		graphics_hardware_interface::CommandBufferHandle::from_index((self.command_buffers.len() - 1) as u64)
	}

	/// Records a workload, submits it on `queue_handle`, and waits until the GPU finished it.
	///
	/// See [`crate::context::Context::record`].
	pub(crate) fn record_workload<'a, R>(
		&'a mut self,
		queue_handle: graphics_hardware_interface::QueueHandle,
		name: Option<&str>,
		record: impl FnOnce(&mut super::super::CommandBufferRecording<'a>) -> R,
	) -> R {
		let synchronizer = synchronizer_for_sequence(&self.synchronizers, self.workload_synchronizer, 0);
		let this: *mut Self = self;
		let name = name.filter(|_| self.settings.debug_labels);
		let mut recording = self.start_recording(queue_handle, name, None, &std::alloc::Global);
		let result = record(&mut recording);
		recording.submit(synchronizer);
		// SAFETY: Submitting consumed the recording, which was the only borrow of the context.
		let this = unsafe { &mut *this };
		this.wait_for_private_synchronizer(synchronizer);
		// The uploads submitted ahead of the workload finish too, so nothing the GPU runs reads the transient pages and
		// the next workload reuses them.
		this.retire_all_internal_uploads();
		this.upload_arenas[this.frames as usize].reset();
		this.scratch_arenas[this.frames as usize].reset();
		result
	}

	/// Starts a recording on `queue_handle` after submitting the uploads it may depend on.
	pub(crate) fn start_recording<'a>(
		&'a mut self,
		queue_handle: graphics_hardware_interface::QueueHandle,
		name: Option<&str>,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
		allocator: &'a dyn std::alloc::Allocator,
	) -> super::super::CommandBufferRecording<'a> {
		// Same-queue uploads stay asynchronous; a queue switch waits for outstanding uploads on another queue because
		// pending writes have no public queue owner.
		for sequence_index in 0..self.internal_upload_queues.len() {
			if self.internal_upload_queues[sequence_index].is_some_and(|owner| owner != queue_handle) {
				self.retire_internal_uploads(sequence_index as u8);
			}
		}
		self.flush_pending_uploads(queue_handle, frame_key);

		super::super::CommandBufferRecording::new(self, queue_handle, name, frame_key, allocator)
	}
}
