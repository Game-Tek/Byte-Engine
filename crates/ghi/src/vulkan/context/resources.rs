use super::*;

impl Context {
	pub(crate) fn get_image_subresource_layout(
		&self,
		texture: &graphics_hardware_interface::ImageHandle,
		mip_level: u32,
	) -> graphics_hardware_interface::ImageSubresourceLayout {
		let image_subresource = vk::ImageSubresource {
			aspect_mask: vk::ImageAspectFlags::COLOR,
			mip_level,
			array_layer: 0,
		};

		let texture = self.images.get(texture.0.0 as usize).expect("No texture with that handle.");

		if true
		/* TILING_OPTIMAL */
		{
			graphics_hardware_interface::ImageSubresourceLayout {
				offset: 0,
				size: texture.size,
				row_pitch: texture.extent.width() as usize * texture.format_.size(),
				array_pitch: texture.extent.width() as usize * texture.extent.height().max(1) as usize * texture.format_.size(),
				depth_pitch: texture.extent.width() as usize
					* texture.extent.height().max(1) as usize
					* texture.extent.depth().max(1) as usize
					* texture.format_.size(),
			}
		} else {
			let image_subresource_layout =
				unsafe { self.device.get_image_subresource_layout(texture.image, image_subresource) };
			graphics_hardware_interface::ImageSubresourceLayout {
				offset: image_subresource_layout.offset as usize,
				size: image_subresource_layout.size as usize,
				row_pitch: image_subresource_layout.row_pitch as usize,
				array_pitch: image_subresource_layout.array_pitch as usize,
				depth_pitch: image_subresource_layout.depth_pitch as usize,
			}
		}
	}

