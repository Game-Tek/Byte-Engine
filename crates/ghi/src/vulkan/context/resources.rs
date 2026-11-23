use super::*;
use crate::{Copies, HostAccess};

/// Every owned buffer and image can be a copy's source and destination, because the GHI orders copies by the
/// resources they name, not by declared uses, and other backends need no usage flags to copy. Clients stage through
/// buffers they own and fill and read images with copies, so a missing transfer use would only make a valid copy
/// invalid on Vulkan.
const TRANSFER_USES: crate::Uses = crate::Uses::TransferSource.union(crate::Uses::TransferDestination);

/// Describes an owned image without native storage or views.
pub(crate) fn unbacked_image(format: crate::Formats, uses: crate::Uses, extent: Extent) -> Image {
	Image {
		next: None,
		allocation: None,
		image: vk::Image::null(),
		full_image_view: vk::ImageView::null(),
		image_views: Vec::new(),
		layer_suffix_views: Vec::new(),
		extent,
		copies: Copies::One,
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
	fn allocation(&self, allocation_handle: crate::vulkan::AllocationHandle) -> &Allocation {
		self.allocations
			.get(allocation_handle.index() as usize)
			.expect("No allocation with that handle.")
	}

	pub(crate) fn bind_vulkan_buffer_memory(
		&self,
		info: &MemoryBackedResourceCreationResult<vk::Buffer>,
		allocation_handle: crate::vulkan::AllocationHandle,
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
		allocation_handle: crate::vulkan::AllocationHandle,
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
		let (requirements, dedicated) = self.buffer_memory_requirements(buffer);
		let memory = self
			.allocate_memory_from_candidates(
				requirements.size,
				requirements.memory_type_bits,
				HostAccess::Read,
				false,
				dedicated.then_some(DedicatedResource::Buffer(buffer)),
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
		let root_handle = ImageHandle::from_index(self.images.len() as u64);
		let root_image = self.swapchain_image(vk_image, format, uses, image_usage_flags);

		if let Some(previous) = previous {
			self.images[previous.index() as usize].next = Some(root_handle);
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
			..unbacked_image(format, uses, Extent::cube(0, 0, 0))
		}
	}

	/// Allocates from the best memory type for `host_access`, moving to the next candidate when a heap is full.
	///
	/// Pass `dedicated` to make the allocation dedicated to one resource. `size` must then be the size the resource's
	/// memory requirements report, and the resource must be bound at offset 0.
	///
	/// Returns `None` when no memory type is compatible or every compatible type failed to allocate.
	pub(crate) fn allocate_memory_from_candidates(
		&self,
		size: u64,
		memory_type_bits: u32,
		host_access: HostAccess,
		device_address: bool,
		dedicated: Option<DedicatedResource>,
	) -> Option<vk::DeviceMemory> {
		crate::vulkan::utils::memory_type_candidates(&self.memory_properties, memory_type_bits, host_access)
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
				// Naming the only resource the memory backs lets the driver tailor the allocation to it.
				let mut memory_dedicated_allocate_info = match dedicated {
					Some(DedicatedResource::Buffer(buffer)) => vk::MemoryDedicatedAllocateInfo::default().buffer(buffer),
					Some(DedicatedResource::Image(image)) => vk::MemoryDedicatedAllocateInfo::default().image(image),
					None => vk::MemoryDedicatedAllocateInfo::default(),
				};
				if dedicated.is_some() {
					memory_allocate_info = memory_allocate_info.push(&mut memory_dedicated_allocate_info);
				}

				unsafe { self.device.allocate_memory(&memory_allocate_info, None) }.ok()
			})
	}

	/// Allocates memory from the device.
	///
	/// Pass [`MemoryBackedResourceCreationResult::dedicated_resource`] as `dedicated` when the allocation backs exactly
	/// that resource, and `None` when it is shared or not tied to a resource yet. A dedicated allocation must use the
	/// resource's reported size and be bound at offset 0.
	pub(crate) fn create_allocation_internal(
		&mut self,
		size: usize,
		memory_bits: Option<u32>,
		host_access: HostAccess,
		dedicated: Option<DedicatedResource>,
	) -> (crate::vulkan::AllocationHandle, Option<*mut u8>) {
		// Allocations not tied to a resource yet may use any memory type.
		let memory_type_bits = memory_bits.unwrap_or(u32::MAX);
		let memory = self
			.allocate_memory_from_candidates(size as u64, memory_type_bits, host_access, true, dedicated)
			.expect(
				"Failed to allocate Vulkan memory. The most likely cause is that every memory heap compatible with the resource is exhausted or the device allocation count limit was reached.",
			);

		let mapped_memory = (host_access != HostAccess::None).then(|| unsafe {
			self.device
				.map_memory(memory, 0, size as u64, vk::MemoryMapFlags::empty())
				.expect("No mapped memory")
				.cast::<u8>()
		});

		let allocation_handle = crate::vulkan::AllocationHandle::from_index(self.allocations.len() as u64);
		self.allocations.push(Allocation {
			memory,
			pointer: crate::vulkan::MappedMemoryPointer(mapped_memory.unwrap_or(std::ptr::null_mut())),
		});

		(allocation_handle, mapped_memory)
	}

	/// Creates a Vulkan buffer bound to its own new allocation and returns it with that allocation, its device address and
	/// its mapping.
	pub(crate) fn create_dedicated_buffer(
		&mut self,
		name: Option<&str>,
		size: usize,
		usage: vk::BufferUsageFlags,
		host_access: HostAccess,
	) -> (
		MemoryBackedResourceCreationResult<vk::Buffer>,
		crate::vulkan::AllocationHandle,
		u64,
		*mut u8,
	) {
		let buffer = self.create_vulkan_buffer(name, size, usage);
		let (allocation_handle, _) = self.create_allocation_internal(
			buffer.size,
			buffer.memory_flags.into(),
			host_access,
			buffer.dedicated_resource(),
		);
		let (address, pointer) = self.bind_vulkan_buffer_memory(&buffer, allocation_handle, 0);
		(buffer, allocation_handle, address, pointer)
	}

	/// Builds one copy of a buffer with the given name, resource uses, size, host access, and copy count.
	///
	/// A buffer the CPU accesses lives in mapped coherent memory, preferring device-local memory, so CPU writes reach
	/// the GPU and GPU writes reach the CPU without a copy. A buffer only GPU commands access lives in device memory.
	/// Mapped storage starts zeroed here; device memory starts zeroed by the fill [`Self::queue_buffer_clear`] queues.
	pub(crate) fn build_buffer_internal(
		&mut self,
		name: Option<&str>,
		resource_uses: crate::Uses,
		size: usize,
		host_access: HostAccess,
		copies: Copies,
	) -> Buffer {
		let unbacked = Buffer {
			buffer: vk::Buffer::null(),
			size: 0,
			device_address: 0,
			pointer: crate::vulkan::MappedMemoryPointer(std::ptr::null_mut()),
			allocation: None,
			uses: resource_uses,
			host_access,
			copies,
			imported: false,
		};
		if size == 0 {
			return unbacked;
		}

		// All buffers are guaranteed to be accessible by device address.
		let usage = uses_to_vk_usage_flags(resource_uses | TRANSFER_USES, self.device.ray_tracing.is_some())
			| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
		let (buffer_creation_result, allocation_handle, device_address, pointer) =
			self.create_dedicated_buffer(name, size, usage, host_access);
		if !pointer.is_null() {
			// Typed buffer APIs expose mapped storage as zeroable POD values, so initialize the complete representation.
			unsafe { std::ptr::write_bytes(pointer, 0, size) };
		}

		Buffer {
			buffer: buffer_creation_result.resource,
			size,
			device_address,
			pointer: crate::vulkan::MappedMemoryPointer(pointer),
			allocation: Some(allocation_handle),
			..unbacked
		}
	}

	/// Queues the zero fill of a new buffer copy that lives in device memory, which the next recording records before
	/// its own commands. Mapped copies were zeroed when they were built.
	pub(crate) fn queue_buffer_clear(&mut self, handle: BufferHandle) {
		let buffer = self.buffers.resource(handle);
		if buffer.host_access == HostAccess::None && !buffer.buffer.is_null() {
			self.pending_buffer_clears.push(handle);
		}
	}


	/// Builds a buffer, links it after `previous` in its per-frame chain, and returns its handle.
	pub(crate) fn create_buffer_internal(
		&mut self,
		previous: Option<BufferHandle>,
		name: Option<&str>,
		resource_uses: crate::Uses,
		size: usize,
		host_access: HostAccess,
		copies: Copies,
	) -> BufferHandle {
		let buffer = self.build_buffer_internal(name, resource_uses, size, host_access, copies);
		let (_, handle) = self.buffers.add(buffer);
		self.queue_buffer_clear(handle);

		if let Some(previous) = previous {
			self.buffers.set_next(previous, Some(handle));
		}

		handle
	}

	/// Creates the native image, memory, and views that `builder` describes, linked to `next`.
	pub(crate) fn build_image_internal(&mut self, next: Option<ImageHandle>, builder: &crate::image::Builder) -> Image {
		let crate::image::Builder {
			name,
			extent,
			format,
			resource_uses,
			copies,
			mip_levels,
			array_layers,
			cube_compatible,
			cube_array_compatible,
			..
		} = *builder;
		let unbacked = Image {
			next,
			layers: array_layers,
			cube_compatible,
			cube_array_compatible,
			mip_levels,
			copies,
			..unbacked_image(format, resource_uses, extent)
		};

		if extent.width() == 0 {
			return unbacked;
		}


		let texture_creation_result = self.create_vulkan_texture(
			name,
			extent,
			format,
			resource_uses | TRANSFER_USES,
			mip_levels,
			array_layers,
			cube_compatible,
			cube_array_compatible,
		);
		let image = texture_creation_result.resource;

		let (image_allocation, _) = self.create_allocation_internal(
			texture_creation_result.size,
			texture_creation_result.memory_flags.into(),
			HostAccess::None,
			texture_creation_result.dedicated_resource(),
		);
		unsafe {
			self.device
				.bind_image_memory(image, self.allocation(image_allocation).memory, 0)
				.expect("No image memory binding")
		};

		let (full_image_view, image_views, layer_suffix_views) = self.create_image_views(
			name,
			image,
			format,
			resource_uses | TRANSFER_USES,
			extent,
			mip_levels,
			array_layers,
		);

		Image {
			allocation: Some(image_allocation),
			image,
			full_image_view,
			image_views,
			layer_suffix_views,
			..unbacked
		}
	}

	/// Creates the views a GHI image exposes: one per array layer, plus a whole-array view for layered images, plus, for
	/// layered render targets, one view from each layer past the first to the last, which layered attachments that
	/// start past the first layer render through.
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
	) -> (vk::ImageView, Vec<vk::ImageView>, Vec<vk::ImageView>) {
		let image_usage_flags = into_vk_image_usage_flags(uses, format);
		if !InnerDevice::image_usage_allows_views(image_usage_flags) {
			return (vk::ImageView::null(), Vec::new(), Vec::new());
		}
		let render_target =
			image_usage_flags.intersects(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT);
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
				(1..layers.get())
					.filter(|_| render_target)
					.map(|first_layer| create_view(first_layer, NonZeroU32::new(layers.get() - first_layer)))
					.collect(),
			),
			None => (vk::ImageView::null(), vec![create_view(0, None)], Vec::new()),
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
				let handle = ImageHandle::from_index(request.image.index());
				let name = self.get_object_debug_name(graphics_hardware_interface::ImageHandle(request.image).into());
				let image = &self.images[handle.index() as usize];
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
					HostAccess::None,
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
				// A heap backs several members, so it is never dedicated to one of them.
				self.create_allocation_internal(layout.size as usize, Some(1 << layout.category), HostAccess::None, None)
					.0
			})
			.collect::<SmallVec<[_; 2]>>();

		for (((handle, name, texture), slot), request) in textures.into_iter().zip(&placement.slots).zip(&requests) {
			unsafe {
				self.device
					.bind_image_memory(texture.resource, self.allocation(heaps[slot.heap]).memory, slot.offset)
					.expect("Failed to bind an image-group member to its heap. The most likely cause is an offset that breaks the member's alignment.")
			};
			let previous = self.images[handle.index() as usize].clone();
			let (full_image_view, image_views, layer_suffix_views) = self.create_image_views(
				name.as_deref(),
				texture.resource,
				previous.format_,
				previous.uses,
				request.extent,
				previous.mip_levels,
				previous.layers,
			);
			self.images[handle.index() as usize] = Image {
				image: texture.resource,
				full_image_view,
				image_views,
				layer_suffix_views,
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
			self.invalidate_descriptors(crate::vulkan::Handles::Image(handle.root(&self.images)));
		}

		// In-flight frames may still use members bound to the previous heaps, so free those heaps once the frames finish.
		let previous_heaps = std::mem::replace(&mut self.image_group_heaps[group.index() as usize], heaps);
		for heap in previous_heaps {
			self.defer_destruction(Tasks::FreeAllocation { handle: heap });
		}
		self.image_groups.commit(group, requests, placement);
	}

	/// Builds an image from `builder` and links it after `previous` in its per-frame chain.
	pub(crate) fn create_image_internal(
		&mut self,
		previous: Option<ImageHandle>,
		builder: &crate::image::Builder,
	) -> ImageHandle {
		let texture_handle = ImageHandle::from_index(self.images.len() as u64);
		let image = self.build_image_internal(None, builder);

		if let Some(previous) = previous {
			self.images[previous.index() as usize].next = Some(texture_handle);
		}
		self.images.push(image);

		texture_handle
	}

	pub(crate) fn create_synchronizer_internal(&mut self, name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		let synchronizer_handle = SynchronizerHandle::from_index(self.synchronizers.len() as u64);
		self.synchronizers.push(Synchronizer {
			next: None,
			signaled,
			armed: signaled,
			fence: self.create_vulkan_fence(signaled),
			semaphore: self.create_vulkan_semaphore(name),
		});
		synchronizer_handle
	}

	/// Replaces one image's storage at `extent`. `array_layers` replaces its layer count, or keeps it when `None`.
	pub(crate) fn resize_image_internal(
		&mut self,
		image_handle: ImageHandle,
		extent: Extent,
		array_layers: Option<std::num::NonZeroU32>,
	) {
		let image = image_handle.access(&self.images);
		let array_layers = array_layers.or(image.layers);
		if !image.owns_image || (image.extent == extent && image.layers == array_layers) {
			return;
		}

		let root_handle = image_handle.root(&self.images);
		let name = self.get_object_debug_name(
			graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(root_handle.index()))
				.into(),
		);

		let image = self.images[image_handle.index() as usize].clone();
		let builder = crate::image::Builder {
			name: name.as_deref(),
			extent,
			array_layers,
			..image.builder()
		};
		let new_image = self.build_image_internal(image.next, &builder);

		self.images[image_handle.index() as usize] = new_image;
		self.retire_image_storage(&image);

		if let Some(state) = self.states.get_mut(&crate::vulkan::Handles::Image(image_handle)) {
			state.layout = vk::ImageLayout::UNDEFINED;
		}

		// Set elements name the image by its chain, and a frame offset can make any copy of a set read this member.
		self.invalidate_descriptors(crate::vulkan::Handles::Image(root_handle));
	}

	/// Queues a destruction for the first task pass after every frame started so far has completed on the GPU.
	///
	/// Frames that already started may have recorded or submitted work that references the object. Before the first
	/// frame, work submitted outside frames is covered by waiting for frame 0, whose fence orders all earlier submissions.
	pub(crate) fn defer_destruction(&mut self, task: Tasks) {
		self.tasks.push(Task::after(
			task,
			crate::vulkan::Completion::Frame(self.last_started_frame.unwrap_or(0)),
		));
	}

	/// Destroys a buffer that [`Self::create_dedicated_buffer`] made, and frees its memory, once the frames recorded
	/// before this call complete.
	pub(crate) fn retire_dedicated_buffer(&mut self, buffer: vk::Buffer, allocation: crate::vulkan::AllocationHandle) {
		self.defer_destruction(Tasks::DeleteVulkanBuffer { handle: buffer });
		self.defer_destruction(Tasks::FreeAllocation { handle: allocation });
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
		for &image_view in image
			.image_views
			.iter()
			.chain(&image.layer_suffix_views)
			.chain([&image.full_image_view])
		{
			if !image_view.is_null() {
				self.defer_destruction(Tasks::DeleteVulkanImageView { handle: image_view });
			}
		}
		if image.owns_image && !image.image.is_null() {
			self.defer_destruction(Tasks::DeleteVulkanImage { handle: image.image });
		}
		if let Some(allocation) = image.allocation {
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
					let allocation = &mut self.allocations[handle.index() as usize];
					let memory = std::mem::replace(&mut allocation.memory, vk::DeviceMemory::null());
					allocation.pointer = crate::vulkan::MappedMemoryPointer(std::ptr::null_mut());
					if !memory.is_null() {
						self.device.free_memory(memory, None);
					}
				}
				Tasks::DeleteAccelerationStructure { handle, storage } => {
					if let Some(ray_tracing) = &self.device.ray_tracing {
						ray_tracing
							.acceleration_structure
							.destroy_acceleration_structure(handle, None);
					}
					self.acceleration_structures.storage.free(storage);
				}
				Tasks::FreeDescriptorRanges {
					resource_index,
					resource_count,
					sampler_index,
					sampler_count,
				} => self.free_descriptor_ranges(resource_index, resource_count, sampler_index, sampler_count),
				_ => return false,
			}
		}
		true
	}

	/// Add the task to all other frames but the current frame.
	pub(crate) fn add_task_to_all_other_frames(&mut self, tasks: Tasks, current_frame: u8) {
		for offset in 1..self.frames {
			self.tasks
				.push(Task::new(tasks, Some((current_frame + offset) % self.frames)));
		}
	}
}
