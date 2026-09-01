use super::*;

impl Device {
	/// Validates metadata that must be true before DX12 can record a regular-image readback.
	pub(crate) fn validate_texture_transfer_source(
		&self,
		image_handle: ImageHandle,
	) -> Result<(), crate::TextureTransferError> {
		let image = self
			.images
			.get(image_handle.0.0 as usize)
			.ok_or(crate::TextureTransferError::InvalidSource)?;
		crate::context::texture_transfer_layout(image.format, image.extent, image.array_layers, image.uses)?;
		Self::dxgi_format(image.format)
			.map(|_| ())
			.ok_or(crate::TextureTransferError::UnsupportedFormat(image.format))
	}

	pub(crate) fn abandon_texture_readback(&mut self, handle: TextureCopyHandle) {
		self.texture_readbacks.abandon_recorded(handle);
	}

	pub(crate) fn record_image_readback(&mut self, command_buffer_handle: CommandBufferHandle, image_handle: ImageHandle) {
		let _ = self.record_image_readback_internal(command_buffer_handle, image_handle, false, 0);
	}

	pub(crate) fn record_image_readback_for_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		sequence_index: u8,
	) -> Result<TextureCopyHandle, crate::TextureTransferError> {
		let readback = self
			.record_image_readback_internal(command_buffer_handle, image_handle, true, sequence_index)?
			.ok_or(crate::TextureTransferError::MappingFailed)?;
		Ok(self.texture_readbacks.insert(readback))
	}

	/// Records one base-mip copy and returns native staging only when the result will be mapped later.
	fn record_image_readback_internal(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		return_readback: bool,
		sequence_index: u8,
	) -> Result<Option<TextureReadback>, crate::TextureTransferError> {
		let command_list = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
			.ok_or(crate::TextureTransferError::MappingFailed)?;
		let image = self
			.images
			.get(image_handle.0.0 as usize)
			.ok_or(crate::TextureTransferError::InvalidSource)?;
		let layout = crate::context::texture_transfer_layout(image.format, image.extent, image.array_layers, image.uses)?;
		let extent = image.extent;
		let image_format = image.format;
		let format = Self::dxgi_format(image_format).ok_or(crate::TextureTransferError::UnsupportedFormat(image_format))?;
		let source = self
			.ensure_image_resource_for_sequence(image_handle.0, sequence_index)
			.ok_or(crate::TextureTransferError::MappingFailed)?;
		let readback_row_pitch = layout
			.bytes_per_row
			.checked_add(255)
			.map(|bytes| bytes & !255)
			.ok_or(crate::TextureTransferError::UnsupportedLayout)?;
		let readback_size = readback_row_pitch
			.checked_mul(layout.row_count)
			.ok_or(crate::TextureTransferError::UnsupportedLayout)?;
		let (Some(readback), ..) = self.create_buffer_resource(readback_size, DeviceAccesses::DeviceToHost) else {
			return Err(crate::TextureTransferError::AllocationFailed);
		};

		let source_location = D3D12_TEXTURE_COPY_LOCATION {
			pResource: std::mem::ManuallyDrop::new(Some(source)),
			Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
			Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
		};
		let destination_location = D3D12_TEXTURE_COPY_LOCATION {
			pResource: std::mem::ManuallyDrop::new(Some(readback.clone())),
			Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
			Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
				PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
					Offset: 0,
					Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
						Format: format,
						Width: extent.width(),
						Height: extent.height(),
						Depth: 1,
						RowPitch: u32::try_from(readback_row_pitch)
							.map_err(|_| crate::TextureTransferError::UnsupportedLayout)?,
					},
				},
			},
		};

		unsafe {
			self.transition_tracked_image(
				&command_list,
				image_handle.0,
				source_location.pResource.as_ref().unwrap(),
				D3D12_RESOURCE_STATE_COPY_SOURCE,
			);
			command_list.CopyTextureRegion(&destination_location, 0, 0, 0, &source_location, None);
			self.transition_tracked_image(
				&command_list,
				image_handle.0,
				source_location.pResource.as_ref().unwrap(),
				D3D12_RESOURCE_STATE_COMMON,
			);
		}
		self.mark_command_buffer_work(command_buffer_handle);
		if !return_readback {
			self.retain_command_buffer_resource(command_buffer_handle, readback);
			return Ok(None);
		}

		Ok(Some(TextureReadback {
			command_buffer_handle: Some(command_buffer_handle),
			completion: None,
			resource: Some(readback),
			sequence_index,
			row_pitch: readback_row_pitch,
			row_bytes: layout.bytes_per_row,
			height: layout.row_count,
			depth: 1,
			size: readback_size,
			mapping_failed: false,
			data: TextureReadbackData {
				bytes: Vec::new(),
				extent,
				format: image_format,
				bytes_per_row: layout.bytes_per_row,
				bytes_per_image: layout.bytes_per_image,
			},
		}))
	}

	/// Releases every unsubmitted readback associated with one discarded command-list recording.
	pub(crate) fn abandon_texture_readbacks_for_command_buffer(&mut self, command_buffer_handle: CommandBufferHandle) {
		let handles = self
			.texture_readbacks
			.entries()
			.filter_map(|(handle, readback)| (readback.command_buffer_handle == Some(command_buffer_handle)).then_some(handle))
			.collect::<SmallVec<[_; 4]>>();
		for handle in handles {
			self.texture_readbacks.abandon_recorded(handle);
		}
	}

	pub(crate) fn refresh_readback_texture_copies(&mut self, sequence_index: Option<u8>) {
		// Maps completed readback buffers and repacks DX12 row padding into compact owned bytes.
		for readback in self.texture_readbacks.values_mut() {
			if readback.resource.is_none()
				|| sequence_index.is_some_and(|sequence_index| readback.sequence_index != sequence_index)
			{
				continue;
			}
			let Some((synchronizer_handle, completion_value)) = readback.completion else {
				continue;
			};
			let Some(synchronizer) = self.synchronizers.get(synchronizer_handle.0 as usize) else {
				continue;
			};
			if unsafe { synchronizer.fence.GetCompletedValue() } < completion_value {
				continue;
			}
			let Some(resource) = readback.resource.as_ref() else {
				continue;
			};

			let mut mapped: *mut std::ffi::c_void = std::ptr::null_mut();
			let read_range = D3D12_RANGE {
				Begin: 0,
				End: readback.size,
			};
			if unsafe { resource.Map(0, Some(&read_range), Some(&mut mapped)) }.is_err() || mapped.is_null() {
				readback.mapping_failed = true;
				readback.resource = None;
				continue;
			}

			let compact_size = readback.data.bytes_per_image;
			let mut compact = Vec::new();
			if compact.try_reserve_exact(compact_size).is_err() {
				unsafe { resource.Unmap(0, Some(&D3D12_RANGE { Begin: 0, End: 0 })) };
				readback.mapping_failed = true;
				readback.resource = None;
				continue;
			}
			compact.resize(compact_size, 0);
			for row in 0..readback.height {
				let source_offset = row * readback.row_pitch;
				let destination_offset = row * readback.row_bytes;
				unsafe {
					std::ptr::copy_nonoverlapping(
						(mapped as *const u8).add(source_offset),
						compact.as_mut_ptr().add(destination_offset),
						readback.row_bytes,
					);
				}
			}
			unsafe { resource.Unmap(0, Some(&D3D12_RANGE { Begin: 0, End: 0 })) };

			readback.data.bytes = compact;
			readback.resource = None;
			self.texture_readback_resolve_count += 1;
		}
	}

	pub(crate) fn write_image_data(&mut self, image_handle: ImageHandle, data: &[RGBAu8]) {
		self.write_image_data_for_sequence(image_handle, data, 0);
	}

	pub(crate) fn write_image_data_for_sequence(&mut self, image_handle: ImageHandle, data: &[RGBAu8], sequence_index: u8) {
		// Writes CPU-side image data for formats with staging storage.
		let image = &mut self.images[image_handle.0.0 as usize];
		let staging = if let Some(frame_data) = image.frame_data.as_mut() {
			let index = (sequence_index as usize).min(frame_data.len().saturating_sub(1));
			frame_data.get_mut(index)
		} else {
			image.data.as_mut()
		};
		let Some(staging) = staging else {
			return;
		};

		let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) };
		let length = staging.len().min(bytes.len());
		staging[..length].copy_from_slice(&bytes[..length]);
	}

	pub(crate) fn clear_image(&mut self, image_handle: crate::BaseImageHandle, clear: crate::ClearValue) {
		self.clear_image_for_sequence(image_handle, clear, 0);
	}

	/// Updates CPU-side image data for a frame sequence so readback-oriented images preserve clear values.
	pub(crate) fn clear_image_for_sequence(
		&mut self,
		image_handle: crate::BaseImageHandle,
		clear: crate::ClearValue,
		sequence_index: u8,
	) {
		let Some(image) = self.images.get_mut(image_handle.0 as usize) else {
			return;
		};
		let staging = if let Some(frame_data) = image.frame_data.as_mut() {
			let index = (sequence_index as usize).min(frame_data.len().saturating_sub(1));
			frame_data.get_mut(index)
		} else {
			image.data.as_mut()
		};
		let Some(staging) = staging else {
			return;
		};

		let color = Self::clear_color_bytes(clear);

		for pixel in staging.chunks_exact_mut(std::mem::size_of::<RGBAu8>()) {
			pixel.copy_from_slice(&color);
		}
	}

	pub(crate) fn clear_color_bytes(clear: crate::ClearValue) -> [u8; 4] {
		match clear {
			crate::ClearValue::None => [0, 0, 0, 0],
			crate::ClearValue::Color(color) => [
				(color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
				(color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
				(color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
				(color.a.clamp(0.0, 1.0) * 255.0).round() as u8,
			],
			crate::ClearValue::Integer(r, g, b, a) => [
				r.min(u8::MAX as u32) as u8,
				g.min(u8::MAX as u32) as u8,
				b.min(u8::MAX as u32) as u8,
				a.min(u8::MAX as u32) as u8,
			],
			crate::ClearValue::Depth(_) => [0, 0, 0, 0],
		}
	}

	pub(crate) fn clear_color_f32(clear: ClearValue) -> [f32; 4] {
		match clear {
			ClearValue::None => [0.0, 0.0, 0.0, 0.0],
			ClearValue::Color(color) => [color.r, color.g, color.b, color.a],
			ClearValue::Integer(r, g, b, a) => [
				(r.min(u8::MAX as u32) as f32) / 255.0,
				(g.min(u8::MAX as u32) as f32) / 255.0,
				(b.min(u8::MAX as u32) as f32) / 255.0,
				(a.min(u8::MAX as u32) as f32) / 255.0,
			],
			ClearValue::Depth(_) => [0.0, 0.0, 0.0, 0.0],
		}
	}

	pub(crate) fn clear_depth_value(clear: ClearValue) -> f32 {
		match clear {
			ClearValue::Depth(depth) => depth,
			_ => 1.0,
		}
	}
}
