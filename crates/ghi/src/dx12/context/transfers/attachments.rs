use super::*;

impl Device {
	pub(crate) fn attachment_render_target_resource(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		attachment: &AttachmentInformation,
		sequence_index: u8,
	) -> Option<(Option<crate::BaseImageHandle>, ID3D12Resource)> {
		match attachment.target {
			ImageOrSwapchain::Image(image_handle) => {
				let resource = self.ensure_image_resource_for_sequence(image_handle, sequence_index)?;
				Some((Some(image_handle), resource))
			}
			ImageOrSwapchain::Swapchain(swapchain_handle) => {
				let resource = self.swapchain_backbuffer_resource(swapchain_handle, sequence_index)?;
				let resource_key = Self::native_resource_key(&resource);
				let present_resources = &mut self.command_buffers[command_buffer_handle.index() as usize].present_resources;
				// Multiple render passes can target one backbuffer, but it needs only one terminal PRESENT transition.
				if present_resources
					.iter()
					.all(|scheduled| Self::native_resource_key(scheduled) != resource_key)
				{
					present_resources.push(resource.clone());
				}
				Some((None, resource))
			}
		}
	}

	pub(crate) fn swapchain_backbuffer_resource(
		&mut self,
		swapchain_handle: SwapchainHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		let (resource, newly_cached) = {
			let swapchain = self.swapchains.get_mut(swapchain_handle.index() as usize)?;
			let sequence_index = usize::from(sequence_index);
			assert!(
				swapchain.acquired_sequences[sequence_index],
				"DX12 swapchain backbuffer was used without an acquisition. The most likely cause is that command recording omitted acquire_swapchain_image for this frame sequence."
			);
			let image_index = swapchain.acquired_image_indices[sequence_index] as usize;
			let current_image_index = unsafe { swapchain.swapchain.GetCurrentBackBufferIndex() } as usize;
			assert_eq!(
				image_index, current_image_index,
				"DX12 acquired backbuffer is no longer current. The most likely cause is that the swapchain advanced outside its owning frame execution."
			);
			let image_index = image_index.min(swapchain.image_count.saturating_sub(1) as usize);
			let mut newly_cached = false;
			if swapchain.backbuffers[image_index].is_none() {
				let resource = unsafe { swapchain.swapchain.GetBuffer::<ID3D12Resource>(image_index as u32) }.ok()?;
				swapchain.backbuffers[image_index] = Some(resource);
				newly_cached = true;
			}
			(swapchain.backbuffers[image_index].clone()?, newly_cached)
		};
		if newly_cached {
			self.image_states
				.insert(Self::native_resource_key(&resource), TextureBarrierState::PRESENT);
		}
		Some(resource)
	}

	pub(crate) fn attachment_format(&self, attachment: &AttachmentInformation) -> Formats {
		match attachment.target {
			ImageOrSwapchain::Image(image) => {
				let image_format = self.images[image.index() as usize].format;
				if let Some(format) = attachment.format {
					assert_eq!(
						format, image_format,
						"Unsupported DX12 attachment format reinterpretation. The most likely cause is that a typed image was bound with an attachment format different from its allocation."
					);
				}
				image_format
			}
			ImageOrSwapchain::Swapchain(_) => {
				let format = attachment.format.unwrap_or(Formats::BGRAsRGB);
				assert!(
					matches!(format, Formats::BGRAu8 | Formats::BGRAsRGB),
					"Unsupported DX12 swapchain attachment format. The most likely cause is that the backbuffer view is not BGRA8 UNORM or BGRA8 sRGB. See https://learn.microsoft.com/en-us/windows-hardware/drivers/display/fully-typed-back-buffers-casting."
				);
				format
			}
		}
	}

	/// Clears each image to its value, after giving image-group members their memory.
	pub(crate) fn clear_images(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		images: &[(crate::BaseImageHandle, crate::ClearValue)],
		sequence_index: u8,
	) {
		for &(image_handle, _) in images {
			if self.image_groups.member(image_handle).is_some() {
				self.initialize_image_contents(image_handle, sequence_index);
			}
		}
		for &(image, clear) in images {
			self.record_image_clear(command_buffer_handle, image, clear, sequence_index, true);
		}
	}

