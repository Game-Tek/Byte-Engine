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

		let queue = self
			.queues
			.iter()
			.find(|queue| queue.queue_type == D3D12_COMMAND_LIST_TYPE_DIRECT)
			.or_else(|| self.queues.first())
			.expect("Failed to create a DXGI swapchain. The most likely cause is that no graphics queue was created.");

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
			backbuffers: std::array::from_fn(|_| None),
			acquired_image_indices: [0; 8],
		});

		SwapchainHandle((self.swapchains.len() - 1) as u64)
	}

	pub fn create_factory(&mut self) -> Option<crate::dx12::factory::Factory> {
		Some(crate::dx12::factory::Factory::default())
	}

	pub fn get_swapchain_image(&mut self, swapchain_handle: SwapchainHandle, uses: Uses) -> (ImageHandle, Formats) {
		let needs_new_proxy = {
			let swapchain = &self.swapchains[swapchain_handle.0 as usize];
			swapchain.images[0].is_none() || !swapchain.proxy_uses[0].contains(uses)
		};

		if needs_new_proxy {
			let extent = self.swapchains[swapchain_handle.0 as usize].extent;
			let mut images = [None; 8];
			for image_index in 0..8 {
				let image = self.build_image(
					crate::image::Builder::new(Formats::BGRAu8, uses | Uses::BlitSource)
						.extent(extent)
						.device_accesses(DeviceAccesses::DeviceOnly)
						.use_case(crate::UseCases::DYNAMIC),
				);
				images[image_index] = Some(image);
			}
			let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
			swapchain.images = images;
			swapchain.proxy_uses = [uses; 8];
		}
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
		let image_index = sequence_index as usize;
		(
			swapchain.images[image_index].or(swapchain.images[0]).expect(
				"Missing DX12 swapchain proxy image. The most likely cause is that swapchain image access did not create the proxy image.",
			),
			Formats::BGRAu8,
		)
	}

	pub fn get_image_data<'a>(&'a self, texture_copy_handle: TextureCopyHandle) -> &'a [u8] {
		self.texture_copies
			.get(texture_copy_handle.0 as usize)
			.map(|v| v.as_slice())
			.unwrap_or(&[])
	}

	pub(crate) fn wait_for_texture_copy_readback(&mut self, texture_copy_handle: TextureCopyHandle) {
		let Some(sequence_index) = self
			.texture_readbacks
			.iter()
			.find(|readback| readback.texture_copy == texture_copy_handle && !readback.resolved)
			.map(|readback| readback.sequence_index)
		else {
			return;
		};
		let synchronizers = self
			.command_buffers
			.iter()
			.filter_map(|command_buffer| match command_buffer.last_submission {
				Some((synchronizer, submitted_sequence)) if submitted_sequence == sequence_index => Some(synchronizer),
				_ => None,
			})
			.collect::<SmallVec<[_; 4]>>();
		for synchronizer in synchronizers {
			self.wait_for_synchronizer_sequence(synchronizer, sequence_index);
		}
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
		master
	}

	pub fn start_frame<'a>(&'a mut self, index: u64, _synchronizer_handle: SynchronizerHandle) -> super::super::Frame<'a> {
		let frame_key = crate::FrameKey {
			frame_index: index,
			sequence_index: (index % u64::from(self.frames)) as u8,
		};
		self.wait_for_synchronizer_sequence(_synchronizer_handle, frame_key.sequence_index);
		super::super::Frame::new(self, frame_key)
	}

	pub fn resize_buffer<T: Copy>(&mut self, buffer_handle: DynamicBufferHandle<T>, size: usize) {
		// Resizes CPU-side buffer storage while discarding previous per-frame contents.
		let buffer_handle: BaseBufferHandle = buffer_handle.into();
		let (current_size, current_layout, current_data, current_access, current_uses, retired_state_keys) = {
			let buffer = self.buffer(buffer_handle).expect(
				"Missing DX12 dynamic buffer. The most likely cause is that the buffer handle came from another device.",
			);
			let mut retired_state_keys = SmallVec::<[usize; 4]>::new();
			retired_state_keys.extend(buffer.resource.as_ref().map(Self::native_resource_key));
			if let Some(frame_resources) = buffer.frame_resources.as_ref() {
				retired_state_keys.extend(
					frame_resources
						.iter()
						.flatten()
						.filter_map(|frame| frame.resource.as_ref())
						.map(Self::native_resource_key),
				);
			}
			(
				buffer.size,
				buffer.layout,
				buffer.data,
				buffer.access,
				buffer.uses,
				retired_state_keys,
			)
		};

		if current_size >= size {
			return;
		}

		let layout = Layout::from_size_align(size, current_layout.align()).unwrap();
		let data = if layout.size() == 0 {
			std::ptr::NonNull::<u8>::dangling().as_ptr()
		} else {
			unsafe { alloc::alloc_zeroed(layout) }
		};
		if layout.size() != 0 && data.is_null() {
			panic!("Failed to resize buffer storage. The most likely cause is that the system is out of memory.");
		}

		if current_layout.size() != 0 && !current_data.is_null() {
			unsafe {
				alloc::dealloc(current_data, current_layout);
			}
		}

		let frame_count = self.frames as usize;
		let resource_size = Self::buffer_resource_size(size, current_uses);
		let (resource, mapped, heap_kind) = self.create_buffer_resource(resource_size, current_access);
		self.invalidate_clear_uav_descriptors_for_resources(&retired_state_keys);
		for key in retired_state_keys {
			self.buffer_states.remove(&key);
		}
		let buffer = self
			.buffer_mut(buffer_handle)
			.expect("Missing DX12 dynamic buffer. The most likely cause is that the buffer handle came from another device.");
		buffer.data = data;
		buffer.layout = layout;
		buffer.size = size;
		buffer.resource = resource;
		buffer.mapped = mapped;
		buffer.heap_kind = heap_kind;
		if let Some(frame_resources) = buffer.frame_resources.as_mut() {
			frame_resources.clear();
			frame_resources.resize_with(frame_count, || None);
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

	pub(crate) fn synchronizer_handles(
		&self,
		synchronizer_handle: SynchronizerHandle,
	) -> SmallVec<[crate::synchronizer::SynchronizerHandle; crate::MAX_FRAMES_IN_FLIGHT]> {
		crate::synchronizer::SynchronizerHandle(synchronizer_handle.0).get_all(&self.synchronizers)
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
		while unsafe { synchronizer.fence.GetCompletedValue() } < synchronizer.value {
			std::thread::yield_now();
		}
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

	pub(crate) fn begin_command_buffer(&mut self, command_buffer_handle: CommandBufferHandle, sequence_index: u8) {
		if let Some((synchronizer_handle, previous_sequence_index)) = self
			.command_buffers
			.get(command_buffer_handle.0 as usize)
			.and_then(|command_buffer| command_buffer.last_submission)
		{
			self.wait_for_synchronizer_sequence(synchronizer_handle, previous_sequence_index);
		}

		let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) else {
			return;
		};
		let (Some(allocator), Some(command_list)) = (command_buffer.allocator.as_ref(), command_buffer.command_list.as_ref())
		else {
			return;
		};

		if command_buffer.is_open {
			let _ = unsafe { command_list.Close() };
			command_buffer.is_open = false;
		}
		command_buffer.recorded_work = false;
		command_buffer.pending_clear_descriptor_copies.clear();
		command_buffer.prepared_clear_descriptors.clear();
		command_buffer.sequence_index = sequence_index;
		command_buffer.last_submission = None;
		let _ = unsafe { allocator.Reset() };
		let _ = unsafe { command_list.Reset(allocator, None) };
		// Reset removes recorded references before fence-complete transient resources and heaps are released.
		command_buffer.retained_descriptor_heaps.clear();
		command_buffer.retained_resources.clear();
		command_buffer.retained_upload_resource_count = 0;
		if let Some(arena) = command_buffer.cbv_srv_uav_staging_heap.as_mut() {
			arena.used = 0;
		}
		if let Some(arena) = command_buffer.sampler_staging_heap.as_mut() {
			arena.used = 0;
		}
		command_buffer.is_open = true;
		// Resetting an unsubmitted command list discards its copies, so its pending readbacks have no future completion.
		self.texture_readbacks
			.retain(|readback| readback.command_buffer_handle != command_buffer_handle);
	}

	/// Marks a command buffer as containing GPU-visible work that must be submitted.
	pub(crate) fn mark_command_buffer_work(&mut self, command_buffer_handle: CommandBufferHandle) {
		if let Some(command_buffer) = self.command_buffers.get_mut(command_buffer_handle.0 as usize) {
			command_buffer.recorded_work = true;
		}
	}
}
