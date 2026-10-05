use super::super::*;
use crate::command_buffer::{CommandBufferRecording as _, CommonCommandBufferMode as _};

impl Context {
	/// Records every pending buffer and image upload into one Metal 4 submission on `queue_handle`.
	///
	/// The uploads use the same recording, copy, and hazard-tracking code as caller recordings, and signal the
	/// internal upload synchronizer of the frame sequence `frame_key` selects.
	pub(super) fn flush_pending_uploads(
		&mut self,
		queue_handle: graphics_hardware_interface::QueueHandle,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
	) {
		if self.pending_buffer_syncs.is_empty() && self.pending_image_syncs.is_empty() {
			return;
		}

		// The recording borrows the context, so the queues move out and come back empty with their capacity.
		let mut buffer_syncs = std::mem::take(&mut self.pending_buffer_syncs);
		let mut image_syncs = std::mem::take(&mut self.pending_image_syncs);
		let synchronizer = self.internal_upload_synchronizer;
		let mut recording =
			CommandBufferRecording::new(self, queue_handle, Some("Pending Uploads"), frame_key, &std::alloc::Global);
		// The region names the upload encoder in capture tools, as "Compute: Pending Uploads".
		recording.start_region(|label| label.write_str("Pending Uploads"));
		for buffer_handle in buffer_syncs.drain(..) {
			recording.sync_private_buffer(buffer_handle);
		}
		for (image_handle, region) in image_syncs.drain(..) {
			recording.sync_image(image_handle, region);
		}
		recording.end_region();
		// The synchronizer owns the upload submission and its retained resources through completion.
		recording.execute(synchronizer);

		self.pending_buffer_syncs = buffer_syncs;
		self.pending_image_syncs = image_syncs;
		let sequence_index = frame_key.map_or(0, |key| key.sequence_index);
		self.internal_upload_queues[sequence_index as usize] = Some(queue_handle);
	}
}