	pub(crate) fn bind_vulkan_buffer_memory(
		&self,
		info: &MemoryBackedResourceCreationResult<vk::Buffer>,
		allocation_handle: graphics_hardware_interface::AllocationHandle,
		offset: usize,
	) -> (u64, *mut u8) {
		let buffer = info.resource;
		let allocation = self
			.allocations
			.get(allocation_handle.0 as usize)
			.expect("No allocation with that handle.");
		unsafe {
			self.device
				.bind_buffer_memory(buffer, allocation.memory, offset as u64)
				.expect("No buffer memory binding")
		};
		unsafe {
			(
				self.device
					.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer)),
				allocation.pointer.0.add(offset),
			)
		}
	}

	pub(crate) fn bind_host_vulkan_buffer_memory(
		&self,
		info: &MemoryBackedResourceCreationResult<vk::Buffer>,
		allocation_handle: graphics_hardware_interface::AllocationHandle,
		offset: usize,
	) -> *mut u8 {
		let buffer = info.resource;
		let allocation = self
			.allocations
			.get(allocation_handle.0 as usize)
			.expect("No allocation with that handle.");
		unsafe {
			self.device
				.bind_buffer_memory(buffer, allocation.memory, offset as u64)
				.expect("No buffer memory binding")
		};
		unsafe { allocation.pointer.0.add(offset) }
	}

	/// Creates and maps one dedicated transfer-destination buffer without leaking partial Vulkan resources.
	pub(crate) fn create_texture_readback_buffer(
		&mut self,
		size: usize,
	) -> Result<(vk::Buffer, vk::DeviceMemory, *mut u8), crate::TextureTransferError> {
		let size = u64::try_from(size).map_err(|_| crate::TextureTransferError::UnsupportedLayout)?;
		let buffer_info = vk::BufferCreateInfo::default()
			.size(size)
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.usage(vk::BufferUsageFlags::TRANSFER_DST);
		let buffer = unsafe {
			self.device
				.create_buffer(&buffer_info, None)
				.map_err(|_| crate::TextureTransferError::AllocationFailed)?
		};
		let requirements = unsafe { self.device.get_buffer_memory_requirements(buffer) };
		let memory = self.allocate_memory_from_candidates(
			requirements.size,
			requirements.memory_type_bits,
			crate::DeviceAccesses::CpuRead,
			false,
		);
		let Some(memory) = memory else {
			unsafe { self.device.destroy_buffer(buffer, None) };
			return Err(crate::TextureTransferError::AllocationFailed);
		};
		if unsafe { self.device.bind_buffer_memory(buffer, memory, 0) }.is_err() {
			unsafe {
				self.device.free_memory(memory, None);
				self.device.destroy_buffer(buffer, None);
			}
			return Err(crate::TextureTransferError::AllocationFailed);
		}
		let pointer = match unsafe {
			self.device
				.map_memory(memory, 0, requirements.size, vk::MemoryMapFlags::empty())
		} {
			Ok(pointer) => pointer.cast::<u8>(),
			Err(_) => {
				unsafe {
					self.device.free_memory(memory, None);
					self.device.destroy_buffer(buffer, None);
				}
				return Err(crate::TextureTransferError::MappingFailed);
			}
		};

		Ok((buffer, memory, pointer))
	}

	pub(crate) fn bind_vulkan_texture_memory(
		&self,
		info: &MemoryBackedResourceCreationResult<vk::Image>,
		allocation_handle: graphics_hardware_interface::AllocationHandle,
		offset: usize,
	) -> (u64, *mut u8) {
		let image = info.resource;
		let allocation = self
			.allocations
			.get(allocation_handle.0 as usize)
			.expect("No allocation with that handle.");
		unsafe {
			self.device
				.bind_image_memory(image, allocation.memory, offset as u64)
				.expect("No image memory binding")
		};
		(0, unsafe { allocation.pointer.0.add(offset) })
	}

	/// Creates swapchain-backed image wrappers chained across frames and returns the root handle.
	pub(crate) fn create_swapchain_image(
		&mut self,
		vk_image: vk::Image,
		format: crate::Formats,
		uses: crate::Uses,
		image_usage_flags: vk::ImageUsageFlags,
		previous: Option<ImageHandle>,
	) -> ImageHandle {
		let root_handle = ImageHandle(self.images.len() as u64);
		let root_image = self.swapchain_image(vk_image, format, uses, image_usage_flags);

		if let Some(previous) = previous {
			self.images[previous.0 as usize].next = Some(root_handle);
		}

		self.images.push(root_image);

		root_handle
	}

	/// Wraps a presentable image that the swapchain owns, so it is never destroyed through the image list.
	pub(crate) fn swapchain_image(
		&self,
		vk_image: vk::Image,
		format: crate::Formats,
		uses: crate::Uses,
		image_usage_flags: vk::ImageUsageFlags,
	) -> Image {
		let image_views = vec![self.create_vulkan_image_view(
			None,
			&vk_image,
			vk::ImageType::TYPE_2D,
			format,
			image_usage_flags,
			1,
			0,
			None,
		)];

		Image {
			next: None,
			size: 0,
			staging_buffer: None,
			staging_allocation: None,
			allocation: None,
			pointer: None,
			image: vk_image,
			full_image_view: vk::ImageView::null(),
			image_views,
			extent: Extent::cube(0, 0, 0),
			access: crate::DeviceAccesses::DeviceOnly,
			format: to_format(format),
			format_: format,
			uses,
			layers: None,
			cube_compatible: false,
			cube_array_compatible: false,
			mip_levels: 1,
			owns_image: false,
		}
	}

	/// Allocates from the best memory type for `device_accesses`, moving to the next candidate when a heap is full.
	///
	/// Returns `None` when no memory type is compatible or every compatible type failed to allocate.
	pub(crate) fn allocate_memory_from_candidates(
		&self,
		size: u64,
		memory_type_bits: u32,
		device_accesses: crate::DeviceAccesses,
		device_address: bool,
	) -> Option<vk::DeviceMemory> {
		crate::vulkan::utils::memory_type_candidates(&self.memory_properties, memory_type_bits, device_accesses)
			.into_iter()
			.find_map(|memory_type_index| {
				let mut memory_allocate_flags_info =
					vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
				let mut memory_allocate_info = vk::MemoryAllocateInfo::default()
					.allocation_size(size)
					.memory_type_index(memory_type_index);
				if device_address {
					memory_allocate_info = memory_allocate_info.push(&mut memory_allocate_flags_info);
				}

				unsafe { self.device.allocate_memory(&memory_allocate_info, None) }.ok()
			})
	}

	/// Allocates memory from the device.
	pub(crate) fn create_allocation_internal(
		&mut self,
		size: usize,
		memory_bits: Option<u32>,
		device_accesses: crate::DeviceAccesses,
	) -> (graphics_hardware_interface::AllocationHandle, Option<*mut u8>) {
		// Allocations not tied to a resource yet may use any memory type.
		let memory_type_bits = memory_bits.unwrap_or(u32::MAX);
		let memory = self
			.allocate_memory_from_candidates(size as u64, memory_type_bits, device_accesses, true)
			.expect(
				"Failed to allocate Vulkan memory. The most likely cause is that every memory heap compatible with the resource is exhausted or the device allocation count limit was reached.",
			);

		let mut mapped_memory = None;

		if device_accesses.intersects(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite) {
			mapped_memory = Some(unsafe {
				self.device
					.map_memory(memory, 0, size as u64, vk::MemoryMapFlags::empty())
					.expect("No mapped memory") as *mut u8
			});
		}

		let allocation_handle = graphics_hardware_interface::AllocationHandle(self.allocations.len() as u64);

		self.allocations.push(Allocation {
			memory,
			pointer: crate::vulkan::MappedMemoryPointer(mapped_memory.unwrap_or(std::ptr::null_mut())),
		});

		(allocation_handle, mapped_memory)
	}

	pub(crate) fn uses_only_host_access(device_accesses: crate::DeviceAccesses) -> bool {
		device_accesses.intersects(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite)
			&& !device_accesses.intersects(crate::DeviceAccesses::GpuRead | crate::DeviceAccesses::GpuWrite)
	}

	/// Creates a Vulkan buffer, allocates memory for it, binds the memory, and returns the tracked buffer object.
	pub(crate) fn create_bound_buffer(
		&mut self,
		name: Option<&str>,
		size: usize,
		vk_usage_flags: vk::BufferUsageFlags,
		allocation_accesses: crate::DeviceAccesses,
		buffer_accesses: crate::DeviceAccesses,
		resource_uses: crate::Uses,
	) -> Buffer {
		let buffer_creation_result = self.create_vulkan_buffer(name, size, vk_usage_flags);
		let (allocation_handle, _) = self.create_allocation_internal(
			buffer_creation_result.size,
			buffer_creation_result.memory_flags.into(),
			allocation_accesses,
		);
		let (device_address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);
		if size != 0 && !pointer.is_null() {
			// Typed buffer APIs expose mapped storage as zeroable POD values, so initialize the complete representation.
			unsafe { std::ptr::write_bytes(pointer, 0, size) };
		}

		Buffer {
			staging: None,
			source: None,
			buffer: buffer_creation_result.resource,
			size,
			device_address,
			pointer: crate::vulkan::MappedMemoryPointer(pointer),
			allocation: Some(allocation_handle),
			uses: resource_uses,
			access: buffer_accesses,
		}
	}

	/// Builds the host-visible buffer that carries CPU reads and writes for a GPU buffer with `device_accesses`.
	fn build_host_staging_buffer(
		&mut self,
		name: Option<&str>,
		size: usize,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
	) -> Buffer {
		let vk_usage_flags = if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
			vk::BufferUsageFlags::TRANSFER_DST
		} else {
			vk::BufferUsageFlags::empty()
		} | if device_accesses.intersects(crate::DeviceAccesses::CpuWrite) {
			vk::BufferUsageFlags::TRANSFER_SRC
		} else {
			vk::BufferUsageFlags::empty()
		} | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;

		let device_access = if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
			crate::DeviceAccesses::GpuWrite | crate::DeviceAccesses::CpuRead
		} else {
			crate::DeviceAccesses::empty()
		} | if device_accesses.intersects(crate::DeviceAccesses::CpuWrite) {
			crate::DeviceAccesses::GpuRead | crate::DeviceAccesses::CpuWrite
		} else {
			crate::DeviceAccesses::empty()
		};

		// The staging allocation itself needs host properties only; GPU access describes how commands use the buffer.
		let allocation_accesses = device_accesses & (crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite);
		self.create_bound_buffer(name, size, vk_usage_flags, allocation_accesses, device_access, resource_uses)
	}

	/// Builds a buffer object with the given name, resource uses, size, Vulkan buffer usage flags, and device accesses.
	///
	/// Buffers that request only host access are created as a single mapped Vulkan buffer. Buffers that include GPU
	/// access and CPU access keep a separate host-visible staging buffer so transfers can synchronize CPU writes with
	/// GPU-visible storage.
	pub(crate) fn build_buffer_internal(
		&mut self,
		_next: Option<BufferHandle>,
		name: Option<&str>,
		resource_uses: crate::Uses,
		size: usize,
		device_accesses: crate::DeviceAccesses,
	) -> Buffer {
		if size == 0 {
			return Buffer {
				staging: None,
				source: None,
				buffer: vk::Buffer::null(),
				size: 0,
				device_address: 0,
				pointer: crate::vulkan::MappedMemoryPointer(std::ptr::null_mut()),
				allocation: None,
				uses: resource_uses,
				access: device_accesses,
			};
		}

		let vk_usage_flags = uses_to_vk_usage_flags(resource_uses);

		// Remove acceleration structure usage flags if ray tracing is disabled (causes validation errors)
		let vk_usage_flags = if !self.settings.ray_tracing {
			vk_usage_flags & !vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
		} else {
			vk_usage_flags
		};

		// Add shader device address usage flag as all buffers are guaranteed to be accessible by addressing
		let vk_usage_flags = vk_usage_flags | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;

		let vk_usage_flags = vk_usage_flags
			| if device_accesses.intersects(crate::DeviceAccesses::CpuWrite) {
				vk::BufferUsageFlags::TRANSFER_DST
			} else {
				vk::BufferUsageFlags::empty()
			} | if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
			vk::BufferUsageFlags::TRANSFER_SRC
		} else {
			vk::BufferUsageFlags::empty()
		};

		if Self::uses_only_host_access(device_accesses) {
			return self.create_bound_buffer(name, size, vk_usage_flags, device_accesses, device_accesses, resource_uses);
		}

		let mut buffer = self.create_bound_buffer(
			name,
			size,
			vk_usage_flags,
			device_accesses & !(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite),
			device_accesses,
			resource_uses,
		);

		let staging = if device_accesses.intersects(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite) {
			let staging_buffer = self.build_host_staging_buffer(name, size, resource_uses, device_accesses);

			let (_, handle) = self.buffers.add(staging_buffer);

			Some(handle)
		} else {
			None
		};

		buffer.staging = staging;
		buffer
	}

	/// Builds a buffer and returns its handle.
	pub(crate) fn create_buffer_internal(
		&mut self,
		next: Option<BufferHandle>,
		previous: Option<BufferHandle>,
		name: Option<&str>,
		resource_uses: crate::Uses,
		size: usize,
		device_accesses: crate::DeviceAccesses,
	) -> BufferHandle {
		let buffer = self.build_buffer_internal(next, name, resource_uses, size, device_accesses);

		let (_, handle) = self.buffers.add(buffer);

		if let Some(previous) = previous {
			self.buffers.set_next(previous, Some(handle));
		}

		self.buffers.set_next(handle, next);

		handle
	}

	/// Creates a CPU-visible staging buffer (TRANSFER_SRC) for use as a per-frame
	/// staging buffer in the persistent write mode. Returns its handle.
	pub(crate) fn create_staging_buffer(&mut self, name: Option<&str>, size: usize) -> BufferHandle {
		let vk_usage_flags = vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
		let device_access = crate::DeviceAccesses::GpuRead | crate::DeviceAccesses::CpuWrite;

		let buffer = self.create_bound_buffer(name, size, vk_usage_flags, device_access, device_access, crate::Uses::empty());
		let (_, handle) = self.buffers.add(buffer);

		handle
	}

	pub(crate) fn build_image_internal(
		&mut self,
		next: Option<ImageHandle>,
		name: Option<&str>,
		format: crate::Formats,
		device_accesses: crate::DeviceAccesses,
		array_layers: Option<NonZeroU32>,
		cube_compatible: bool,
		cube_array_compatible: bool,
		extent: Extent,
		resource_uses: crate::Uses,
		mip_levels: u32,
	) -> Image {
		// Every array layer has a complete image payload in the shared staging buffer.
		let layer_count = array_layers.map_or(1, NonZeroU32::get) as usize;
		let size = extent.width() as usize
			* extent.height().max(1) as usize
			* extent.depth().max(1) as usize
			* format.size()
			* layer_count;

		if extent.width() == 0 {
			return Image {
				next,
				size: 0,
				staging_buffer: None,
				staging_allocation: None,
				allocation: None,
				pointer: None,
				image: vk::Image::null(),
				full_image_view: vk::ImageView::null(),
				image_views: Vec::new(),
				extent,
				access: device_accesses,
				format: to_format(format),
				format_: format,
				uses: resource_uses,
				layers: array_layers,
				cube_compatible,
				cube_array_compatible,
				mip_levels,
				owns_image: true,
			};
		}

		let transfer_uses = (if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
			crate::Uses::TransferSource
		} else {
			crate::Uses::empty()
		}) | (if device_accesses.intersects(crate::DeviceAccesses::CpuWrite) {
			crate::Uses::TransferDestination
		} else {
			crate::Uses::empty()
		});

		let texture_creation_result = self.create_vulkan_texture(
			name,
			extent,
			format,
			resource_uses | transfer_uses,
			mip_levels,
			array_layers,
			cube_compatible,
			cube_array_compatible,
		);

		let uses_cpu_staging = device_accesses.intersects(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite);

		let m_device_accesses = if uses_cpu_staging {
			crate::DeviceAccesses::DeviceOnly
		} else {
			device_accesses
		};

		let (image_allocation, _) = self.create_allocation_internal(
			texture_creation_result.size,
			texture_creation_result.memory_flags.into(),
			m_device_accesses,
		);

		let _ = self.bind_vulkan_texture_memory(&texture_creation_result, image_allocation, 0);

		let (staging_buffer, staging_allocation, pointer) = if uses_cpu_staging {
			// A staging buffer may serve both readback and upload when the image allows both CPU access modes.
			let vk_buffer_usage_flags = (if device_accesses.contains(crate::DeviceAccesses::CpuRead) {
				vk::BufferUsageFlags::TRANSFER_DST
			} else {
				vk::BufferUsageFlags::empty()
			}) | (if device_accesses.contains(crate::DeviceAccesses::CpuWrite) {
				vk::BufferUsageFlags::TRANSFER_SRC
			} else {
				vk::BufferUsageFlags::empty()
			});
			// Preserve both host access directions so allocation selects visible memory and coherent uploads.
			let allocation_accesses = device_accesses & (crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite);

			let buffer_creation_result = self.create_vulkan_buffer(name, size, vk_buffer_usage_flags);
			let (allocation_handle, _) = self.create_allocation_internal(
				buffer_creation_result.size,
				buffer_creation_result.memory_flags.into(),
				allocation_accesses,
			);
			let pointer = self.bind_host_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

			(
				Some(buffer_creation_result.resource),
				Some(allocation_handle),
				Some(crate::vulkan::MappedMemoryPointer(pointer)),
			)
		} else {
			(None, None, None)
		};

		let image_usage_flags = into_vk_image_usage_flags(resource_uses | transfer_uses, format);
		// Vulkan only allows image views for images created with view-capable usage bits.
		// Transfer-only staging/readback images intentionally keep null views.
		let image_can_have_views = InnerDevice::image_usage_allows_views(image_usage_flags);
		let image_type = crate::vulkan::utils::image_type_from_extent(extent).expect("Failed to get VkImageType from extent");

		let full_image_view = image_can_have_views
			.then(|| {
				array_layers.map(|layers| {
					self.create_vulkan_image_view(
						name,
						&texture_creation_result.resource,
						image_type,
						format,
						image_usage_flags,
						mip_levels,
						0,
						Some(layers),
					)
				})
			})
			.flatten();

		let image_views = if image_can_have_views {
			let mut image_views = Vec::with_capacity(array_layers.map_or(1, NonZeroU32::get) as usize);

			if let Some(l) = array_layers.map(|e| e.get()) {
				for i in 0..l {
					image_views.push(self.create_vulkan_image_view(
						name,
						&texture_creation_result.resource,
						image_type,
						format,
						image_usage_flags,
						mip_levels,
						i,
						NonZeroU32::new(1),
					));
				}
			} else {
				image_views.push(self.create_vulkan_image_view(
					name,
					&texture_creation_result.resource,
					image_type,
					format,
					image_usage_flags,
					mip_levels,
					0,
					None,
				));
			}

			image_views
		} else {
			Vec::new()
		};

		Image {
			next,
			size,
			staging_buffer,
			staging_allocation,
			allocation: Some(image_allocation),
			pointer,
			image: texture_creation_result.resource,
			full_image_view: full_image_view.unwrap_or(vk::ImageView::null()),
			image_views,
			extent,
			access: device_accesses,
			format: to_format(format),
			format_: format,
			uses: resource_uses,
			layers: array_layers,
			cube_compatible,
			cube_array_compatible,
			mip_levels,
			owns_image: true,
		}
	}

	pub(crate) fn create_image_internal(
		&mut self,
		next: Option<ImageHandle>,
		previous: Option<ImageHandle>,
		name: Option<&str>,
		format: crate::Formats,
		device_accesses: crate::DeviceAccesses,
		array_layers: Option<NonZeroU32>,
		cube_compatible: bool,
		cube_array_compatible: bool,
		extent: Extent,
		resource_uses: crate::Uses,
		mip_levels: u32,
	) -> ImageHandle {
		let texture_handle = ImageHandle(self.images.len() as u64);

		let image = self.build_image_internal(
			next,
			name,
			format,
			device_accesses,
			array_layers,
			cube_compatible,
			cube_array_compatible,
			extent,
			resource_uses,
			mip_levels,
		);

		if let Some(previous) = previous {
			self.images[previous.0 as usize].next = Some(texture_handle);
		}

		self.images.push(image);

		texture_handle
	}

	pub(crate) fn create_synchronizer_internal(&mut self, name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		let synchronizer_handle = SynchronizerHandle(self.synchronizers.len() as u64);

		self.synchronizers.push(Synchronizer {
			next: None,
			signaled,
			armed: signaled,
			fence: self.create_vulkan_fence(signaled),
			semaphore: self.create_vulkan_semaphore(name, signaled),
		});

		synchronizer_handle
	}

	/// Grows every frame copy of a dynamic buffer, its staging, and its persistent source to `size`.
	///
	/// Contents are discarded, matching the other backends. Replaced storage is destroyed only after the frames that
	/// may still read it have completed, so the resize is safe while earlier frames are in flight.
	pub(crate) fn resize_buffer_internal(&mut self, buffer_handle: BufferHandle, size: usize) {
		if self.buffers.resource(buffer_handle).size >= size {
			return;
		}

		let master_handle = graphics_hardware_interface::BaseBufferHandle::new(buffer_handle.0);
		let name = self.get_object_debug_name(master_handle.into());
		let name = name.as_deref();

		// Copies for later sequences may not exist yet; their pending build tasks copy the master's new size.
		let mut frame_copies = SmallVec::<[BufferHandle; MAX_FRAMES_IN_FLIGHT]>::new();
		for sequence_index in 0..self.frames as usize {
			let handle = self
				.buffers
				.nth_handle(master_handle, sequence_index)
				.expect("Missing Vulkan dynamic buffer. The most likely cause is that the handle came from another context.");
			if !frame_copies.contains(&handle) {
				frame_copies.push(handle);
			}
		}

		let mut persistent_source = None;
		for handle in frame_copies {
			let current = *self.buffers.resource(handle);
			let mut replacement = self.build_buffer_internal(None, name, current.uses, size, current.access);
			if let Some(source_handle) = current.source {
				persistent_source = Some((source_handle, current.access));
				replacement.source = Some(source_handle);
			}

			if let Some(staging_handle) = current.staging {
				self.retire_buffer_storage(staging_handle);
			}
			self.retire_buffer_storage(handle);
			*self.buffers.resource_mut(handle) = replacement;

			// The replacement has no GPU history; stale ranges would only add barriers against the retired buffer.
			self.states.remove(&crate::vulkan::Handles::Buffer(handle));
			self.buffer_states.remove(&crate::vulkan::Handles::Buffer(handle));
		}

		// Pending build tasks captured the shared source handle, so it is replaced in place rather than reallocated.
		if let Some((source_handle, device_accesses)) = persistent_source {
			let uses = self.buffers.resource(source_handle).uses;
			let replacement = self.build_host_staging_buffer(name, size, uses, device_accesses);
			self.retire_buffer_storage(source_handle);
			*self.buffers.resource_mut(source_handle) = replacement;
		}

		for sequence_index in 0..self.frames {
			self.bump_descriptor_sequence_epoch(sequence_index);
		}
	}

	pub(crate) fn resize_image_internal(&mut self, image_handle: ImageHandle, extent: Extent, sequence_index: u8) {
		let image = image_handle.access(&self.images);
		if !image.owns_image || image.extent == extent {
			return;
		}

		let root_handle = image_handle.root(&self.images);
		let name = self.get_object_debug_name(
			graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(root_handle.0)).into(),
		);

		let image = self.images[image_handle.0 as usize].clone();
		let new_image = self.build_image_internal(
			image.next,
			name.as_deref(),
			image.format_,
			image.access,
			image.layers,
			image.cube_compatible,
			image.cube_array_compatible,
			extent,
			image.uses,
			image.mip_levels,
		);

		self.images[image_handle.0 as usize] = new_image;
		self.retire_image_storage(&image);

		if let Some(state) = self.states.get_mut(&crate::vulkan::Handles::Image(image_handle)) {
			state.layout = vk::ImageLayout::UNDEFINED;
		}

		// A static image is one instance shared by every sequence, so every sequence's snapshots may hold its old views.
		if root_handle.get_all(&self.images).len() == 1 {
			for sequence_index in 0..self.frames {
				self.bump_descriptor_sequence_epoch(sequence_index);
			}
		} else {
			self.bump_descriptor_sequence_epoch(sequence_index);
		}
	}

	/// Queues a destruction for the first task pass after every frame started so far has completed on the GPU.
	///
	/// Frames that already started may have recorded or submitted work that references the object. Before the first
	/// frame, work submitted outside frames is covered by waiting for frame 0, whose fence orders all earlier submissions.
	pub(crate) fn defer_destruction(&mut self, task: Tasks) {
		self.tasks
			.push(Task::after_frame(task, self.last_started_frame.unwrap_or(0)));
	}

	/// Retires a buffer's Vulkan object and memory, leaving its entry empty so it is never destroyed twice.
	pub(crate) fn retire_buffer_storage(&mut self, handle: BufferHandle) {
		let buffer = self.buffers.resource_mut(handle);
		let vk_buffer = std::mem::replace(&mut buffer.buffer, vk::Buffer::null());
		let allocation = buffer.allocation.take();
		buffer.pointer = crate::vulkan::MappedMemoryPointer(std::ptr::null_mut());
		buffer.size = 0;
		buffer.device_address = 0;

		if !vk_buffer.is_null() {
			self.defer_destruction(Tasks::DeleteVulkanBuffer { handle: vk_buffer });
		}
		if let Some(allocation) = allocation {
			self.defer_destruction(Tasks::FreeAllocation { handle: allocation });
		}
	}

	/// Retires every Vulkan object and allocation of a replaced image, views first so none outlives its image.
	pub(crate) fn retire_image_storage(&mut self, image: &Image) {
		for &image_view in image.image_views.iter().chain(std::iter::once(&image.full_image_view)) {
			if !image_view.is_null() {
				self.defer_destruction(Tasks::DeleteVulkanImageView { handle: image_view });
			}
		}
		if image.owns_image && !image.image.is_null() {
			self.defer_destruction(Tasks::DeleteVulkanImage { handle: image.image });
		}
		if let Some(staging_buffer) = image.staging_buffer {
			self.defer_destruction(Tasks::DeleteVulkanBuffer { handle: staging_buffer });
		}
		for allocation in [image.allocation, image.staging_allocation].into_iter().flatten() {
			self.defer_destruction(Tasks::FreeAllocation { handle: allocation });
		}
	}

	/// Destroys one retired object. Returns `false` for tasks that are not destructions.
	pub(crate) fn run_destruction_task(&mut self, task: &Tasks) -> bool {
		unsafe {
			match *task {
				Tasks::DeleteVulkanImage { handle } => self.device.destroy_image(handle, None),
				Tasks::DeleteVulkanImageView { handle } => self.device.destroy_image_view(handle, None),
				Tasks::DeleteVulkanBuffer { handle } => self.device.destroy_buffer(handle, None),
				Tasks::FreeAllocation { handle } => {
					let allocation = &mut self.allocations[handle.0 as usize];
					let memory = std::mem::replace(&mut allocation.memory, vk::DeviceMemory::null());
					allocation.pointer = crate::vulkan::MappedMemoryPointer(std::ptr::null_mut());
					if !memory.is_null() {
						self.device.free_memory(memory, None);
					}
				}
				_ => return false,
			}
		}
		true
	}

	/// Add the task to all frames
	pub(crate) fn add_task_to_all_frames(&mut self, tasks: Tasks) {
		for i in 0..self.frames {
			self.tasks.push(Task::new(tasks, Some(i)));
		}
	}

	/// Add the task to all other frames but the current frame.
	pub(crate) fn add_task_to_all_other_frames(&mut self, tasks: Tasks, current_frame: u8) {
		for i in 1..self.frames {
			// Skip current frame
			let i = current_frame + i; // Offset by current frame
			let i = i.rem_euclid(self.frames); // Wrap around frames
			self.tasks.push(Task::new(tasks, Some(i)));
		}
	}
}
