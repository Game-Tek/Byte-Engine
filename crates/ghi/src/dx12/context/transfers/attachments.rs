use super::*;

impl Device {
	pub(crate) fn attachment_render_target_resource(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		attachment: &AttachmentInformation,
		sequence_index: u8,
	) -> Option<(Option<crate::BaseImageHandle>, ID3D12Resource, bool)> {
		match attachment.target {
			ImageOrSwapchain::Image(image_handle) => {
				let resource = self.ensure_image_resource_for_sequence(image_handle, sequence_index)?;
				Some((Some(image_handle), resource, false))
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
				Some((None, resource, true))
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

	pub(crate) fn attachment_image_handle(&mut self, attachment: &AttachmentInformation) -> crate::BaseImageHandle {
		match attachment.target {
			ImageOrSwapchain::Image(image) => image,
			// Every acquired backbuffer of a swapchain renders through its one proxy image.
			ImageOrSwapchain::Swapchain(swapchain) => self.get_swapchain_image(swapchain, Uses::RenderTarget).0.0,
		}
	}

	pub(crate) fn attachment_format(&self, attachment: &AttachmentInformation) -> Formats {
		match attachment.target {
			ImageOrSwapchain::Image(image) => {
				let image_format = self
					.images
					.get(image.index() as usize)
					.map(|image| image.format)
					.unwrap_or(Formats::RGBA8UNORM);
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

	/// Records one clear batch after staging all compatible UAV descriptors in contiguous runs.
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
			let Some(resource) = self.ensure_image_resource_for_sequence(image_handle, sequence_index) else {
				continue;
			};
			let Some(image) = self.images.get(image_handle.index() as usize) else {
				continue;
			};
			let Some(format) = Self::image_resource_flags(image.format, image.uses)
				.contains(D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS)
				.then(|| Self::dxgi_shader_resource_format(image.format))
				.flatten()
			else {
				continue;
			};
			let description = Self::texture_uav_desc(format, image.extent, image.is_3d, image.array_layers);
			self.prepare_clear_descriptor(command_buffer_handle, &resource, &description);
		}
		self.flush_pending_clear_descriptor_copies(command_buffer_handle);
		for &(image, clear) in images {
			self.record_image_clear_with_final_state(
				command_buffer_handle,
				ImageHandle(image),
				clear,
				sequence_index,
				None,
				true,
			);
		}
	}

	/// Records an image clear and optionally transitions directly to the caller's next use.
	pub(crate) fn record_image_clear_with_final_state(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		clear: crate::ClearValue,
		sequence_index: u8,
		final_state: Option<TextureBarrierState>,
		transition_before_clear: bool,
	) {
		let command_list = self.command_list(command_buffer_handle);
		let Some(destination) = self.ensure_image_resource_for_sequence(image_handle.0, sequence_index) else {
			return;
		};
		let Some(image) = self.images.get(image_handle.0.index() as usize) else {
			return;
		};
		let image_format = image.format;
		let extent = image.extent;
		let is_3d = image.is_3d;
		let clears_through_uav =
			Self::image_resource_flags(image_format, image.uses).contains(D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS);
		let array_layers = image.array_layers;
		let Some(format) = clears_through_uav
			.then(|| Self::dxgi_shader_resource_format(image_format))
			.flatten()
		else {
			self.record_image_clear_upload_fallback(
				command_buffer_handle,
				&command_list,
				image_handle.0,
				destination.clone(),
				image_format,
				extent,
				clear,
				sequence_index,
			);
			if let Some(final_state) = final_state {
				self.transition_tracked_image(&command_list, image_handle.0, &destination, final_state);
			}
			return;
		};
		let desc = Self::texture_uav_desc(format, extent, is_3d, array_layers);
		if !self
			.command_buffers
			.get(command_buffer_handle.index() as usize)
			.is_some_and(|command_buffer| {
				let resource = Self::native_resource_key(&destination);
				command_buffer
					.prepared_clear_descriptors
					.iter()
					.any(|descriptor| descriptor.resource == resource)
			}) {
			self.prepare_clear_descriptor(command_buffer_handle, &destination, &desc);
			self.flush_pending_clear_descriptor_copies(command_buffer_handle);
		}
		let Some(descriptor) = self.take_prepared_clear_descriptor(command_buffer_handle, &destination) else {
			self.record_image_clear_upload_fallback(
				command_buffer_handle,
				&command_list,
				image_handle.0,
				destination.clone(),
				image_format,
				extent,
				clear,
				sequence_index,
			);
			if let Some(final_state) = final_state {
				self.transition_tracked_image(&command_list, image_handle.0, &destination, final_state);
			}
			return;
		};

		if transition_before_clear {
			self.transition_tracked_image(
				&command_list,
				image_handle.0,
				&destination,
				TextureBarrierState::unordered_access(D3D12_BARRIER_SYNC_CLEAR_UNORDERED_ACCESS_VIEW),
			);
		}
		unsafe {
			match clear {
				crate::ClearValue::Integer(r, g, b, a) => {
					command_list.ClearUnorderedAccessViewUint(descriptor.gpu, descriptor.cpu, &destination, &[r, g, b, a], &[]);
				}
				crate::ClearValue::Color(color) => {
					command_list.ClearUnorderedAccessViewFloat(
						descriptor.gpu,
						descriptor.cpu,
						&destination,
						&[color.r, color.g, color.b, color.a],
						&[],
					);
				}
				crate::ClearValue::None => {
					command_list.ClearUnorderedAccessViewFloat(
						descriptor.gpu,
						descriptor.cpu,
						&destination,
						&[0.0, 0.0, 0.0, 0.0],
						&[],
					);
				}
				crate::ClearValue::Depth(_) => {}
			}
		}
		if let Some(final_state) = final_state {
			// The transition orders the UAV clear and makes a separate UAV barrier redundant.
			self.transition_tracked_image(&command_list, image_handle.0, &destination, final_state);
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
		_sequence_index: u8,
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
