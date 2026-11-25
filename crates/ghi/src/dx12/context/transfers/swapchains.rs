use super::*;

impl Device {
	/// Waits for every native queue, then releases a swapchain, its backbuffers, and their attachment views.
	pub(crate) fn destroy_swapchain(&mut self, swapchain_handle: SwapchainHandle) {
		let index = swapchain_handle.index() as usize;
		if self.swapchains.get(index).is_none() {
			return;
		}
		self.idle_for_topology_change("destroying a swapchain");
		let retired_backbuffers = self.swapchains[index]
			.backbuffers
			.iter()
			.flatten()
			.map(Self::native_resource_key)
			.collect::<SmallVec<[usize; 8]>>();
		self.forget_native_resources(&retired_backbuffers);
		// Dropping the swapchain releases its backbuffers and the DXGI swapchain itself.
		self.swapchains.remove(index);
	}

	/// Resizes a swapchain and its proxy textures while every native queue is idle.
	pub(crate) fn swapchain_extent(&mut self, swapchain_handle: SwapchainHandle, sequence_index: u8) -> Extent {
		let Some(swapchain) = self.swapchains.get(swapchain_handle.index() as usize) else {
			return Extent::rectangle(0, 0);
		};
		let extent = Self::query_window_extent(&swapchain.handles, swapchain.extent);
		let image_count = swapchain_image_count(self.frames, swapchain.desired_image_count);
		if (extent != swapchain.extent || image_count != swapchain.image_count) && extent.width() > 0 && extent.height() > 0 {
			self.idle_for_topology_change("resizing a swapchain");
			let swapchain = &mut self.swapchains[swapchain_handle.index() as usize];
			let retired_backbuffers = swapchain
				.backbuffers
				.iter()
				.flatten()
				.map(Self::native_resource_key)
				.collect::<SmallVec<[usize; 8]>>();
			let proxy = swapchain.image;
			swapchain.resize_buffers(image_count, extent);
			self.forget_native_resources(&retired_backbuffers);
			if let Some(proxy) = proxy {
				self.resize_image_internal(proxy, extent, None, sequence_index);
				// Finish replacing all proxy copies before any descriptor can bind them again.
				self.process_all_tasks_after_idle();
				// Swapchain bindings name the swapchain rather than the proxy image handle.
				self.mark_stale_descriptors(|descriptor, _| descriptor == WriteData::Swapchain(swapchain_handle));
			}
		}
		extent
	}

	pub(crate) fn next_swapchain_image_index(&mut self, swapchain_handle: SwapchainHandle) -> u8 {
		let Some(swapchain) = self.swapchains.get_mut(swapchain_handle.index() as usize) else {
			return 0;
		};

		let index = unsafe { swapchain.swapchain.GetCurrentBackBufferIndex() } as u8;
		let image_count = swapchain.image_count.max(1);
		swapchain.next_image_index = (index + 1) % image_count;
		index
	}

	pub(crate) fn present_swapchain(&mut self, present_key: PresentKey) {
		let swapchain_index = present_key.swapchain.index() as usize;
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

		// Acquisition chose the sync interval. A present that does not wait for a refresh tears when the swapchain
		// allows it; otherwise the compositor shows the newest frame on the next refresh.
		let flags = if swapchain.sync_interval == 0 && swapchain.allows_tearing {
			DXGI_PRESENT_ALLOW_TEARING
		} else {
			DXGI_PRESENT(0)
		};

		let result = unsafe { swapchain.swapchain.Present(swapchain.sync_interval, flags) };
		if result.is_err() {
			let report = device_removed_report(&self.device);
			self.log_dx12_error(format!("Failed to present DX12 swapchain. HRESULT: {result:?}. {report}"));
			panic!(
				"Failed to present the DXGI swapchain. The most likely cause is that the device was removed or the swapchain became invalid."
			);
		}
		// One swapchain can have only one outstanding acquisition, so a successful Present consumes its full ownership state.
		swapchain.acquired_sequences = [false; 8];
	}
}
