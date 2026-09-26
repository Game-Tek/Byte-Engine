use super::*;
use crate::DeviceAccesses;

/// Returns the transfer usage of a buffer that is a copy source and/or a copy destination.
fn transfer_usage(source: bool, destination: bool) -> vk::BufferUsageFlags {
	let usage_if = |enabled, usage| if enabled { usage } else { vk::BufferUsageFlags::empty() };
	usage_if(source, vk::BufferUsageFlags::TRANSFER_SRC) | usage_if(destination, vk::BufferUsageFlags::TRANSFER_DST)
}

/// Describes an owned image without native storage or views.
fn unbacked_image(format: crate::Formats, uses: crate::Uses, extent: Extent, access: DeviceAccesses) -> Image {
	Image {
		next: None,
		size: 0,
		staging_buffer: None,
		staging_allocation: None,
		allocation: None,
		pointer: None,
		image: vk::Image::null(),
		full_image_view: vk::ImageView::null(),
		image_views: Vec::new(),
		extent,
		access,
		format: to_format(format),
		format_: format,
		uses,
		layers: None,
		cube_compatible: false,
		cube_array_compatible: false,
		mip_levels: 1,
		owns_image: true,
	}
}

impl Context {
	fn allocation(&self, allocation_handle: graphics_hardware_interface::AllocationHandle) -> &Allocation {
		self.allocations
			.get(allocation_handle.0 as usize)
			.expect("No allocation with that handle.")
	}

