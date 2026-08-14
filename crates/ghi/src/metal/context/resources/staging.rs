use super::super::*;

impl Context {
	pub fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		let handle = self.buffers.nth_handle(buffer_handle.into(), 0).unwrap();
		let buffer = self.buffers.resource(handle);
		if buffer.staging.is_some() {
			self.pending_buffer_syncs.push_back(handle);
		}
	}

	/// Appends every pending buffer and image upload to one Metal blit submission.
	pub(super) fn flush_pending_uploads(
		&mut self,
		queue_handle: Option<graphics_hardware_interface::QueueHandle>,
		sequence_index: u8,
	) {
		if self.pending_buffer_syncs.is_empty() && self.pending_image_syncs.is_empty() {
			return;
		}

		let queue = queue_handle
			.and_then(|queue_handle| self.queues.get(queue_handle.0 as usize))
			.unwrap_or_else(|| self.transfer_queue());
		let command_buffer = self.create_metal_command_buffer(
			queue.queue.as_ref(),
			Some("Pending Uploads"),
			"Metal upload command buffer creation failed. The most likely cause is that the transfer queue did not provide a command buffer.",
		);
		let blit_encoder = command_buffer.blitCommandEncoder().expect(
			"Metal blit command encoder creation failed. The most likely cause is that the command buffer is in an invalid state.",
		);
		#[cfg(debug_assertions)]
		if self.settings.debug_labels {
			blit_encoder.setLabel(Some(&NSString::from_str("Pending Uploads")));
		}

		while let Some(buffer_handle) = self.pending_buffer_syncs.pop_front() {
			let buffer = self.buffers.resource(buffer_handle);
			let Some(staging_handle) = buffer.staging else {
				continue;
			};
			let staging = self.buffers.resource(staging_handle);
			unsafe {
				blit_encoder.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
					staging.buffer.as_ref(),
					0,
					buffer.buffer.as_ref(),
					0,
					buffer.size as _,
				);
			}
		}

		while let Some(image_handle) = self.pending_image_syncs.pop_front() {
			let image = self.images.resource(image_handle);
			let Some(staging) = image.staging.as_ref() else {
				continue;
			};
			self.encode_texture_upload(
				blit_encoder.as_ref(),
				image.texture.as_ref(),
				image.format,
				image.extent,
				image.array_layers,
				staging,
			);
		}

		blit_encoder.endEncoding();
		self.submit_internal_metal_command_buffer(command_buffer, sequence_index);
	}
}
