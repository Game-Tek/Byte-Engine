use super::*;

/// Resolves one packed image source layout and rejects pitches that cannot contain complete rows or images.
fn validated_texture_source_layout(
	row_bytes: usize,
	row_count: usize,
	row_pitch: usize,
	image_pitch: usize,
	image_count: usize,
) -> (usize, usize, usize) {
	let row_pitch = if row_pitch == 0 { row_bytes } else { row_pitch };
	assert!(
		row_pitch >= row_bytes,
		"DX12 texture source row pitch is too small. The most likely cause is that upload metadata omits bytes required by one image row.",
	);
	let minimum_image_pitch = row_pitch.checked_mul(row_count).expect(
		"DX12 texture source image pitch overflowed. The most likely cause is that the row pitch or row count exceeds the host address range.",
	);
	let image_pitch = if image_pitch == 0 { minimum_image_pitch } else { image_pitch };
	assert!(
		image_pitch >= minimum_image_pitch,
		"DX12 texture source image pitch is too small. The most likely cause is that upload metadata omits rows required by one image or array layer.",
	);
	let required_bytes = if image_count == 0 {
		0
	} else {
		(image_count - 1)
			.checked_mul(image_pitch)
			.and_then(|offset| offset.checked_add(minimum_image_pitch))
			.expect(
				"DX12 texture source range overflowed. The most likely cause is that the image pitch or layer count exceeds the host address range.",
			)
	};
	(row_pitch, image_pitch, required_bytes)
}

impl Device {
	pub(crate) fn copy_buffer_to_images(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copies: &[crate::BufferImageCopyDescriptor],
		sequence_index: u8,
	) {
		for copy in copies {
			self.copy_buffer_to_image(copy, sequence_index);
			self.record_buffer_to_image_copy(command_buffer_handle, copy, sequence_index);
		}
	}

	pub(crate) fn copy_buffer_to_image(&mut self, copy: &crate::BufferImageCopyDescriptor, sequence_index: u8) {
		let Some(image) = self.images.get(copy.destination_image.0 as usize) else {
			return;
		};
		let Some((row_bytes, row_count, compact_bytes_per_image)) = utils::texture_copy_layout(image.format, image.extent)
		else {
			return;
		};
		let extent = image.extent;
		let depth = extent.depth().max(1) as usize;
		let (row_stride, image_stride, required_source_bytes) = validated_texture_source_layout(
			row_bytes,
			row_count,
			copy.source_bytes_per_row,
			copy.source_bytes_per_image,
			depth,
		);
		let Some((source_data, source_len)) =
			self.buffer_range_parts_for_sequence(copy.source_buffer, copy.source_offset, required_source_bytes, sequence_index)
		else {
			return;
		};
		// SAFETY: The range resolver returns a non-null pointer whose checked range lies in stable buffer storage.
		let source_bytes = unsafe { std::slice::from_raw_parts(source_data, source_len) };
		let Some(destination) = self.image_data_mut_for_sequence(copy.destination_image, sequence_index) else {
			return;
		};

		for layer in 0..depth {
			for y in 0..row_count {
				let source_start = layer
					.checked_mul(image_stride)
					.and_then(|offset| y.checked_mul(row_stride).and_then(|row| offset.checked_add(row)))
					.expect(
						"DX12 texture source row offset overflowed. The most likely cause is invalid image pitch metadata.",
					);
				let source_end = source_start.checked_add(row_bytes).expect(
					"DX12 texture source row range overflowed. The most likely cause is invalid image extent metadata.",
				);
				let destination_start = layer
					.checked_mul(compact_bytes_per_image)
					.and_then(|offset| y.checked_mul(row_bytes).and_then(|row| offset.checked_add(row)))
					.expect(
						"DX12 texture destination row offset overflowed. The most likely cause is invalid image extent metadata.",
					);
				let destination_end = destination_start.checked_add(row_bytes).expect(
					"DX12 texture destination row range overflowed. The most likely cause is invalid image extent metadata.",
				);
				if source_end > source_bytes.len() || destination_end > destination.len() {
					panic!(
						"Failed to copy DX12 buffer data into an image. The most likely cause is that the source row layout or destination image extent is invalid."
					);
				}
				destination[destination_start..destination_end].copy_from_slice(&source_bytes[source_start..source_end]);
			}
		}
	}

