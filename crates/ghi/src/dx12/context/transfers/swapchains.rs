use super::*;

impl Device {
	pub(crate) fn swapchain_extent(&mut self, swapchain_handle: SwapchainHandle) -> Extent {
		let Some(swapchain) = self.swapchains.get(swapchain_handle.0 as usize) else {
			return Extent::rectangle(0, 0);
		};
		let extent = Self::query_window_extent(&swapchain.handles, swapchain.extent);
		if extent != swapchain.extent && extent.width() > 0 && extent.height() > 0 {
			let retired_backbuffers = swapchain
				.backbuffers
				.iter()
				.flatten()
				.map(Self::native_resource_key)
				.collect::<SmallVec<[usize; 8]>>();
			self.invalidate_attachment_views_for_resources(&retired_backbuffers);
			let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
			// DXGI requires every application-owned backbuffer reference to be released before ResizeBuffers.
			swapchain.backbuffers = std::array::from_fn(|_| None);
			let result = unsafe {
				swapchain.swapchain.ResizeBuffers(
					swapchain.image_count as u32,
					extent.width(),
					extent.height(),
					DXGI_FORMAT_B8G8R8A8_UNORM,
					DXGI_SWAP_CHAIN_FLAG(0),
				)
			};

			if result.is_err() {
				panic!(
					"Failed to resize the DXGI swapchain buffers. The most likely cause is that the swapchain is still in use or the device was removed."
				);
			}

			swapchain.extent = extent;
		}
		extent
	}

	pub(crate) fn next_swapchain_image_index(&mut self, swapchain_handle: SwapchainHandle) -> u8 {
		let Some(swapchain) = self.swapchains.get_mut(swapchain_handle.0 as usize) else {
			return 0;
		};

		let index = unsafe { swapchain.swapchain.GetCurrentBackBufferIndex() } as u8;
		let image_count = swapchain.image_count.max(1);
		swapchain.next_image_index = (index + 1) % image_count;
		index
	}

	pub(crate) fn present_swapchain(&mut self, present_key: PresentKey) {
		let Some(swapchain) = self.swapchains.get_mut(present_key.swapchain.0 as usize) else {
			return;
		};

		let sync_interval = match swapchain.present_mode {
			PresentationModes::FIFO => 1,
			PresentationModes::Mailbox | PresentationModes::Inmediate => 0,
		};

		let result = unsafe { swapchain.swapchain.Present(sync_interval, DXGI_PRESENT(0)) };
		if result.is_err() {
			let removed_reason = unsafe { self.device.GetDeviceRemovedReason() };
			self.log_dx12_error(format!(
				"Failed to present DX12 swapchain. HRESULT: {result:?}. Device removed reason: {removed_reason:?}"
			));
			panic!(
				"Failed to present the DXGI swapchain. The most likely cause is that the device was removed or the swapchain became invalid."
			);
		}
	}
}
