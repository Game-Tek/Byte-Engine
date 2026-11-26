use super::super::*;
use crate::command_buffer::CommonCommandBufferMode as _;

impl Context {
	/// Records every pending buffer clear and acceleration-structure compaction into one Metal 4 submission on
	/// `queue_handle`.
	///
	/// The work uses the same recording and hazard-tracking code as caller recordings, and signals the internal upload
	/// synchronizer of the frame sequence `frame_key` selects.
	pub(crate) fn flush_pending_uploads(
		&mut self,
		queue_handle: graphics_hardware_interface::QueueHandle,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
	) {
		if self.pending_buffer_clears.is_empty() && self.compaction_copies.is_empty() {
			return;
		}

		// The recording borrows the context, so the queues move out and come back empty with their capacity.
		let mut buffer_clears = std::mem::take(&mut self.pending_buffer_clears);
		let mut compaction_copies = std::mem::take(&mut self.compaction_copies);
		let sequence_index = frame_key.map_or(0, |key| key.sequence_index);
		let synchronizer = synchronizer_for_sequence(&self.synchronizers, self.internal_upload_synchronizer, sequence_index);
		let mut recording =
			CommandBufferRecording::new(self, queue_handle, Some("Pending Uploads"), frame_key, &std::alloc::Global);
		// The region names the upload encoder in capture tools, as "Compute: Pending Uploads".
		recording.start_region(|label| label.write_str("Pending Uploads"));
		for buffer_handle in buffer_clears.drain(..) {
			recording.clear_buffer(buffer_handle);
		}
		recording.compact_acceleration_structures(&compaction_copies);
		compaction_copies.clear();
		recording.end_region();
		// The synchronizer owns the upload submission and its retained resources through completion.
		recording.submit(synchronizer);

		self.pending_buffer_clears = buffer_clears;
		self.compaction_copies = compaction_copies;
		self.internal_upload_queues[sequence_index as usize] = Some(queue_handle);
	}
}
