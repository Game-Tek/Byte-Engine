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
	/// Records copies from buffers into images on the GPU, reading each source where the caller staged it.
	pub(crate) fn copy_buffer_to_images(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copies: &[crate::BufferImageCopyDescriptor],
		sequence_index: u8,
	) {
		for copy in copies {
			self.record_buffer_to_image_copy(command_buffer_handle, copy, sequence_index);
		}
	}

	/// Records one buffer-to-image copy: one `CopyTextureRegion` for each array layer, reading a placed footprint at the
	/// source's offset and pitches.
	///
	/// The device's unrestricted copy pitches let the footprint start at any offset the GHI's staging alignment
	/// produces; see `require_modern_dx12_capabilities`.
	pub(crate) fn record_buffer_to_image_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		copy: &crate::BufferImageCopyDescriptor,
		sequence_index: u8,
	) {
		let command_list = self.command_list(command_buffer_handle);
		let destination = self
			.ensure_image_resource_for_sequence(copy.destination_image, sequence_index)
			.expect("DX12 image copy destination has no native texture. The most likely cause is an image-group member whose group isn't placed yet. Place the group before copying into its members.");
		let source = self
			.buffer_resource_for_sequence(copy.source_buffer, sequence_index)
			.expect("DX12 image copy source has no native buffer. The most likely cause is an empty or stale source buffer. Copy from a buffer with the staged bytes.");
		let source_size = self.buffer(copy.source_buffer).size;
		let image = &self.images[copy.destination_image.index() as usize];
		assert!(
			copy.destination_mip_level < image.mip_levels,
			"DX12 image copy names mip {} of an image with {} mips. The most likely cause is upload metadata that doesn't match the image. Copy into a mip the image has.",
			copy.destination_mip_level,
			image.mip_levels
		);
		let (format, array_layers, mip_levels) = (image.format, image.array_layers.max(1), image.mip_levels);
		let mip_extent = crate::image::mip_extent(image.extent, copy.destination_mip_level);
		if let Some(region) = copy.destination_region {
			region.validate(mip_extent, format, array_layers);
		}
		let copy_extent = copy
			.destination_region
			.map_or(mip_extent, |region| Extent::rectangle(region.size[0], region.size[1]));
		let origin = copy.destination_region.map_or([0, 0], |region| region.offset);
		let Some((row_bytes, row_count, _)) = utils::texture_copy_layout(format, copy_extent) else {
			panic!(
				"DX12 image copy has no copy layout for {format:?}. The most likely cause is a format without a buffer layout, such as a depth format. Fill such images by rendering to them."
			);
		};
		let depth = copy_extent.depth().max(1) as usize;
		let (row_pitch, image_pitch, required_bytes) = validated_texture_source_layout(
			row_bytes,
			row_count,
			copy.source_bytes_per_row,
			copy.source_bytes_per_image,
			depth * array_layers as usize,
		);
		assert!(
			copy.source_offset + required_bytes <= source_size,
			"DX12 image copy reads {required_bytes} bytes at offset {} of a {source_size}-byte buffer. The most likely cause is staging sized for another extent or layer count. Size the staging with ImageCopyLayout::padded_size.",
			copy.source_offset
		);

		self.transition_tracked_buffer(&command_list, &source, BufferBarrierState::COPY_SOURCE);
		self.transition_tracked_image(
			&command_list,
			copy.destination_image,
			&destination,
			TextureBarrierState::COPY_DESTINATION,
		);
		for layer in 0..array_layers {
			let subresource = layer * mip_levels + copy.destination_mip_level;
			let mut footprint = self
				.native_texture_copy_footprint(&destination, subresource)
				.expect("DX12 image copy has no copyable footprint. The most likely cause is a subresource the device can't copy into. Check the image's format and extent.")
				.placed;
			// The native footprint names the format and the whole mip's block-aligned size; the copy reads the caller's
			// rows, and only the region's size when it updates part of the mip.
			footprint.Offset = (copy.source_offset + layer as usize * image_pitch * depth) as u64;
			footprint.Footprint.RowPitch = row_pitch as u32;
			if copy.destination_region.is_some() {
				footprint.Footprint.Width = copy_extent.width();
				footprint.Footprint.Height = copy_extent.height();
			}
			let mut source_location = D3D12_TEXTURE_COPY_LOCATION {
				pResource: std::mem::ManuallyDrop::new(Some(source.clone())),
				Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
				Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
					PlacedFootprint: footprint,
				},
			};
			let mut destination_location = D3D12_TEXTURE_COPY_LOCATION {
				pResource: std::mem::ManuallyDrop::new(Some(destination.clone())),
				Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
				Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
					SubresourceIndex: subresource,
				},
			};
			// SAFETY: Both resources stay retained by the command buffer through submission, and the checks above keep
			// the footprint inside the source buffer.
			unsafe {
				command_list.CopyTextureRegion(&destination_location, origin[0], origin[1], 0, &source_location, None);
				// The copy only borrows the locations; release their COM clones.
				std::mem::ManuallyDrop::drop(&mut source_location.pResource);
				std::mem::ManuallyDrop::drop(&mut destination_location.pResource);
			}
		}
		self.transition_tracked_image(
			&command_list,
			copy.destination_image,
			&destination,
			TextureBarrierState::COMMON,
		);
		self.transition_tracked_buffer(&command_list, &source, BufferBarrierState::COMMON);
		self.mark_command_buffer_work(command_buffer_handle);
	}

	pub(crate) fn begin_debug_region(&mut self, command_buffer_handle: CommandBufferHandle, name: &str) {
		if !self.settings.debug_labels {
			return;
		}

		let command_list = self.command_list(command_buffer_handle);

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
	}

	pub(crate) fn end_debug_region(&mut self, command_buffer_handle: CommandBufferHandle) {
		if !self.settings.debug_labels {
			return;
		}

		let command_list = self.command_list(command_buffer_handle);

		unsafe {
			command_list.EndEvent();
		}
	}
}
