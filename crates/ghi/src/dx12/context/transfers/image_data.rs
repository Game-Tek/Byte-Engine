use super::*;

impl Device {
	pub(crate) fn copy_image_to_cpu(&mut self, image_handle: ImageHandle) -> TextureCopyHandle {
		self.copy_image_to_cpu_for_sequence(image_handle, 0)
	}

	pub(crate) fn copy_image_to_cpu_for_sequence(
		&mut self,
		image_handle: ImageHandle,
		sequence_index: u8,
	) -> TextureCopyHandle {
		// Copies stored image data into a new staging buffer for CPU reads.
		let image = &self.images[image_handle.0 .0 as usize];
		let data = image
			.frame_data
			.as_ref()
			.and_then(|frames| frames.get(sequence_index as usize).or_else(|| frames.first()))
			.cloned()
			.or_else(|| image.data.clone())
			.unwrap_or_default();
		self.texture_copies.push(data);
		TextureCopyHandle((self.texture_copies.len() - 1) as u64)
	}

	pub(crate) fn record_image_readback(&mut self, command_buffer_handle: CommandBufferHandle, image_handle: ImageHandle) {
		self.record_image_readback_internal(command_buffer_handle, image_handle, None, 0);
	}

	pub(crate) fn record_image_readback_for_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		texture_copy: TextureCopyHandle,
		sequence_index: u8,
	) {
		self.record_image_readback_internal(command_buffer_handle, image_handle, Some(texture_copy), sequence_index);
	}

	pub(crate) fn record_image_readback_internal(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		texture_copy: Option<TextureCopyHandle>,
		sequence_index: u8,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let source = self.ensure_image_resource_for_sequence(image_handle.0, sequence_index);
		let Some(image) = self.images.get(image_handle.0 .0 as usize) else {
			return;
		};
		let (Some(source), Some(format), Some((row_bytes, row_count, _))) = (
			source,
			Self::dxgi_format(image.format),
			utils::texture_copy_layout(image.format, image.extent),
		) else {
			return;
		};

		let extent = image.extent;
		let depth = extent.depth().max(1) as usize;
		let readback_row_pitch = Self::align_up(row_bytes, 256);
		let readback_size = readback_row_pitch * row_count * depth;
		let (Some(readback), ..) = self.create_buffer_resource(readback_size, DeviceAccesses::DeviceToHost) else {
			return;
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
						Depth: depth as u32,
						RowPitch: readback_row_pitch as u32,
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
		if texture_copy.is_none() {
			self.retain_command_buffer_resource(command_buffer_handle, readback);
			return;
		}
		let texture_copy = texture_copy.expect(
			"Missing DX12 texture-copy handle. The most likely cause is that a retained readback was created without CPU copy storage.",
		);
		self.texture_readbacks.push(TextureReadback {
			command_buffer_handle,
			texture_copy,
			completion: None,
			resource: readback,
			sequence_index,
			row_pitch: readback_row_pitch,
			row_bytes,
			height: row_count,
			depth,
			size: readback_size,
			resolved: false,
		});
	}

	pub(crate) fn refresh_readback_texture_copies(&mut self, sequence_index: Option<u8>) {
		// Maps completed readback buffers and repacks DX12 row padding into compact texture copies.
		for readback in &mut self.texture_readbacks {
			if readback.resolved {
				continue;
			}
			if sequence_index.is_some_and(|sequence_index| readback.sequence_index != sequence_index) {
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
			if readback.size == 0 {
				continue;
			}

			let mut mapped: *mut std::ffi::c_void = std::ptr::null_mut();
			let read_range = D3D12_RANGE {
				Begin: 0,
				End: readback.size,
			};
			let result = unsafe { readback.resource.Map(0, Some(&read_range), Some(&mut mapped)) };
			if result.is_err() || mapped.is_null() {
				continue;
			}

			let compact_size = readback.row_bytes * readback.height * readback.depth;
			let mut compact = vec![0; compact_size];
			for layer in 0..readback.depth {
				for row in 0..readback.height {
					let source_offset = (layer * readback.height + row) * readback.row_pitch;
					let destination_offset = (layer * readback.height + row) * readback.row_bytes;
					unsafe {
						std::ptr::copy_nonoverlapping(
							(mapped as *const u8).add(source_offset),
							compact.as_mut_ptr().add(destination_offset),
							readback.row_bytes,
						);
					}
				}
			}
			let written_range = D3D12_RANGE { Begin: 0, End: 0 };
			unsafe {
				readback.resource.Unmap(0, Some(&written_range));
			}

			if let Some(texture_copy) = self.texture_copies.get_mut(readback.texture_copy.0 as usize) {
				*texture_copy = compact;
				self.texture_readback_resolve_count += 1;
				readback.resolved = true;
			}
		}
		// The compact CPU copy owns the result after resolution, so the native readback resource can retire now.
		self.texture_readbacks.retain(|readback| !readback.resolved);
	}

	pub(crate) fn write_image_data(&mut self, image_handle: ImageHandle, data: &[RGBAu8]) {
		self.write_image_data_for_sequence(image_handle, data, 0);
	}

	pub(crate) fn write_image_data_for_sequence(&mut self, image_handle: ImageHandle, data: &[RGBAu8], sequence_index: u8) {
		// Writes CPU-side image data for formats with staging storage.
		let image = &mut self.images[image_handle.0 .0 as usize];
		let staging = if let Some(frame_data) = image.frame_data.as_mut() {
			let index = (sequence_index as usize).min(frame_data.len().saturating_sub(1));
			frame_data.get_mut(index)
		} else {
			image.data.as_mut()
		};
		let Some(staging) = staging else {
			return;
		};

		let bytes =
			unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * std::mem::size_of::<RGBAu8>()) };
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
