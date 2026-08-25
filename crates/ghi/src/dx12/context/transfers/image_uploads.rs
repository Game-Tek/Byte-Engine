use super::*;

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
		let row_stride = if copy.source_bytes_per_row == 0 {
			row_bytes
		} else {
			copy.source_bytes_per_row
		};
		let image_stride = if copy.source_bytes_per_image == 0 {
			row_stride * row_count
		} else {
			copy.source_bytes_per_image
		};
		let depth = extent.depth().max(1) as usize;
		let source_bytes =
			self.buffer_range_for_sequence(copy.source_buffer, copy.source_offset, image_stride * depth, sequence_index);
		let Some(destination) = self.image_data_mut_for_sequence(copy.destination_image, sequence_index) else {
			return;
		};

		for layer in 0..depth {
			for y in 0..row_count {
				let source_start = layer * image_stride + y * row_stride;
				let source_end = source_start + row_bytes;
				let destination_start = layer * compact_bytes_per_image + y * row_bytes;
				let destination_end = destination_start + row_bytes;
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
		let (Some(destination), Some(format), Some((row_bytes, row_count, _))) = (
			destination,
			Self::dxgi_format(image.format),
			utils::texture_copy_layout(image.format, extent),
		) else {
			return;
		};

		let source_row_pitch = if copy.source_bytes_per_row == 0 {
			row_bytes
		} else {
			copy.source_bytes_per_row
		};
		let source_image_pitch = if copy.source_bytes_per_image == 0 {
			source_row_pitch * row_count
		} else {
			copy.source_bytes_per_image
		};
		let array_layers = image.array_layers.max(1) as usize;
		let mip_levels = image.mip_levels;
		let source_bytes = self.buffer_range_for_sequence(
			copy.source_buffer,
			copy.source_offset,
			source_image_pitch * extent.depth().max(1) as usize * array_layers,
			sequence_index,
		);
		for layer in 0..array_layers {
			let start = layer * source_image_pitch;
			let end = start + source_image_pitch;
			self.record_image_upload(
				command_buffer_handle,
				&command_list,
				copy.destination_image,
				destination.clone(),
				format,
				extent,
				&source_bytes[start..end],
				source_row_pitch,
				source_image_pitch,
				copy.destination_mip_level + layer as u32 * mip_levels,
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
		let (Some(destination), Some(format), Some((source_row_pitch, ..))) = (
			destination,
			Self::dxgi_format(image.format),
			utils::texture_copy_layout(image.format, image.extent),
		) else {
			return;
		};
		let extent = image.extent;
		let source_bytes =
			unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * std::mem::size_of::<RGBAu8>()) };
		if self.record_image_upload(
			command_buffer_handle,
			&command_list,
			image_handle.0,
			destination,
			format,
			extent,
			source_bytes,
			source_row_pitch,
			source_row_pitch
				* utils::texture_copy_layout(image.format, image.extent)
					.map(|(_, rows, _)| rows)
					.unwrap_or(0),
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
		let pending = std::mem::take(&mut self.pending_texture_syncs);
		for (image_handle, sequence_index) in pending {
			let image_mismatch = image_filter.is_some_and(|filter| filter != image_handle);
			let sequence_mismatch = sequence_filter.is_some_and(|filter| filter != sequence_index);
			if image_mismatch || sequence_mismatch {
				self.pending_texture_syncs.push((image_handle, sequence_index));
				continue;
			}
			self.record_image_storage_upload(command_buffer_handle, ImageHandle(image_handle), sequence_index);
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
		let (Some(destination), Some(format), Some((source_row_pitch, ..))) = (
			destination,
			Self::dxgi_format(image.format),
			utils::texture_copy_layout(image.format, image.extent),
		) else {
			return;
		};
		let extent = image.extent;
		let source_bytes = image
			.frame_data
			.as_ref()
			.and_then(|frames| frames.get(sequence_index as usize).or_else(|| frames.first()))
			.cloned()
			.or_else(|| image.data.clone())
			.unwrap_or_default();
		if self.record_image_upload(
			command_buffer_handle,
			&command_list,
			image_handle.0,
			destination,
			format,
			extent,
			&source_bytes,
			source_row_pitch,
			source_row_pitch
				* utils::texture_copy_layout(image.format, image.extent)
					.map(|(_, rows, _)| rows)
					.unwrap_or(0),
			0,
		) {
			self.gpu_uploaded_images.insert(image_handle.0);
		}
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
		command_list: &ID3D12GraphicsCommandList,
		image_handle: crate::BaseImageHandle,
		destination: ID3D12Resource,
		format: DXGI_FORMAT,
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
		let depth = extent.depth().max(1) as usize;
		let upload_row_pitch = Self::align_up(row_bytes, 256);
		let upload_size = upload_row_pitch * row_count * depth;
		let (Some(upload), mapped, _) = self.create_buffer_resource(upload_size, DeviceAccesses::HostToDevice) else {
			return false;
		};
		if mapped.is_null() {
			return false;
		}

		unsafe {
			std::ptr::write_bytes(mapped, 0, upload_size);
			for layer in 0..depth {
				for y in 0..row_count {
					let source_start = layer * source_image_pitch + y * source_row_pitch;
					let source_end = source_start + row_bytes;
					let upload_start = (layer * row_count + y) * upload_row_pitch;
					if source_end > source_bytes.len() {
						return false;
					}
					std::ptr::copy_nonoverlapping(
						source_bytes[source_start..source_end].as_ptr(),
						mapped.add(upload_start),
						row_bytes,
					);
				}
			}
		}

		let source_location = D3D12_TEXTURE_COPY_LOCATION {
			pResource: std::mem::ManuallyDrop::new(Some(upload.clone())),
			Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
			Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
				PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
					Offset: 0,
					Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
						Format: format,
						Width: extent.width(),
						Height: extent.height(),
						Depth: depth as u32,
						RowPitch: upload_row_pitch as u32,
					},
				},
			},
		};
		let destination_location = D3D12_TEXTURE_COPY_LOCATION {
			pResource: std::mem::ManuallyDrop::new(Some(destination)),
			Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
			Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
				SubresourceIndex: destination_mip_level,
			},
		};

		unsafe {
			self.transition_tracked_image(
				command_list,
				image_handle,
				destination_location.pResource.as_ref().unwrap(),
				D3D12_RESOURCE_STATE_COPY_DEST,
			);
			command_list.CopyTextureRegion(&destination_location, 0, 0, 0, &source_location, None);
			self.transition_tracked_image(
				command_list,
				image_handle,
				destination_location.pResource.as_ref().unwrap(),
				D3D12_RESOURCE_STATE_COMMON,
			);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		self.retain_command_buffer_upload_resource(command_buffer_handle, upload);
		true
	}
}