	pub(crate) fn record_buffer_to_image_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copy: &crate::BufferImageCopyDescriptor,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let destination = self.ensure_image_resource_for_sequence(copy.destination_image, sequence_index);
		let Some(image) = self.images.get(copy.destination_image.0 as usize) else {
			return;
		};
		if copy.destination_mip_level >= image.mip_levels {
			panic!(
				"DX12 texture copy mip level is out of range. The most likely cause is that the upload metadata does not match the allocated image."
			);
		}
		let extent = crate::image::mip_extent(image.extent, copy.destination_mip_level);
		let (Some(destination), Some((row_bytes, row_count, _))) =
			(destination, utils::texture_copy_layout(image.format, extent))
		else {
			return;
		};

		let array_layers = image.array_layers.max(1) as usize;
		let mip_levels = image.mip_levels;
		let image_count = (extent.depth().max(1) as usize).checked_mul(array_layers).expect(
			"DX12 texture source layer count overflowed. The most likely cause is invalid image depth or array metadata.",
		);
		let (source_row_pitch, source_image_pitch, required_source_bytes) = validated_texture_source_layout(
			row_bytes,
			row_count,
			copy.source_bytes_per_row,
			copy.source_bytes_per_image,
			image_count,
		);
		let Some((source_data, source_len)) =
			self.buffer_range_parts_for_sequence(copy.source_buffer, copy.source_offset, required_source_bytes, sequence_index)
		else {
			return;
		};
		// SAFETY: The range resolver returns a non-null pointer whose checked range lies in stable buffer storage.
		let source_bytes = unsafe { std::slice::from_raw_parts(source_data, source_len) };
		let source_array_pitch = source_image_pitch.checked_mul(extent.depth().max(1) as usize).expect(
			"DX12 texture array pitch overflowed. The most likely cause is invalid source pitch or image depth metadata.",
		);
		for layer in 0..array_layers {
			let start = layer
				.checked_mul(source_array_pitch)
				.expect("DX12 texture array offset overflowed. The most likely cause is invalid source pitch metadata.");
			let end = start
				.checked_add(source_array_pitch)
				.expect("DX12 texture array range overflowed. The most likely cause is invalid source pitch metadata.");
			assert!(
				end <= source_bytes.len(),
				"DX12 texture array range exceeds the source buffer. The most likely cause is that source_bytes_per_image does not cover every copied array layer.",
			);
			let destination_subresource = layer
				.checked_mul(mip_levels as usize)
				.and_then(|base| base.checked_add(copy.destination_mip_level as usize))
				.and_then(|subresource| u32::try_from(subresource).ok())
				.expect(
					"DX12 texture destination subresource overflowed. The most likely cause is invalid array-layer or mip metadata.",
				);
			self.record_image_upload(
				command_buffer_handle,
				&command_list,
				copy.destination_image,
				destination.clone(),
				extent,
				&source_bytes[start..end],
				source_row_pitch,
				source_image_pitch,
				destination_subresource,
			);
		}
	}

	pub(crate) fn record_image_data_write(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		data: &[RGBAu8],
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let destination = self.ensure_image_resource_for_sequence(image_handle.0, sequence_index);
		let Some(image) = self.images.get(image_handle.0.0 as usize) else {
			return;
		};
		let (Some(destination), Some((source_row_pitch, row_count, _))) =
			(destination, utils::texture_copy_layout(image.format, image.extent))
		else {
			return;
		};
		let extent = image.extent;
		let source_bytes = bytemuck::cast_slice(data);
		if self.record_image_upload(
			command_buffer_handle,
			&command_list,
			image_handle.0,
			destination,
			extent,
			source_bytes,
			source_row_pitch,
			source_row_pitch * row_count,
			0,
		) {
			self.gpu_uploaded_images.insert(image_handle.0);
		}
	}

	/// Uploads only pending image data selected for the current command buffer and frame.
	pub(crate) fn flush_pending_texture_syncs(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_filter: Option<crate::BaseImageHandle>,
		sequence_filter: Option<u8>,
	) {
		// Visit only entries that were pending when the flush started. Failed uploads move to the
		// end for a later recording, and the same allocation remains available for future frames.
		let mut pending = std::mem::take(&mut self.pending_texture_syncs);
		let mut remaining = pending.len();
		let mut index = 0;
		while remaining > 0 {
			let (image_handle, sequence_index) = pending[index];
			remaining -= 1;
			let image_mismatch = image_filter.is_some_and(|filter| filter != image_handle);
			let sequence_mismatch = sequence_filter.is_some_and(|filter| filter != sequence_index);
			if image_mismatch || sequence_mismatch {
				index += 1;
				continue;
			}

			pending.swap_remove(index);
			if self.record_image_storage_upload(command_buffer_handle, ImageHandle(image_handle), sequence_index) {
				if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) {
					command_buffer.recorded_texture_syncs.push((image_handle, sequence_index));
				}
			} else {
				pending.push((image_handle, sequence_index));
			}
		}
		self.pending_texture_syncs = pending;
	}

	/// Restores texture uploads whose command list was reset or abandoned before submission.
	pub(crate) fn requeue_recorded_texture_syncs_for_command_buffer(&mut self, command_buffer_handle: CommandBufferHandle) {
		let mut recorded = self
			.command_buffers
			.get_mut(command_buffer_handle.0 as usize)
			.map(|command_buffer| std::mem::take(&mut command_buffer.recorded_texture_syncs))
			.unwrap_or_default();
		for (image_handle, sequence_index) in recorded.drain(..) {
			self.queue_texture_sync_for_sequence(image_handle, sequence_index);
		}
		if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) {
			command_buffer.recorded_texture_syncs = recorded;
		}
	}

	pub(crate) fn flush_pending_texture_syncs_for_sequence(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		sequence_filter: u8,
	) {
		self.flush_pending_texture_syncs(command_buffer_handle, None, Some(sequence_filter));
	}

	pub(crate) fn record_image_storage_upload(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		sequence_index: u8,
	) -> bool {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return false;
		};
		let destination = self.ensure_image_resource_for_sequence(image_handle.0, sequence_index);
		let image_index = image_handle.0.0 as usize;
		let Some(image) = self.images.get(image_index) else {
			return false;
		};
		let (Some(destination), Some((source_row_pitch, row_count, _))) =
			(destination, utils::texture_copy_layout(image.format, image.extent))
		else {
			return false;
		};
		let extent = image.extent;
		let source_image_pitch = source_row_pitch * row_count;
		let frame_data_index = image
			.frame_data
			.as_ref()
			.and_then(|frames| (!frames.is_empty()).then(|| (sequence_index as usize).min(frames.len() - 1)));
		let source_bytes = if let Some(frame_data_index) = frame_data_index {
			std::mem::take(&mut self.images[image_index].frame_data.as_mut().unwrap()[frame_data_index])
		} else {
			let Some(source_bytes) = self.images[image_index].data.take() else {
				return false;
			};
			source_bytes
		};
		let recorded = self.record_image_upload(
			command_buffer_handle,
			&command_list,
			image_handle.0,
			destination,
			extent,
			&source_bytes,
			source_row_pitch,
			source_image_pitch,
			0,
		);
		if let Some(frame_data_index) = frame_data_index {
			self.images[image_index].frame_data.as_mut().unwrap()[frame_data_index] = source_bytes;
		} else {
			self.images[image_index].data = Some(source_bytes);
		}
		if recorded {
			self.gpu_uploaded_images.insert(image_handle.0);
		}
		recorded
	}

	pub(crate) fn begin_debug_region(&self, command_buffer_handle: CommandBufferHandle, name: &str) {
		if !self.settings.debug_labels {
			return;
		}

		let Some(command_list) = self.command_buffers[command_buffer_handle.0 as usize].command_list.as_ref() else {
			return;
		};

		// Metadata version zero tells PIX to decode the payload as a null-terminated UTF-16 event
		// name. Keep the encoded name alive until BeginEvent has copied it into the command list.
		let mut encoded_name = name.encode_utf16().collect::<SmallVec<[u16; 128]>>();
		encoded_name.push(0);
		let encoded_size = u32::try_from(std::mem::size_of_val(encoded_name.as_slice())).expect(
			"PIX debug label is too long. The most likely cause is a generated label larger than the DX12 event-size limit.",
		);
		unsafe {
			command_list.BeginEvent(0, Some(encoded_name.as_ptr().cast()), encoded_size);
		}
		self.debug_region_begin_count.set(self.debug_region_begin_count.get() + 1);
	}

	pub(crate) fn end_debug_region(&self, command_buffer_handle: CommandBufferHandle) {
		if !self.settings.debug_labels {
			return;
		}

		let Some(command_list) = self.command_buffers[command_buffer_handle.0 as usize].command_list.as_ref() else {
			return;
		};

		unsafe {
			command_list.EndEvent();
		}
		self.debug_region_end_count.set(self.debug_region_end_count.get() + 1);
	}

	pub(crate) fn record_image_upload(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList7,
		image_handle: crate::BaseImageHandle,
		destination: ID3D12Resource,
		extent: Extent,
		source_bytes: &[u8],
		source_row_pitch: usize,
		source_image_pitch: usize,
		destination_mip_level: u32,
	) -> bool {
		let Some((row_bytes, row_count, _)) = utils::texture_copy_layout(self.images[image_handle.0 as usize].format, extent)
		else {
			return false;
		};
		let Some(footprint) = self.native_texture_copy_footprint(&destination, destination_mip_level) else {
			return false;
		};
		let depth = extent.depth().max(1) as usize;
		let Ok(native_depth) = usize::try_from(footprint.placed.Footprint.Depth) else {
			return false;
		};
		if footprint.row_size != row_bytes || footprint.row_count != row_count || native_depth != depth {
			return false;
		}
		let Ok(upload_row_pitch) = usize::try_from(footprint.placed.Footprint.RowPitch) else {
			return false;
		};
		let upload_image_pitch = upload_row_pitch.checked_mul(row_count).expect(
			"DX12 texture upload image pitch overflowed. The most likely cause is an image extent that exceeds the host address range.",
		);
		let upload_size = footprint.total_size;
		let native_required = depth
			.saturating_sub(1)
			.checked_mul(upload_image_pitch)
			.and_then(|offset| {
				row_count
					.saturating_sub(1)
					.checked_mul(upload_row_pitch)
					.and_then(|row| offset.checked_add(row))
			})
			.and_then(|offset| offset.checked_add(row_bytes));
		if native_required.is_none_or(|required| required > upload_size) {
			return false;
		}
		let (source_row_pitch, source_image_pitch, required_source_bytes) =
			validated_texture_source_layout(row_bytes, row_count, source_row_pitch, source_image_pitch, depth);
		if required_source_bytes > source_bytes.len() {
			return false;
		}
		let (Some(upload), mapped, _) = self.create_buffer_resource(upload_size, DeviceAccesses::HostToDevice) else {
			return false;
		};
		if mapped.is_null() {
			return false;
		}

		// D3D12 reads only the texel bytes in each pitched row, so row padding does not need initialization.
		// SAFETY: `mapped` covers `upload_size`; checked pitches keep every copied row in that allocation and source slice.
		unsafe {
			for layer in 0..depth {
				for y in 0..row_count {
					let source_start = layer * source_image_pitch + y * source_row_pitch;
					let source_end = source_start + row_bytes;
					let upload_start = layer * upload_image_pitch + y * upload_row_pitch;
					debug_assert!(source_end <= source_bytes.len());
					debug_assert!(upload_start + row_bytes <= upload_size);
					std::ptr::copy_nonoverlapping(
						source_bytes[source_start..source_end].as_ptr(),
						mapped.add(upload_start),
						row_bytes,
					);
				}
			}
		}

		let mut source_location = D3D12_TEXTURE_COPY_LOCATION {
			pResource: std::mem::ManuallyDrop::new(Some(upload.clone())),
			Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
			Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
				PlacedFootprint: footprint.placed,
			},
		};
		let mut destination_location = D3D12_TEXTURE_COPY_LOCATION {
			pResource: std::mem::ManuallyDrop::new(Some(destination)),
			Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
			Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
				SubresourceIndex: destination_mip_level,
			},
		};

		self.transition_tracked_image(
			command_list,
			image_handle,
			destination_location.pResource.as_ref().unwrap(),
			TextureBarrierState::COPY_DESTINATION,
		);
		// SAFETY: Both retained resources and their driver-provided subresource footprints remain valid through submission.
		unsafe {
			command_list.CopyTextureRegion(&destination_location, 0, 0, 0, &source_location, None);
		}
		self.transition_tracked_image(
			command_list,
			image_handle,
			destination_location.pResource.as_ref().unwrap(),
			TextureBarrierState::COMMON,
		);
		// The copy call only borrows these descriptors. Release their COM clones while the separately retained resources stay alive.
		unsafe {
			std::mem::ManuallyDrop::drop(&mut source_location.pResource);
			std::mem::ManuallyDrop::drop(&mut destination_location.pResource);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.retain_command_buffer_upload_resource(command_buffer_handle, upload);
		true
	}
}
