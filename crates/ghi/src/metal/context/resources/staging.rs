use super::super::*;

impl Context {
	pub fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		let handle = self.buffers.nth_handle(buffer_handle.into(), 0).unwrap();
		let buffer = self.buffers.resource(handle);
		if buffer.staging.is_some() {
			self.pending_buffer_syncs.push_back(handle);
		}
	}

	/// Appends every pending buffer and image upload to one Metal 4 compute submission.
	pub(super) fn flush_pending_uploads(&mut self, queue_handle: graphics_hardware_interface::QueueHandle, sequence_index: u8) {
		if self.pending_buffer_syncs.is_empty() && self.pending_image_syncs.is_empty() {
			return;
		}

		let queue_index = queue_handle.0 as usize;
		let mut command_buffer = self.create_metal_command_buffer(queue_handle, Some("Pending Uploads"));
		let transfer_encoder = command_buffer.compute_command_encoder().expect(
			"Metal 4 transfer encoder creation failed. The most likely cause is that the command buffer is in an invalid state.",
		);
		let mut resource_tracker = std::mem::take(&mut self.queues[queue_index].resource_tracker);
		resource_tracker.begin_recording();
		let scope = synchronization::MetalEncoderScope::Encoder(0);
		#[cfg(debug_assertions)]
		if self.settings.debug_labels {
			transfer_encoder.setLabel(Some(&NSString::from_str("Pending Uploads")));
		}

		while let Some(buffer_handle) = self.pending_buffer_syncs.pop_front() {
			let buffer = self.buffers.resource(buffer_handle);
			let Some(staging_handle) = buffer.staging else {
				continue;
			};
			let staging = self.buffers.resource(staging_handle);
			command_buffer.retain_buffer(buffer.buffer.clone());
			command_buffer.retain_buffer(staging.buffer.clone());
			let barrier = resource_tracker.consume(
				scope,
				[
					synchronization::MetalResourceUse::buffer(
						staging_handle,
						0,
						buffer.size,
						mtl::MTLStages::Blit,
						crate::AccessPolicies::READ,
					),
					synchronization::MetalResourceUse::buffer(
						buffer_handle,
						0,
						buffer.size,
						mtl::MTLStages::Blit,
						crate::AccessPolicies::WRITE,
					),
				],
			);
			barrier.encode_compute(transfer_encoder.as_ref());
			unsafe {
				transfer_encoder.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
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
			command_buffer.retain_texture(image.texture.clone());
			let barrier = resource_tracker.consume(
				scope,
				[synchronization::MetalResourceUse::image(
					image_handle,
					Some(0),
					None,
					mtl::MTLStages::Blit,
					crate::AccessPolicies::WRITE,
				)],
			);
			barrier.encode_compute(transfer_encoder.as_ref());
			if let Some(upload_buffer) = crate::metal::command_buffer::encode_texture_upload(
				self.device.as_ref(),
				transfer_encoder.as_ref(),
				image.texture.as_ref(),
				image.format,
				image.extent,
				image.array_layers,
				staging,
			) {
				command_buffer.retain_buffer(upload_buffer);
			}
		}

		transfer_encoder.endEncoding();
		resource_tracker.finish_recording();
		self.queues[queue_index].resource_tracker = resource_tracker;
		let synchronizer = self.internal_upload_synchronizer(sequence_index);
		let submitted = self.queues[queue_index].submit_batch(queue_handle, SmallVec::from_iter([command_buffer]));
		// The synchronizer owns the upload submission and its retained resources through completion.
		self.synchronizers.resource_mut(synchronizer).signal(submitted);
		self.internal_upload_queues[sequence_index as usize] = Some(queue_handle);
	}
}
