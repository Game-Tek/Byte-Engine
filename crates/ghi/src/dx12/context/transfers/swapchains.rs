use super::*;

impl Device {
	/// Resizes a swapchain only after every native queue is idle, then updates its frame-local proxy through deferred tasks.
	pub(crate) fn swapchain_extent(&mut self, swapchain_handle: SwapchainHandle, sequence_index: u8) -> Extent {
		let Some(swapchain) = self.swapchains.get(swapchain_handle.0 as usize) else {
			return Extent::rectangle(0, 0);
		};
		let extent = Self::query_window_extent(&swapchain.handles, swapchain.extent);
		let image_count = self.frames.max(2);
		if (extent != swapchain.extent || image_count != swapchain.image_count) && extent.width() > 0 && extent.height() > 0 {
			self.wait_for_all_queues_idle().expect(
				"Failed to wait for DX12 queues before resizing a swapchain. The most likely cause is that the device was removed.",
			);
			self.prepare_for_topology_change().expect(
				"Failed to reset DX12 command lists before resizing a swapchain. The most likely cause is that a completed command list became invalid.",
			);
			self.process_all_tasks_after_idle();
			let retired_backbuffers = self.swapchains[swapchain_handle.0 as usize]
				.backbuffers
				.iter()
				.flatten()
				.map(Self::native_resource_key)
				.collect::<SmallVec<[usize; 8]>>();
			self.invalidate_attachment_views_for_resources(&retired_backbuffers);
			for &key in &retired_backbuffers {
				self.image_states.remove(&key);
			}
			let proxy = self.swapchains[swapchain_handle.0 as usize].images[0];
			let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
			// DXGI requires every application-owned backbuffer reference to be released before ResizeBuffers.
			swapchain.backbuffers = std::array::from_fn(|_| None);
			swapchain.acquired_sequences = [false; 8];
			let result = unsafe {
				swapchain.swapchain.ResizeBuffers(
					image_count as u32,
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
			swapchain.image_count = image_count;
			swapchain.next_image_index %= image_count;
			if let Some(proxy) = proxy {
				self.resize_image_internal(proxy, extent, sequence_index);
			}
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
		let swapchain_index = present_key.swapchain.0 as usize;
		if swapchain_index >= self.swapchains.len() {
			return;
		}
		let current_image_index = unsafe { self.swapchains[swapchain_index].swapchain.GetCurrentBackBufferIndex() } as u8;
		assert_eq!(
			current_image_index, present_key.image_index,
			"DX12 swapchain image changed before presentation. The most likely cause is that an acquired image was presented or advanced outside its owning execution."
		);
		// Present queues native work outside command lists. Keep it poisoned until the execution appends its terminal signal.
		self.untracked_present_work = true;
		let swapchain = &mut self.swapchains[swapchain_index];

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
		// One swapchain can have only one outstanding acquisition, so a successful Present consumes its full ownership state.
		swapchain.acquired_sequences = [false; 8];
	}
}