	/// Records an image clear: through an unordered-access view when the image allows one, transitioning it first
	/// when `transition_before_clear` is set, and otherwise by copying the clear color from upload memory.
	pub(crate) fn record_image_clear(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: crate::BaseImageHandle,
		clear: crate::ClearValue,
		sequence_index: u8,
		transition_before_clear: bool,
	) {
		let command_list = self.command_list(command_buffer_handle);
		let Some(destination) = self.ensure_image_resource_for_sequence(image_handle, sequence_index) else {
			return;
		};
		let image = &self.images[image_handle.index() as usize];
		let (image_format, extent, is_3d, array_layers) = (image.format, image.extent, image.is_3d, image.array_layers);
		let uav_format = Self::image_resource_flags(image_format, image.uses)
			.contains(D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS)
			.then(|| Self::dxgi_shader_resource_format(image_format))
			.flatten();
		let Some(uav_format) = uav_format else {
			self.record_image_clear_upload_fallback(
				command_buffer_handle,
				&command_list,
				image_handle,
				destination,
				image_format,
				extent,
				clear,
			);
			return;
		};
		let description = Self::image_clear_uav_desc(uav_format, extent, is_3d, array_layers);
		let (gpu, cpu) = self.clear_uav_descriptors(command_buffer_handle, &destination, &description);

		if transition_before_clear {
			self.transition_tracked_image(
				&command_list,
				image_handle,
				&destination,
				TextureBarrierState::unordered_access(D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW),
			);
		}
		unsafe {
			match clear {
				crate::ClearValue::Integer(r, g, b, a) => {
					command_list.ClearUnorderedAccessViewUint(gpu, cpu, &destination, &[r, g, b, a], &[]);
				}
				crate::ClearValue::Depth(_) => {}
				_ => command_list.ClearUnorderedAccessViewFloat(gpu, cpu, &destination, &Self::clear_color_f32(clear), &[]),
			}
		}
		self.mark_command_buffer_work(command_buffer_handle);
	}

	/// Clears the first mip of a texture that can't be cleared through an unordered-access view by copying the clear
	/// color from a temporary upload buffer, which the command buffer keeps until it completes.
	pub(crate) fn record_image_clear_upload_fallback(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList7,
		image_handle: crate::BaseImageHandle,
		destination: ID3D12Resource,
		format: Formats,
		extent: Extent,
		clear: crate::ClearValue,
	) {
		let (Some(_), Some(bytes_per_pixel)) = (Self::dxgi_format(format), utils::bytes_per_pixel(format)) else {
			return;
		};
		if bytes_per_pixel != std::mem::size_of::<RGBAu8>() {
			return;
		}
		let Some(footprint) = self.native_texture_copy_footprint(&destination, 0) else {
			return;
		};
		let (Some(upload), mapped, _) = self.create_buffer_resource(footprint.total_size, crate::HostAccess::Write) else {
			return;
		};
		let color = Self::clear_color_bytes(clear);
		let row_pitch = footprint.placed.Footprint.RowPitch as usize;
		let row_count = footprint.row_count * extent.depth().max(1) as usize;
		// SAFETY: The mapping covers the footprint's total size, which holds `row_count` rows of `row_pitch` bytes, each
		// starting with the row's `width` texels.
		unsafe {
			for row in 0..row_count {
				let row_start = mapped.add(row * row_pitch);
				for texel in 0..extent.width() as usize {
					std::ptr::copy_nonoverlapping(color.as_ptr(), row_start.add(texel * bytes_per_pixel), bytes_per_pixel);
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
			pResource: std::mem::ManuallyDrop::new(Some(destination.clone())),
			Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
			Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
		};
		self.transition_tracked_image(
			command_list,
			image_handle,
			&destination,
			TextureBarrierState::COPY_DESTINATION,
		);
		// SAFETY: Both resources stay retained through submission, and the footprint describes the upload's layout.
		unsafe {
			command_list.CopyTextureRegion(&destination_location, 0, 0, 0, &source_location, None);
			std::mem::ManuallyDrop::drop(&mut source_location.pResource);
			std::mem::ManuallyDrop::drop(&mut destination_location.pResource);
		}
		self.transition_tracked_image(command_list, image_handle, &destination, TextureBarrierState::COMMON);
		self.mark_command_buffer_work(command_buffer_handle);
		self.retain_command_buffer_resource(command_buffer_handle, &upload);
	}
}
