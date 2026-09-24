use super::super::*;

impl Context {
	/// Waits for one private synchronizer and returns every completed command to its queue.
	pub(crate) fn wait_for_private_synchronizer(&mut self, synchronizer_handle: crate::synchronizer::SynchronizerHandle) {
		if let Some(error) = self.synchronizers.resource_mut(synchronizer_handle).wait(&mut self.queues) {
			panic!("{error}");
		}
	}

	pub(crate) fn start_frame<'a>(
		&'a mut self,
		index: u64,
		synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		queue_handle: graphics_hardware_interface::QueueHandle,
		allocator: &'a dyn std::alloc::Allocator,
	) -> crate::queue::StartedFrame<super::super::Frame<'a>> {
		let frame_key = graphics_hardware_interface::FrameKey {
			frame_index: index,
			sequence_index: (index % u64::from(self.frames)) as u8,
		};
		let completed_frame = crate::queue::completed_frame_key(index, self.frames);
		let synchronizer_handle = self.synchronizer_for_sequence(synchronizer_handle, frame_key.sequence_index);
		self.wait_for_private_synchronizer(synchronizer_handle);
		self.retire_internal_uploads(frame_key.sequence_index);
		// Every command that read this sequence's upload pages has completed, so the pages can be rewound.
		self.upload_arenas[frame_key.sequence_index as usize].reset();
		self.process_tasks(frame_key.sequence_index);
		crate::queue::StartedFrame::new(
			super::super::Frame::new(self, frame_key, queue_handle, allocator),
			completed_frame,
		)
	}
}
