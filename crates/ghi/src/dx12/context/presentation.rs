//! DX12 device operations for presentation.

use super::*;

impl Device {
	pub fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: PresentationModes,
		fallback_extent: Extent,
		_uses: Uses,
	) -> SwapchainHandle {
		let extent = Self::query_window_extent(window_os_handles, fallback_extent);
		let image_count = self.frames.max(2);

		let (queue_index, queue) = self
			.queues
			.iter()
			.enumerate()
			.find(|(_, queue)| queue.workloads.intersects(WorkloadTypes::RASTER))
			.or_else(|| self.queues.first().map(|queue| (0, queue)))
			.expect("Failed to create a DXGI swapchain. The most likely cause is that no graphics queue was created.");
		let queue_handle = QueueHandle(queue_index as u64);

		let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }.unwrap_or_else(|_| {
			panic!("Failed to create a DXGI factory. The most likely cause is that the DXGI runtime is unavailable.");
		});

		let swapchain_desc = DXGI_SWAP_CHAIN_DESC1 {
			Width: extent.width(),
			Height: extent.height(),
			Format: DXGI_FORMAT_B8G8R8A8_UNORM,
			Stereo: false.into(),
			SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
			BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
			BufferCount: image_count as u32,
			Scaling: DXGI_SCALING_STRETCH,
			SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
			AlphaMode: DXGI_ALPHA_MODE_IGNORE,
			Flags: 0,
		};

		let swapchain = unsafe { factory.CreateSwapChainForHwnd(&queue.queue, window_os_handles.hwnd, &swapchain_desc, None, None) }.unwrap_or_else(|_| {
			panic!("Failed to create a DXGI swapchain. The most likely cause is that the window handle is invalid or the device does not support the swapchain format.");
		});

		let swapchain: IDXGISwapChain3 = swapchain.cast().unwrap_or_else(|_| {
			panic!(
				"Failed to upgrade the DXGI swapchain. The most likely cause is that the DXGI runtime does not support IDXGISwapChain3."
			);
		});

		let _ = unsafe { factory.MakeWindowAssociation(window_os_handles.hwnd, DXGI_MWA_NO_ALT_ENTER) };

		self.swapchains.push(Swapchain {
			handles: window::Handles {
				hinstance: window_os_handles.hinstance,
				hwnd: window_os_handles.hwnd,
			},
			swapchain,
			extent,
			image_count,
			next_image_index: 0,
			present_mode: presentation_mode,
			images: std::array::from_fn(|_| None),
			proxy_uses: std::array::from_fn(|_| Uses::empty()),
			proxy_resource_uses: Uses::empty(),
			backbuffers: std::array::from_fn(|_| None),
			acquired_image_indices: [0; 8],
			acquired_sequences: [false; 8],
			queue_handle,
		});

		SwapchainHandle((self.swapchains.len() - 1) as u64)
	}

	pub fn create_factory(&mut self) -> Option<crate::dx12::factory::Factory> {
		Some(crate::dx12::factory::Factory::default())
	}

	/// Returns one logical proxy whose dynamic storage resolves to the active frame sequence.
	pub fn get_swapchain_image(&mut self, swapchain_handle: SwapchainHandle, uses: Uses) -> (ImageHandle, Formats) {
		let (needs_new_proxy, requested_uses, resource_uses) = {
			let swapchain = &self.swapchains[swapchain_handle.0 as usize];
			let requested_uses = swapchain.proxy_uses[0] | uses;
			// A storage proxy is copied into the native backbuffer before Present. Other proxy uses should not
			// require optional typed-UAV support or unrelated render/copy capabilities from the BGRA format.
			let presentation_copy_uses = if requested_uses.intersects(Uses::Storage) {
				Uses::BlitSource
			} else {
				Uses::empty()
			};
			let resource_uses = swapchain.proxy_resource_uses | requested_uses | Uses::Image | presentation_copy_uses;
			(
				swapchain.images[0].is_none() || !swapchain.proxy_resource_uses.contains(requested_uses),
				requested_uses,
				resource_uses,
			)
		};

		if needs_new_proxy {
			let extent = self.swapchains[swapchain_handle.0 as usize].extent;
			let image = self.build_image(
				crate::image::Builder::new(Formats::BGRAu8, resource_uses)
					.extent(extent)
					.device_accesses(DeviceAccesses::DeviceOnly)
					.use_case(crate::UseCases::DYNAMIC),
			);
			let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
			// One dynamic logical image already resolves to distinct native resources for each frame sequence.
			swapchain.images = [Some(image); 8];
			swapchain.proxy_resource_uses = resource_uses;
		}
		self.swapchains[swapchain_handle.0 as usize].proxy_uses = [requested_uses; 8];
		if needs_new_proxy {
			self.invalidate_descriptor_materializations();
		}

		(
			self.swapchains[swapchain_handle.0 as usize].images[0].expect(
				"Missing DX12 swapchain proxy image. The most likely cause is that swapchain image access did not create the proxy image.",
			),
			Formats::BGRAu8,
		)
	}

	pub(crate) fn get_swapchain_image_for_sequence(
		&mut self,
		swapchain_handle: SwapchainHandle,
		uses: Uses,
		sequence_index: u8,
	) -> (ImageHandle, Formats) {
		self.get_swapchain_image(swapchain_handle, uses);
		let swapchain = &self.swapchains[swapchain_handle.0 as usize];
		debug_assert!(sequence_index < self.frames);
		(
			swapchain.images[0].expect(
				"Missing DX12 swapchain proxy image. The most likely cause is that swapchain image access did not create the proxy image.",
			),
			Formats::BGRAu8,
		)
	}

	pub fn get_image_data(
		&mut self,
		texture_copy_handle: TextureCopyHandle,
	) -> Result<crate::TextureReadback, crate::TextureTransferError> {
		// Keep native storage alive while submitted work is unresolved; only completed or failed mappings are consumed.
		if self.texture_readbacks.submitted(texture_copy_handle)?.resource.is_some() {
			return Err(crate::TextureTransferError::MappingFailed);
		}
		let readback = self.texture_readbacks.take_submitted(texture_copy_handle)?;
		if readback.mapping_failed {
			return Err(crate::TextureTransferError::MappingFailed);
		}
		Ok(crate::TextureReadback {
			bytes: readback.data.bytes,
			extent: readback.data.extent,
			format: readback.data.format,
			bytes_per_row: readback.data.bytes_per_row,
			bytes_per_image: readback.data.bytes_per_image,
		})
	}

	pub(crate) fn wait_for_texture_copy_readback(&mut self, texture_copy_handle: TextureCopyHandle) {
		let Some((synchronizer, value)) = self
			.texture_readbacks
			.get(texture_copy_handle)
			.filter(|readback| readback.resource.is_some())
			.and_then(|readback| readback.completion)
		else {
			return;
		};
		self.wait_for_private_synchronizer_value(synchronizer, value);
	}

	pub(crate) fn create_synchronizer_internal(&mut self, signaled: bool) -> crate::synchronizer::SynchronizerHandle {
		let handle = crate::synchronizer::SynchronizerHandle(self.synchronizers.len() as u64);
		let initial_value = if signaled { 1 } else { 0 };
		let fence = unsafe { self.device.CreateFence(initial_value, D3D12_FENCE_FLAGS(0)) }
			.expect("Failed to create a D3D12 fence. The most likely cause is that the device does not support fences.");
		self.synchronizers.push(Synchronizer {
			next: None,
			fence,
			value: initial_value,
			last_signal_queue: None,
		});
		handle
	}

	pub fn create_synchronizer(&mut self, _name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		let master = SynchronizerHandle(self.synchronizers.len() as u64);
		let mut previous: Option<crate::synchronizer::SynchronizerHandle> = None;
		for _ in 0..self.frames {
			let handle = self.create_synchronizer_internal(signaled);
			if let Some(previous) = previous {
				self.synchronizers[previous.0 as usize].next = Some(handle);
			}
			previous = Some(handle);
		}
		self.synchronizer_masters.push(master);
		master
	}

	pub fn start_frame<'a>(&'a mut self, index: u64, synchronizer_handle: SynchronizerHandle) -> super::super::Frame<'a> {
		let frame_key = crate::FrameKey {
			frame_index: index,
			sequence_index: (index % u64::from(self.frames)) as u8,
		};
		assert!(
			!self.untracked_present_work
				&& self
					.command_buffers
					.iter()
					.all(|command_buffer| !command_buffer.frames_any(|lifecycle| lifecycle == CommandBufferLifecycle::Poisoned)),
			"DX12 frame reuse is unavailable after an untracked native submission. The most likely cause is that presentation or its terminal fence signal failed."
		);
		let previous = self.last_frame_synchronizers[frame_key.sequence_index as usize];
		if let Some(previous) = previous {
			self.wait_for_synchronizer_sequence(previous, frame_key.sequence_index);
		}
		if previous != Some(synchronizer_handle) {
			self.wait_for_synchronizer_sequence(synchronizer_handle, frame_key.sequence_index);
		}
		self.last_frame_synchronizers[frame_key.sequence_index as usize] = Some(synchronizer_handle);
		self.process_tasks(frame_key.sequence_index);
		super::super::Frame::new(self, frame_key, synchronizer_handle)
	}

	/// Replaces CPU shadow storage immediately while retaining each native allocation through its owning sequence fence.
	pub fn resize_buffer<T: crate::Pod>(&mut self, buffer_handle: DynamicBufferHandle<T>, size: usize) {
		let buffer_handle: BaseBufferHandle = buffer_handle.into();
		let (current_size, current_layout, current_access, current_uses) = {
			let buffer = self.buffer(buffer_handle).expect(
				"Missing DX12 dynamic buffer. The most likely cause is that the buffer handle came from another device.",
			);
			(buffer.size, buffer.layout, buffer.access, buffer.uses)
		};

		if current_size >= size {
			return;
		}

		let layout = Layout::from_size_align(size, current_layout.align()).unwrap();
		let data = if layout.size() == 0 {
			Self::zero_sized_buffer_pointer(layout)
		} else {
			unsafe { alloc::alloc_zeroed(layout) }
		};
		if layout.size() != 0 && data.is_null() {
			panic!("Failed to resize buffer storage. The most likely cause is that the system is out of memory.");
		}

		let frame_count = self.frames as usize;
		let resource_size = Self::buffer_resource_size(size, current_uses);
		let (resource, mapped, heap_kind) = self.create_buffer_resource(resource_size, current_access);
		let (retired_resource, retired_frame_resources, retired_data, retired_layout) = {
			let buffer = self.buffer_mut(buffer_handle).expect(
				"Missing DX12 dynamic buffer. The most likely cause is that the buffer handle came from another device.",
			);
			let retired_resource = std::mem::replace(&mut buffer.resource, resource);
			let retired_frame_resources = buffer.frame_resources.as_mut().map_or_else(SmallVec::new, |resources| {
				let mut retired = SmallVec::<[(usize, BufferFrameStorage); crate::MAX_FRAMES_IN_FLIGHT]>::new();
				for (sequence_index, storage) in resources.iter_mut().enumerate() {
					if let Some(storage) = storage.take() {
						retired.push((sequence_index, storage));
					}
				}
				resources.resize_with(frame_count, || None);
				retired
			});
			let retired_data = std::mem::replace(&mut buffer.data, data);
			let retired_layout = std::mem::replace(&mut buffer.layout, layout);
			buffer.size = size;
			buffer.host_generation = 1;
			buffer.uploaded_generation = 0;
			buffer.mapped = mapped;
			buffer.heap_kind = heap_kind;
			(retired_resource, retired_frame_resources, retired_data, retired_layout)
		};
		if retired_layout.size() != 0 && !retired_data.is_null() {
			// SAFETY: The pointer was allocated with retired_layout and ownership moved out of the buffer above.
			unsafe { alloc::dealloc(retired_data, retired_layout) };
		}
		let mut retired_state_keys = SmallVec::<[usize; 4]>::new();
		if let Some(resource) = retired_resource {
			retired_state_keys.push(Self::native_resource_key(&resource));
			self.defer_task(0, DeferredTask::RetireResource(resource));
		}
		for (sequence_index, storage) in retired_frame_resources {
			if let Some(resource) = storage.resource.as_ref() {
				retired_state_keys.push(Self::native_resource_key(resource));
			}
			self.defer_task(sequence_index as u8, DeferredTask::RetireBufferFrameStorage(storage));
		}
		self.invalidate_clear_uav_descriptors_for_resources(&retired_state_keys);
		for key in retired_state_keys {
			self.buffer_states.remove(&key);
		}
		self.invalidate_descriptor_materializations();
	}

	pub fn start_frame_capture(&mut self) {
		self.debugger.start_frame_capture();
	}

	pub fn end_frame_capture(&mut self) {
		self.debugger.end_frame_capture();
	}

	pub fn wait(&self) {
		for index in 0..self.synchronizers.len() {
			self.wait_for_private_synchronizer(crate::synchronizer::SynchronizerHandle(index as u64));
		}
	}

	/// Establishes and waits for one terminal fence on every distinct native queue.
	pub(crate) fn wait_for_all_queues_idle(&mut self) -> windows::core::Result<()> {
		assert!(
			self.active_command_buffer.is_none()
				&& self.command_buffers.iter().all(|command_buffer| {
					!command_buffer.frames_any(|lifecycle| {
						matches!(
							lifecycle,
							CommandBufferLifecycle::Recording | CommandBufferLifecycle::Scheduled
						)
					})
				}),
			"DX12 queue-idle wait started during command recording. The most likely cause is that a resource topology change was requested before its execution finished collecting command buffers."
		);
		let mut terminal_fences = SmallVec::<[(usize, ID3D12Fence); 4]>::new();
		for queue in &self.queues {
			let identity = queue.queue.as_raw() as usize;
			if terminal_fences.iter().any(|(queued, _)| *queued == identity) {
				continue;
			}
			let fence = unsafe { self.device.CreateFence(0, D3D12_FENCE_FLAGS(0)) }?;
			unsafe { queue.queue.Signal(&fence, 1) }?;
			terminal_fences.push((identity, fence));
		}
		for (_, fence) in terminal_fences {
			if let Err(event_error) = unsafe { fence.SetEventOnCompletion(1, HANDLE::default()) } {
				// Event registration may fail under memory pressure. The queue signal is already ordered, so polling
				// its value is a safe, allocation-free fallback for this exceptional topology/shutdown path.
				loop {
					let completed = unsafe { fence.GetCompletedValue() };
					if completed == u64::MAX {
						return Err(unsafe { self.device.GetDeviceRemovedReason() }.err().unwrap_or(event_error));
					}
					if completed >= 1 {
						break;
					}
					std::thread::yield_now();
				}
			} else if unsafe { fence.GetCompletedValue() } == u64::MAX {
				return unsafe { self.device.GetDeviceRemovedReason() };
			}
		}
		Ok(())
	}

	/// Queues an unresolved fence wait so tests can prove that another frame sequence does not wait on it from the CPU.
	#[cfg(test)]
	pub(crate) fn block_queue_until_test_fence(&self, queue_handle: QueueHandle) -> windows::core::Result<ID3D12Fence> {
		let queue = self
			.queues
			.get(queue_handle.0 as usize)
			.expect("Invalid DX12 queue handle. The most likely cause is that the test used a queue from another device.");
		let fence = unsafe { self.device.CreateFence(0, D3D12_FENCE_FLAGS(0)) }?;
		unsafe { queue.queue.Wait(&fence, 1) }?;
		Ok(fence)
	}

	/// Resets completed command lists and releases engine-owned references before a resource topology change.
	pub(crate) fn prepare_for_topology_change(&mut self) -> windows::core::Result<()> {
		assert!(
			self.active_command_buffer.is_none()
				&& self.command_buffers.iter().all(|command_buffer| {
					!command_buffer.frames_any(|lifecycle| {
						!matches!(lifecycle, CommandBufferLifecycle::Idle | CommandBufferLifecycle::Submitted)
					})
				}),
			"DX12 topology is unavailable while command recording or an untracked native submission is active. The most likely cause is that a resize was requested before its execution completed reliably."
		);
		for command_buffer in &mut self.command_buffers {
			for frame in &mut command_buffer.frames {
				if let (Some(allocator), Some(command_list)) = (frame.allocator.as_ref(), frame.command_list.as_ref()) {
					if frame.is_open {
						if let Err(error) = unsafe { command_list.Close() } {
							// D3D12 permanently invalidates a command list whose Close call fails.
							frame.lifecycle = CommandBufferLifecycle::Poisoned;
							return Err(error);
						}
						frame.is_open = false;
					}
					unsafe { allocator.Reset() }?;
					unsafe { command_list.Reset(allocator, None) }?;
					if let Err(error) = unsafe { command_list.Close() } {
						// A failed Close permanently retires the list; do not publish it as reusable after topology maintenance.
						frame.lifecycle = CommandBufferLifecycle::Poisoned;
						return Err(error);
					}
				}
				frame.is_open = false;
				frame.lifecycle = CommandBufferLifecycle::Idle;
				frame.clear_recording_state();
				frame.release_staging_heaps();
			}
			command_buffer.active_sequence_index = 0;
		}
		self.untracked_present_work = false;
		self.invalidate_descriptor_materializations();
		Ok(())
	}

	/// Returns every fence allocated for a logical synchronizer, including dormant frame sequences.
	pub(crate) fn all_synchronizer_handles(
		&self,
		synchronizer_handle: SynchronizerHandle,
	) -> SmallVec<[crate::synchronizer::SynchronizerHandle; crate::MAX_FRAMES_IN_FLIGHT]> {
		crate::synchronizer::SynchronizerHandle(synchronizer_handle.0).get_all(&self.synchronizers)
	}

	/// Returns only the fence chain visible to the current frames-in-flight topology.
	pub(crate) fn synchronizer_handles(
		&self,
		synchronizer_handle: SynchronizerHandle,
	) -> SmallVec<[crate::synchronizer::SynchronizerHandle; crate::MAX_FRAMES_IN_FLIGHT]> {
		let mut handles = self.all_synchronizer_handles(synchronizer_handle);
		handles.truncate(self.frames as usize);
		handles
	}

	pub(crate) fn synchronizer_for_sequence(
		&self,
		synchronizer_handle: SynchronizerHandle,
		sequence_index: u8,
	) -> Option<crate::synchronizer::SynchronizerHandle> {
		let handles = self.synchronizer_handles(synchronizer_handle);
		handles
			.get(sequence_index as usize)
			.copied()
			.or_else(|| handles.last().copied())
	}

	pub(crate) fn wait_for_private_synchronizer(&self, synchronizer_handle: crate::synchronizer::SynchronizerHandle) {
		let Some(synchronizer) = self.synchronizers.get(synchronizer_handle.0 as usize) else {
			return;
		};
		self.wait_for_private_synchronizer_value(synchronizer_handle, synchronizer.value);
	}

	/// Blocks without polling until one concrete fence reaches the captured submission value.
	pub(crate) fn wait_for_private_synchronizer_value(
		&self,
		synchronizer_handle: crate::synchronizer::SynchronizerHandle,
		value: u64,
	) {
		let Some(synchronizer) = self.synchronizers.get(synchronizer_handle.0 as usize) else {
			return;
		};
		if unsafe { synchronizer.fence.GetCompletedValue() } >= value {
			return;
		}
		// A null event asks D3D12 to block this thread until completion without a busy-yield loop.
		unsafe { synchronizer.fence.SetEventOnCompletion(value, HANDLE::default()) }.expect(
			"Failed to wait for a DX12 fence. The most likely cause is that the fence was invalid or the device was removed.",
		);
	}

	pub(crate) fn wait_for_synchronizer(&mut self, synchronizer_handle: SynchronizerHandle) {
		for handle in self.synchronizer_handles(synchronizer_handle) {
			self.wait_for_private_synchronizer(handle);
		}
		self.refresh_readback_texture_copies(None);
	}

	pub(crate) fn wait_for_synchronizer_sequence(&mut self, synchronizer_handle: SynchronizerHandle, sequence_index: u8) {
		let Some(handle) = self.synchronizer_for_sequence(synchronizer_handle, sequence_index) else {
			return;
		};
		self.wait_for_private_synchronizer(handle);
		self.refresh_readback_texture_copies(Some(sequence_index));
	}

	pub(crate) fn synchronizer_value(&self, synchronizer_handle: SynchronizerHandle) -> Option<u64> {
		self.synchronizers
			.get(synchronizer_handle.0 as usize)
			.map(|synchronizer| synchronizer.value)
	}

	/// Validates acquired swapchain ownership and uniqueness before command recording or queue presentation mutates state.
	pub(crate) fn validate_present_keys(&self, queue_handle: QueueHandle, sequence_index: u8, present_keys: &[PresentKey]) {
		for (index, &present_key) in present_keys.iter().enumerate() {
			assert!(
				!present_keys[..index].contains(&present_key),
				"Duplicate DX12 present key. The most likely cause is that one execution returned the same acquired image more than once."
			);
			assert_eq!(
				present_key.sequence_index, sequence_index,
				"Invalid DX12 present sequence. The most likely cause is that a present key was reused by a different frame."
			);
			let swapchain = self.swapchains.get(present_key.swapchain.0 as usize).expect(
				"Invalid DX12 swapchain handle. The most likely cause is that the present key came from another device.",
			);
			assert_eq!(
				swapchain.queue_handle, queue_handle,
				"Invalid DX12 presentation queue. The most likely cause is that the swapchain was created for a different queue."
			);
			assert!(
				present_key.image_index < swapchain.image_count,
				"Invalid DX12 swapchain image index. The most likely cause is that the present key predates a swapchain resize."
			);
			let sequence = usize::from(sequence_index);
			assert!(
				swapchain.acquired_sequences[sequence],
				"DX12 swapchain image was not acquired. The most likely cause is that a present key was reused after presentation."
			);
			assert_eq!(
				swapchain.acquired_image_indices[sequence], present_key.image_index,
				"DX12 swapchain image no longer matches the acquired image. The most likely cause is that a stale present key was reused."
			);
		}
	}

	/// Checks that recorded proxy preparation and the final presentation set describe the same storage-present work.
	pub(crate) fn validate_present_preparation(&self, prepared: &[PresentKey], presented: &[PresentKey]) {
		for &present_key in prepared {
			assert!(
				presented.contains(&present_key),
				"Missing DX12 presentation request. The most likely cause is that a prepared swapchain image was omitted from the execution's returned present keys."
			);
		}
		for &present_key in presented {
			let uses_storage_proxy = self
				.swapchains
				.get(present_key.swapchain.0 as usize)
				.is_some_and(|swapchain| swapchain.proxy_uses[present_key.sequence_index as usize].intersects(Uses::Storage));
			assert!(
				!uses_storage_proxy || prepared.contains(&present_key),
				"Missing DX12 storage presentation preparation. The most likely cause is that record_with_present_keys omitted a storage-backed swapchain image."
			);
		}
	}

	/// Rejects new work until a global idle boundary recovers any presentation without a terminal fence.
	pub(crate) fn validate_queue_submission_state(&self) {
		assert!(
			!self.untracked_present_work
				&& self
					.command_buffers
					.iter()
					.all(|command_buffer| !command_buffer.frames_any(|lifecycle| lifecycle == CommandBufferLifecycle::Poisoned)),
			"DX12 queue submission is unavailable after untracked native work. The most likely cause is that presentation, command-list closure, or its terminal fence signal failed."
		);
	}

	/// Publishes that a successful terminal signal now tracks every presentation in the execution.
	pub(crate) fn complete_present_submission(&mut self, presented: bool) {
		if presented {
			self.untracked_present_work = false;
		}
	}

	pub(crate) fn begin_command_buffer(&mut self, command_buffer_handle: CommandBufferHandle, sequence_index: u8) {
		self.validate_queue_submission_state();
		self.command_buffers
			.get_mut(command_buffer_handle.0 as usize)
			.expect("Invalid DX12 command buffer handle. The most likely cause is that the handle came from another device.")
			.activate_sequence(sequence_index);
		let (lifecycle, last_submission) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.map(|command_buffer| (command_buffer.lifecycle, command_buffer.last_submission))
			.expect("Invalid DX12 command buffer handle. The most likely cause is that the handle came from another device.");
		assert!(
			matches!(
				lifecycle,
				CommandBufferLifecycle::Idle | CommandBufferLifecycle::Recording | CommandBufferLifecycle::Submitted
			),
			"DX12 command buffer cannot be reset. The most likely cause is that it is scheduled or its native submission has no reliable completion fence."
		);
		if let Some((synchronizer_handle, previous_sequence_index)) = last_submission {
			self.wait_for_synchronizer_sequence(synchronizer_handle, previous_sequence_index);
		}
		self.discard_command_buffer_recording(command_buffer_handle);

		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		if command_buffer.allocator.is_none() || command_buffer.command_list.is_none() {
			return;
		}

		if command_buffer.is_open {
			let close_result = unsafe { command_buffer.command_list.as_ref().unwrap().Close() };
			if close_result.is_err() {
				// Close failure permanently invalidates this native list. Poison the device-facing handle so it is never reset.
				command_buffer.lifecycle = CommandBufferLifecycle::Poisoned;
				panic!(
					"Failed to close an abandoned DX12 command list. The most likely cause is that earlier command recording was invalid."
				);
			}
			command_buffer.is_open = false;
		}
		unsafe { command_buffer.allocator.as_ref().unwrap().Reset() }.expect(
			"Failed to reset a DX12 command allocator. The most likely cause is that its previous GPU submission is still running.",
		);
		let reset_result = unsafe {
			command_buffer
				.command_list
				.as_ref()
				.unwrap()
				.Reset(command_buffer.allocator.as_ref().unwrap(), None)
		};
		reset_result.expect(
			"Failed to reset a DX12 command list. The most likely cause is that the list was not closed or its allocator is invalid.",
		);
		// Reset removes recorded references before fence-complete transient resources and heaps are released.
		command_buffer.clear_recording_state();
		command_buffer.sequence_index = sequence_index;
		command_buffer.rewind_staging_heaps();
		command_buffer.is_open = true;
		command_buffer.lifecycle = CommandBufferLifecycle::Recording;
		self.begin_command_buffer_state_transaction(command_buffer_handle);
	}

	/// Associates a frame recording with the logical fence that must complete its sequence.
	pub(crate) fn set_command_buffer_frame_synchronizer(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		synchronizer_handle: SynchronizerHandle,
	) {
		let command_buffer = self
			.command_buffers
			.get_mut(command_buffer_handle.0 as usize)
			.expect("Invalid DX12 command buffer handle. The most likely cause is that the handle came from another device.");
		command_buffer.frame_synchronizer = Some(synchronizer_handle);
	}

	/// Marks a command buffer as containing GPU-visible work that must be submitted.
	pub(crate) fn mark_command_buffer_work(&mut self, command_buffer_handle: CommandBufferHandle) {
		if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) {
			command_buffer.recorded_work = true;
		}
	}
}
