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
				allocation.pointer.add(offset),
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
		unsafe { allocation.pointer.add(offset) }
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
		let memory_type_index = self
			.memory_properties
			.memory_types
			.iter()
			.enumerate()
			.find_map(|(index, memory_type)| {
				let supported = requirements.memory_type_bits & (1 << index) != 0;
				let visible = memory_type.property_flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE);
				(supported && visible).then_some(index as u32)
			});
		let Some(memory_type_index) = memory_type_index else {
			unsafe { self.device.destroy_buffer(buffer, None) };
			return Err(crate::TextureTransferError::AllocationFailed);
		};
		let allocation_info = vk::MemoryAllocateInfo::default()
			.allocation_size(requirements.size)
			.memory_type_index(memory_type_index);
		let memory = match unsafe { self.device.allocate_memory(&allocation_info, None) } {
			Ok(memory) => memory,
			Err(_) => {
				unsafe { self.device.destroy_buffer(buffer, None) };
				return Err(crate::TextureTransferError::AllocationFailed);
			}
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
		(0, unsafe { allocation.pointer.add(offset) })
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
		let root_image = {
			let image_views = vec![self.create_vulkan_image_view(None, &vk_image, format, image_usage_flags, 1, 0, None)];

			Image {
				next: None,
				size: 0,
				staging_buffer: None,
				staging_allocation: None,
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
		};

		if let Some(previous) = previous {
			self.images[previous.0 as usize].next = Some(root_handle);
		}

		self.images.push(root_image);

		root_handle
	}

	/// Allocates memory from the device.
	pub(crate) fn create_allocation_internal(
		&mut self,
		size: usize,
		memory_bits: Option<u32>,
		device_accesses: crate::DeviceAccesses,
	) -> (graphics_hardware_interface::AllocationHandle, Option<*mut u8>) {
		let memory_property_flags = {
			let mut memory_property_flags = vk::MemoryPropertyFlags::empty();

			memory_property_flags |=
				if device_accesses.intersects(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite) {
					vk::MemoryPropertyFlags::HOST_VISIBLE
				} else {
					vk::MemoryPropertyFlags::empty()
				};
			memory_property_flags |= if device_accesses.contains(crate::DeviceAccesses::CpuWrite) {
				vk::MemoryPropertyFlags::HOST_COHERENT
			} else {
				vk::MemoryPropertyFlags::empty()
			};
			memory_property_flags |= if device_accesses.contains(crate::DeviceAccesses::GpuRead) {
				vk::MemoryPropertyFlags::DEVICE_LOCAL
			} else {
				vk::MemoryPropertyFlags::empty()
			};
			memory_property_flags |= if device_accesses.contains(crate::DeviceAccesses::GpuWrite) {
				vk::MemoryPropertyFlags::DEVICE_LOCAL
			} else {
				vk::MemoryPropertyFlags::empty()
			};

			memory_property_flags
		};

		let memory_properties = &self.memory_properties;

		let memory_type_index = memory_properties
			.memory_types
			.iter()
			.enumerate()
			.find_map(|(index, memory_type)| {
				let memory_type = memory_type.property_flags.contains(memory_property_flags);

				if (memory_bits.unwrap_or(0) & (1 << index)) != 0 && memory_type {
					Some(index as u32)
				} else {
					None
				}
			})
			.expect("No memory type index found.");

		let mut memory_allocate_flags_info =
			vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);

		let memory_allocate_info = vk::MemoryAllocateInfo::default()
			.allocation_size(size as u64)
			.memory_type_index(memory_type_index)
			.push(&mut memory_allocate_flags_info);

		let memory = unsafe { self.device.allocate_memory(&memory_allocate_info, None).expect("No memory") };

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
			pointer: mapped_memory.unwrap_or(std::ptr::null_mut()),
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

		Buffer {
			staging: None,
			source: None,
			buffer: buffer_creation_result.resource,
			size,
			device_address,
			pointer,
			uses: resource_uses,
			access: buffer_accesses,
		}
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
				pointer: std::ptr::null_mut(),
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
			let staging_buffer =
				self.create_bound_buffer(name, size, vk_usage_flags, allocation_accesses, device_access, resource_uses);

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

		let (allocation_handle, _) = self.create_allocation_internal(
			texture_creation_result.size,
			texture_creation_result.memory_flags.into(),
			m_device_accesses,
		);

		let _ = self.bind_vulkan_texture_memory(&texture_creation_result, allocation_handle, 0);

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

			(Some(buffer_creation_result.resource), Some(allocation_handle), Some(pointer))
		} else {
			(None, None, None)
		};

		let image_usage_flags = into_vk_image_usage_flags(resource_uses | transfer_uses, format);
		// Vulkan only allows image views for images created with view-capable usage bits.
		// Transfer-only staging/readback images intentionally keep null views.
		let image_can_have_views = InnerDevice::image_usage_allows_views(image_usage_flags);

		let full_image_view = image_can_have_views
			.then(|| {
				array_layers.map(|layers| {
					self.create_vulkan_image_view(
						name,
						&texture_creation_result.resource,
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
			fence: self.create_vulkan_fence(signaled),
			semaphore: self.create_vulkan_semaphore(name, signaled),
		});

		synchronizer_handle
	}

	pub(crate) fn resize_buffer_internal(&mut self, buffer_handle: BufferHandle, size: usize) {
		let current_buffer = self.buffers.resource(buffer_handle);

		if current_buffer.size >= size {
			return;
		}

		assert!(current_buffer.staging.is_none(), "Cannot resize buffers with staging buffers");

		if current_buffer.size != 0 {
			let current_vk_buffer = current_buffer.buffer;

			self.tasks.push(Task::delete_vulkan_buffer(current_vk_buffer, None));

			// todo!("copy data from old buffer to new buffer");
		}

		let new_buffer = self.build_buffer_internal(
			None,
			None,
			current_buffer.uses,
			size,
			crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		);

		*self.buffers.resource_mut(buffer_handle) = new_buffer;
		for sequence_index in 0..self.frames {
			self.bump_descriptor_sequence_epoch(sequence_index);
		}
	}

	pub(crate) fn resize_image_internal(&mut self, image_handle: ImageHandle, extent: Extent, sequence_index: u8) {
		let name = self.get_object_debug_name(
			graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(
				image_handle.root(&self.images).0,
			))
			.into(),
		);

		let image = image_handle.access(&self.images);

		if !image.owns_image {
			return;
		}

		if image.extent == extent {
			// Requested extent matches current extent, no resize needed
			return;
		}

		if let Some(staging_buffer_handle) = image.staging_buffer {
			self.tasks
				.push(Task::delete_vulkan_buffer(staging_buffer_handle, Some(sequence_index)));
		}

		for &image_view in &image.image_views {
			if !image_view.is_null() {
				self.tasks.push(Task::delete_vulkan_image_view(image_view, sequence_index));
			}
		}

		if !image.full_image_view.is_null() {
			self.tasks
				.push(Task::delete_vulkan_image_view(image.full_image_view, sequence_index));
		}

		self.tasks.push(Task::delete_vulkan_image(image.image, sequence_index));

		// TODO: release memory/allocation

		let new_image = self.build_image_internal(
			image.next,
			name.as_ref().map(|e| e.as_str()),
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

		if let Some(state) = self.states.get_mut(&crate::vulkan::Handles::Image(image_handle)) {
			state.layout = vk::ImageLayout::UNDEFINED;
		}

		self.bump_descriptor_sequence_epoch(sequence_index);
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