	pub(crate) fn bind_vulkan_buffer_memory(
		&self,
		info: &MemoryBackedResourceCreationResult<vk::Buffer>,
		allocation_handle: graphics_hardware_interface::AllocationHandle,
		offset: usize,
	) -> (u64, *mut u8) {
		let pointer = self.bind_host_vulkan_buffer_memory(info, allocation_handle, offset);
		let address = unsafe {
			self.device
				.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(info.resource))
		};
		(address, pointer)
	}

	/// Binds buffer memory without querying a device address, for buffers created without device-address usage.
	pub(crate) fn bind_host_vulkan_buffer_memory(
		&self,
		info: &MemoryBackedResourceCreationResult<vk::Buffer>,
		allocation_handle: graphics_hardware_interface::AllocationHandle,
		offset: usize,
	) -> *mut u8 {
		let allocation = self.allocation(allocation_handle);
		unsafe {
			self.device
				.bind_buffer_memory(info.resource, allocation.memory, offset as u64)
				.expect("No buffer memory binding");
			allocation.pointer.0.add(offset)
		}
	}

	/// Creates and maps one dedicated transfer-destination buffer without leaking partial Vulkan resources.
	pub(crate) fn create_texture_readback_buffer(
		&mut self,
		size: usize,
	) -> Result<(vk::Buffer, vk::DeviceMemory, *mut u8), crate::TextureTransferError> {
		use crate::TextureTransferError::{AllocationFailed, MappingFailed};

		let size = u64::try_from(size).map_err(|_| crate::TextureTransferError::UnsupportedLayout)?;
		let buffer_info = vk::BufferCreateInfo::default()
			.size(size)
			.sharing_mode(vk::SharingMode::EXCLUSIVE)
			.usage(vk::BufferUsageFlags::TRANSFER_DST);
		let device = &self.device;
		let buffer = unsafe { device.create_buffer(&buffer_info, None) }.map_err(|_| AllocationFailed)?;
		let fail = |memory: Option<vk::DeviceMemory>, error| {
			unsafe {
				if let Some(memory) = memory {
					device.free_memory(memory, None);
				}
				device.destroy_buffer(buffer, None);
			}
			error
		};
		let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
		let memory = self
			.allocate_memory_from_candidates(
				requirements.size,
				requirements.memory_type_bits,
				DeviceAccesses::CpuRead,
				false,
			)
			.ok_or_else(|| fail(None, AllocationFailed))?;
		unsafe { device.bind_buffer_memory(buffer, memory, 0) }.map_err(|_| fail(Some(memory), AllocationFailed))?;
		let pointer = unsafe { device.map_memory(memory, 0, requirements.size, vk::MemoryMapFlags::empty()) }
			.map_err(|_| fail(Some(memory), MappingFailed))?;

		Ok((buffer, memory, pointer.cast::<u8>()))
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
		let image_views =
			vec![self.create_vulkan_image_view(None, &vk_image, vk::ImageType::TYPE_2D, format, image_usage_flags, 1, 0, None)];

		Image {
			image: vk_image,
			image_views,
			owns_image: false,
			..unbacked_image(format, uses, Extent::cube(0, 0, 0), DeviceAccesses::DeviceOnly)
		}
	}

	/// Allocates from the best memory type for `device_accesses`, moving to the next candidate when a heap is full.
	///
	/// Returns `None` when no memory type is compatible or every compatible type failed to allocate.
	pub(crate) fn allocate_memory_from_candidates(
		&self,
		size: u64,
		memory_type_bits: u32,
		device_accesses: DeviceAccesses,
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
		device_accesses: DeviceAccesses,
	) -> (graphics_hardware_interface::AllocationHandle, Option<*mut u8>) {
		// Allocations not tied to a resource yet may use any memory type.
		let memory_type_bits = memory_bits.unwrap_or(u32::MAX);
		let memory = self
			.allocate_memory_from_candidates(size as u64, memory_type_bits, device_accesses, true)
			.expect(
				"Failed to allocate Vulkan memory. The most likely cause is that every memory heap compatible with the resource is exhausted or the device allocation count limit was reached.",
			);

		let mapped_memory = device_accesses.intersects(DeviceAccesses::HostOnly).then(|| unsafe {
			self.device
				.map_memory(memory, 0, size as u64, vk::MemoryMapFlags::empty())
				.expect("No mapped memory")
				.cast::<u8>()
		});

		let allocation_handle = graphics_hardware_interface::AllocationHandle(self.allocations.len() as u64);
		self.allocations.push(Allocation {
			memory,
			pointer: crate::vulkan::MappedMemoryPointer(mapped_memory.unwrap_or(std::ptr::null_mut())),
		});

		(allocation_handle, mapped_memory)
	}

	pub(crate) fn uses_only_host_access(device_accesses: DeviceAccesses) -> bool {
		device_accesses.intersects(DeviceAccesses::HostOnly) && !device_accesses.intersects(DeviceAccesses::DeviceOnly)
	}

	/// Creates a Vulkan buffer, allocates memory for it, binds the memory, and returns the tracked buffer object.
	/// Creates a Vulkan buffer bound to its own new allocation and returns it with that allocation, its device address and
	/// its mapping.
	pub(crate) fn create_dedicated_buffer(
		&mut self,
		name: Option<&str>,
		size: usize,
		usage: vk::BufferUsageFlags,
		device_accesses: crate::DeviceAccesses,
	) -> (
		MemoryBackedResourceCreationResult<vk::Buffer>,
		graphics_hardware_interface::AllocationHandle,
		u64,
		*mut u8,
	) {
		let buffer = self.create_vulkan_buffer(name, size, usage);
		let (allocation_handle, _) = self.create_allocation_internal(buffer.size, buffer.memory_flags.into(), device_accesses);
		let (address, pointer) = self.bind_vulkan_buffer_memory(&buffer, allocation_handle, 0);
		(buffer, allocation_handle, address, pointer)
	}

	pub(crate) fn create_bound_buffer(
		&mut self,
		name: Option<&str>,
		size: usize,
		vk_usage_flags: vk::BufferUsageFlags,
		allocation_accesses: DeviceAccesses,
		buffer_accesses: DeviceAccesses,
		resource_uses: crate::Uses,
	) -> Buffer {
		let (buffer_creation_result, allocation_handle, device_address, pointer) =
			self.create_dedicated_buffer(name, size, vk_usage_flags, allocation_accesses);
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
		device_accesses: DeviceAccesses,
	) -> Buffer {
		let cpu_read = device_accesses.contains(DeviceAccesses::CpuRead);
		let cpu_write = device_accesses.contains(DeviceAccesses::CpuWrite);
		let usage = transfer_usage(cpu_write, cpu_read) | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
		let mut buffer_accesses = DeviceAccesses::empty();
		buffer_accesses.set(DeviceAccesses::DeviceToHost, cpu_read);
		buffer_accesses.set(DeviceAccesses::HostToDevice, cpu_write);

		// The staging allocation itself needs host properties only; GPU access describes how commands use the buffer.
		let allocation_accesses = device_accesses & DeviceAccesses::HostOnly;
		self.create_bound_buffer(name, size, usage, allocation_accesses, buffer_accesses, resource_uses)
	}

	/// Builds a buffer object with the given name, resource uses, size, Vulkan buffer usage flags, and device accesses.
	///
	/// Buffers that request only host access are created as a single mapped Vulkan buffer. Buffers that include GPU
	/// access and CPU access keep a separate host-visible staging buffer so transfers can synchronize CPU writes with
	/// GPU-visible storage.
	pub(crate) fn build_buffer_internal(
		&mut self,
		name: Option<&str>,
		resource_uses: crate::Uses,
		size: usize,
		device_accesses: DeviceAccesses,
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

		let cpu_read = device_accesses.contains(DeviceAccesses::CpuRead);
		let cpu_write = device_accesses.contains(DeviceAccesses::CpuWrite);
		// All buffers are guaranteed to be accessible by device address.
		let mut usage = uses_to_vk_usage_flags(resource_uses)
			| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
			| transfer_usage(cpu_read, cpu_write);
		// Acceleration structure build input usage causes validation errors when ray tracing is disabled.
		if !self.settings.ray_tracing {
			usage &= !vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
		}

		if Self::uses_only_host_access(device_accesses) {
			return self.create_bound_buffer(name, size, usage, device_accesses, device_accesses, resource_uses);
		}

		let allocation_accesses = device_accesses - DeviceAccesses::HostOnly;
		let mut buffer = self.create_bound_buffer(name, size, usage, allocation_accesses, device_accesses, resource_uses);

		if device_accesses.intersects(DeviceAccesses::HostOnly) {
			let staging_buffer = self.build_host_staging_buffer(name, size, resource_uses, device_accesses);
			buffer.staging = Some(self.buffers.add(staging_buffer).1);
		}

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
		device_accesses: DeviceAccesses,
	) -> BufferHandle {
		let buffer = self.build_buffer_internal(name, resource_uses, size, device_accesses);
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
		let device_access = DeviceAccesses::HostToDevice;
		let buffer = self.create_bound_buffer(name, size, vk_usage_flags, device_access, device_access, crate::Uses::empty());
		self.buffers.add(buffer).1
	}

	pub(crate) fn build_image_internal(
		&mut self,
		next: Option<ImageHandle>,
		name: Option<&str>,
		format: crate::Formats,
		device_accesses: DeviceAccesses,
		array_layers: Option<NonZeroU32>,
		cube_compatible: bool,
		cube_array_compatible: bool,
		extent: Extent,
		resource_uses: crate::Uses,
		mip_levels: u32,
	) -> Image {
		let unbacked = Image {
			next,
			layers: array_layers,
			cube_compatible,
			cube_array_compatible,
			mip_levels,
			..unbacked_image(format, resource_uses, extent, device_accesses)
		};

		if extent.width() == 0 {
			return unbacked;
		}

		// Every array layer has a complete image payload in the shared staging buffer.
		let layer_count = array_layers.map_or(1, NonZeroU32::get) as usize;
		let size = extent.width() as usize
			* extent.height().max(1) as usize
			* extent.depth().max(1) as usize
			* format.size()
			* layer_count;

		let cpu_read = device_accesses.contains(DeviceAccesses::CpuRead);
		let cpu_write = device_accesses.contains(DeviceAccesses::CpuWrite);
		let mut transfer_uses = crate::Uses::empty();
		transfer_uses.set(crate::Uses::TransferSource, cpu_read);
		transfer_uses.set(crate::Uses::TransferDestination, cpu_write);

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
		let image = texture_creation_result.resource;

		let uses_cpu_staging = device_accesses.intersects(DeviceAccesses::HostOnly);
		let (image_allocation, _) = self.create_allocation_internal(
			texture_creation_result.size,
			texture_creation_result.memory_flags.into(),
			if uses_cpu_staging {
				DeviceAccesses::DeviceOnly
			} else {
				device_accesses
			},
		);
		unsafe {
			self.device
				.bind_image_memory(image, self.allocation(image_allocation).memory, 0)
				.expect("No image memory binding")
		};

		let (staging_buffer, staging_allocation, pointer) = if uses_cpu_staging {
			// A staging buffer may serve both readback and upload when the image allows both CPU access modes.
			let buffer_creation_result = self.create_vulkan_buffer(name, size, transfer_usage(cpu_write, cpu_read));
			// Preserve both host access directions so allocation selects visible memory and coherent uploads.
			let (allocation_handle, _) = self.create_allocation_internal(
				buffer_creation_result.size,
				buffer_creation_result.memory_flags.into(),
				device_accesses & DeviceAccesses::HostOnly,
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

		let (full_image_view, image_views) = self.create_image_views(
			name,
			image,
			format,
			resource_uses | transfer_uses,
			extent,
			mip_levels,
			array_layers,
		);

		Image {
			size,
			staging_buffer,
			staging_allocation,
			allocation: Some(image_allocation),
			pointer,
			image,
			full_image_view,
			image_views,
			..unbacked
		}
	}

	/// Creates the views a GHI image exposes: one per array layer, plus a whole-array view for layered images.
	///
	/// Returns a null whole-array view for single-layer images, and no views at all for transfer-only images, since
	/// Vulkan only allows views of images created with view-capable usage bits.
	fn create_image_views(
		&self,
		name: Option<&str>,
		image: vk::Image,
		format: crate::Formats,
		uses: crate::Uses,
		extent: Extent,
		mip_levels: u32,
		array_layers: Option<NonZeroU32>,
	) -> (vk::ImageView, Vec<vk::ImageView>) {
		let image_usage_flags = into_vk_image_usage_flags(uses, format);
		if !InnerDevice::image_usage_allows_views(image_usage_flags) {
			return (vk::ImageView::null(), Vec::new());
		}
		let image_type = crate::vulkan::utils::image_type_from_extent(extent).expect("Failed to get VkImageType from extent");
		let create_view = |base_layer, layer_count| {
			self.create_vulkan_image_view(
				name,
				&image,
				image_type,
				format,
				image_usage_flags,
				mip_levels,
				base_layer,
				layer_count,
			)
		};
		match array_layers {
			Some(layers) => (
				create_view(0, Some(layers)),
				(0..layers.get())
					.map(|layer| create_view(layer, NonZeroU32::new(1)))
					.collect(),
			),
			None => (vk::ImageView::null(), vec![create_view(0, None)]),
		}
	}

	/// Recreates every member of an image group in memory that members with disjoint lifetimes share.
	///
	/// Does nothing when the group is already placed from the same requests. Members keep their handles, their
	/// previous images and memory are destroyed once in-flight frames finish, and every sequence's descriptors are
	/// refreshed.
	pub(crate) fn place_image_group(
		&mut self,
		group: graphics_hardware_interface::ImageGroupHandle,
		requests: &[crate::ImageGroupMember],
	) {
		let Some(requests) = self.image_groups.requests_in_member_order(group, requests) else {
			return;
		};

		// Vulkan reports memory requirements per image, so each member's image is created before the heaps are sized.
		let textures = requests
			.iter()
			.map(|request| {
				let handle = ImageHandle(request.image.0);
				let name = self.get_object_debug_name(graphics_hardware_interface::ImageHandle(request.image).into());
				let image = &self.images[handle.0 as usize];
				let texture = self.create_vulkan_texture(
					name.as_deref(),
					request.extent,
					image.format_,
					image.uses,
					image.mip_levels,
					image.layers,
					image.cube_compatible,
					image.cube_array_compatible,
				);
				(handle, name, texture)
			})
			.collect::<Vec<_>>();
		let requirements = textures
			.iter()
			.zip(&requests)
			.map(|((_, name, texture), request)| {
				// Members share memory only when one memory type suits both, so each member's preferred type is its category.
				let memory_type = crate::vulkan::utils::memory_type_candidates(
					&self.memory_properties,
					texture.memory_flags,
					DeviceAccesses::DeviceOnly,
				)
				.first()
				.copied()
				.unwrap_or_else(|| {
					panic!(
						"Image '{}' has no device-local memory type. The most likely cause is an image usage the device cannot back with device memory.",
						name.as_deref().unwrap_or("unnamed"),
					)
				});
				(
					crate::image_group::MemoryRequirements {
						size: texture.size as u64,
						alignment: texture.alignment,
						category: memory_type,
					},
					request.lifetime.clone(),
				)
			})
			.collect::<Vec<_>>();
		let placement = crate::image_group::pack(&requirements);

		let heaps = placement
			.heaps
			.iter()
			.map(|layout| {
				self.create_allocation_internal(layout.size as usize, Some(1 << layout.category), DeviceAccesses::DeviceOnly)
					.0
			})
			.collect::<SmallVec<[_; 2]>>();

		for (((handle, name, texture), slot), request) in textures.into_iter().zip(&placement.slots).zip(&requests) {
			unsafe {
				self.device
					.bind_image_memory(texture.resource, self.allocation(heaps[slot.heap]).memory, slot.offset)
					.expect("Failed to bind an image-group member to its heap. The most likely cause is an offset that breaks the member's alignment.")
			};
			let previous = self.images[handle.0 as usize].clone();
			let (full_image_view, image_views) = self.create_image_views(
				name.as_deref(),
				texture.resource,
				previous.format_,
				previous.uses,
				request.extent,
				previous.mip_levels,
				previous.layers,
			);
			self.images[handle.0 as usize] = Image {
				image: texture.resource,
				full_image_view,
				image_views,
				extent: request.extent,
				// The group owns the heap, so replacing the member must not free it.
				allocation: None,
				owns_image: true,
				..previous.clone()
			};
			self.retire_image_storage(&previous);

			if let Some(state) = self.states.get_mut(&crate::vulkan::Handles::Image(handle)) {
				state.layout = vk::ImageLayout::UNDEFINED;
			}
		}

		// In-flight frames may still use members bound to the previous heaps, so free those heaps once the frames finish.
		let previous_heaps = std::mem::replace(&mut self.image_group_heaps[group.0 as usize], heaps);
		for heap in previous_heaps {
			self.defer_destruction(Tasks::FreeAllocation { handle: heap });
		}
		for sequence_index in 0..self.frames {
			self.bump_descriptor_sequence_epoch(sequence_index);
		}
		self.image_groups.commit(group, requests, placement);
	}

	pub(crate) fn create_image_internal(
		&mut self,
		next: Option<ImageHandle>,
		previous: Option<ImageHandle>,
		name: Option<&str>,
		format: crate::Formats,
		device_accesses: DeviceAccesses,
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
			semaphore: self.create_vulkan_semaphore(name),
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
			let mut replacement = self.build_buffer_internal(name, current.uses, size, current.access);
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
		self.tasks.push(Task::after_frame(task, self.last_started_frame.unwrap_or(0)));
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
		for &image_view in image.image_views.iter().chain([&image.full_image_view]) {
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
		for offset in 1..self.frames {
			self.tasks
				.push(Task::new(tasks, Some((current_frame + offset) % self.frames)));
		}
	}
}
