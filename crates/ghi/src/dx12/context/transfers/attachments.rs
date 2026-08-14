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
				self.present_transitions
					.entry(command_buffer_handle)
					.or_default()
					.push(resource.clone());
				Some((None, resource, true))
			}
		}
	}

	pub(crate) fn swapchain_backbuffer_resource(
		&mut self,
		swapchain_handle: SwapchainHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		let resource = {
			let swapchain = self.swapchains.get_mut(swapchain_handle.0 as usize)?;
			let image_index = swapchain.acquired_image_indices[sequence_index as usize] as usize;
			let image_index = image_index.min(swapchain.image_count.saturating_sub(1) as usize);
			if swapchain.backbuffers[image_index].is_none() {
				let resource = unsafe { swapchain.swapchain.GetBuffer::<ID3D12Resource>(image_index as u32) }.ok()?;
				swapchain.backbuffers[image_index] = Some(resource);
			}
			swapchain.backbuffers[image_index].clone()?
		};
		self.materialize_render_target_views(&resource, Formats::BGRAu8, 1);
		Some(resource)
	}

	pub(crate) fn attachment_image_handle(
		&mut self,
		attachment: &AttachmentInformation,
		sequence_index: u8,
	) -> crate::BaseImageHandle {
		match attachment.target {
			ImageOrSwapchain::Image(image) => image,
			ImageOrSwapchain::Swapchain(swapchain) => {
				let image_index =
					self.swapchains[swapchain.0 as usize].acquired_image_indices[sequence_index as usize] as usize;
				self.get_swapchain_image(swapchain, Uses::RenderTarget);
				self.swapchains[swapchain.0 as usize].images[image_index]
					.unwrap_or_else(|| self.swapchains[swapchain.0 as usize].images[0].expect(
						"Missing DX12 swapchain proxy image. The most likely cause is that swapchain image access did not create the proxy image.",
					))
					.0
			}
		}
	}

	pub(crate) fn attachment_format(&self, attachment: &AttachmentInformation) -> Formats {
		match attachment.target {
			ImageOrSwapchain::Image(image) => self
				.images
				.get(image.0 as usize)
				.map(|image| image.format)
				.unwrap_or(Formats::RGBA8UNORM),
			ImageOrSwapchain::Swapchain(_) => Formats::BGRAu8,
		}
	}

	/// Records a DX12 image clear without allocating a full-size upload buffer when the image supports UAV clears.
	pub(crate) fn record_image_clear(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		image_handle: ImageHandle,
		clear: crate::ClearValue,
		sequence_index: u8,
	) {
		self.record_image_clear_with_final_state(command_buffer_handle, image_handle, clear, sequence_index, None, true);
	}

	/// Records one clear batch after staging all compatible UAV descriptors in contiguous runs.
	pub(crate) fn clear_images(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		images: &[(crate::BaseImageHandle, crate::ClearValue)],
		sequence_index: u8,
	) {
		for &(image_handle, _) in images {
			let Some(resource) = self.ensure_image_resource_for_sequence(image_handle, sequence_index) else {
				continue;
			};
			let Some(image) = self.images.get(image_handle.0 as usize) else {
				continue;
			};
			let Some(format) = image
				.uses
				.intersects(Uses::Storage)
				.then(|| Self::dxgi_shader_resource_format(image.format))
				.flatten()
			else {
				continue;
			};
			let description = Self::texture_uav_desc(format, image.array_layers);
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
		final_state: Option<D3D12_RESOURCE_STATES>,
		transition_before_clear: bool,
	) {
		let Some(command_list) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.command_list.clone())
		else {
			return;
		};
		let Some(destination) = self.ensure_image_resource_for_sequence(image_handle.0, sequence_index) else {
			return;
		};
		let Some(image) = self.images.get(image_handle.0 .0 as usize) else {
			return;
		};
		let image_format = image.format;
		let extent = image.extent;
		let uses_storage = image.uses.intersects(Uses::Storage);
		let array_layers = image.array_layers;
		let Some(format) = uses_storage
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
				unsafe {
					self.transition_tracked_image(&command_list, image_handle.0, &destination, final_state);
				}
			}
			return;
		};
		let desc = Self::texture_uav_desc(format, array_layers);
		if !self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
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
				unsafe {
					self.transition_tracked_image(&command_list, image_handle.0, &destination, final_state);
				}
			}
			return;
		};

		unsafe {
			if transition_before_clear {
				self.transition_tracked_image(
					&command_list,
					image_handle.0,
					&destination,
					D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
				);
			}
			self.bind_active_staged_descriptor_heaps(command_buffer_handle);
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
			if let Some(final_state) = final_state {
				// The transition orders the UAV clear and makes a separate UAV barrier redundant.
				self.transition_tracked_image(&command_list, image_handle.0, &destination, final_state);
			}
		}

		self.mark_command_buffer_work(command_buffer_handle);
		self.gpu_uploaded_images.insert(image_handle.0);
	}

	/// Records the legacy upload-backed clear path for textures that cannot be cleared through a DX12 UAV descriptor.
	pub(crate) fn record_image_clear_upload_fallback(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		command_list: &ID3D12GraphicsCommandList,
		image_handle: crate::BaseImageHandle,
		destination: ID3D12Resource,
		format: Formats,
		extent: Extent,
		clear: crate::ClearValue,
		sequence_index: u8,
	) {
		let (Some(dxgi_format), Some(bytes_per_pixel)) = (Self::dxgi_format(format), utils::bytes_per_pixel(format)) else {
			return;
		};
		if bytes_per_pixel != std::mem::size_of::<RGBAu8>() {
			return;
		}

		self.clear_image_for_sequence(image_handle, clear, sequence_index);

		let color = Self::clear_color_bytes(clear);
		let pixel_count = extent.width() as usize * extent.height() as usize * extent.depth().max(1) as usize;
		let mut source_bytes = vec![0u8; pixel_count * bytes_per_pixel];
		for pixel in source_bytes.chunks_exact_mut(bytes_per_pixel) {
			pixel.copy_from_slice(&color);
		}
		self.record_image_upload(
			command_buffer_handle,
			command_list,
			image_handle,
			destination,
			dxgi_format,
			extent,
			&source_bytes,
			extent.width() as usize * bytes_per_pixel,
			extent.width() as usize * extent.height() as usize * bytes_per_pixel,
			0,
		);
	}
}
