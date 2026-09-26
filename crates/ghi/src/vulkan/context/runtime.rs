use super::*;

impl Context {
	pub(crate) fn new(device: &Device) -> Result<Self, &'static str> {
		let mut device = device.inner.clone().ok_or("Failed to create a Vulkan context. The most likely cause is that a detached device was used as the primary graphics device.")?;
		let queues = std::mem::take(&mut device.queues);
		let vk_queues = std::mem::take(&mut device.vk_queues)
			.into_iter()
			.map(std::sync::Mutex::new)
			.collect();

		let mut context = Context {
			memory_properties: device.memory_properties,
			settings: device.settings,
			device,

			frames: 2, // Assuming double buffering

			queues,
			vk_queues,
			allocations: Vec::new(),
			buffers: ResourceCollection::with_capacity(1024),
			images: Vec::with_capacity(512),
			samplers: Vec::with_capacity(128),
			pipeline_layouts: Vec::with_capacity(64),
			pipeline_layout_indices: HashMap::with_capacity_and_hasher(64, Default::default()),
			descriptor_sets: Vec::with_capacity(512),
			descriptor_heaps: None,
			descriptor_materializations: Vec::with_capacity(512),
			materialization_indices: HashMap::with_capacity_and_hasher(512, Default::default()),
			retired_materializations: std::array::from_fn(|_| Vec::with_capacity(128)),
			free_materialization_handles: Vec::with_capacity(128),
			descriptor_sequence_epochs: [0; MAX_FRAMES_IN_FLIGHT],
			acceleration_structures: Vec::new(),
			shaders: Vec::with_capacity(1024),
			pipelines: Vec::with_capacity(1024),
			meshes: Vec::new(),
			command_buffers: Vec::with_capacity(32),
			synchronizers: Vec::with_capacity(32),
			swapchains: Vec::with_capacity(4),
			texture_readbacks: crate::context::TextureReadbackRegistry::new(),

			states: HashMap::with_capacity_and_hasher(4096, Default::default()),
			buffer_states: HashMap::with_capacity_and_hasher(4096, Default::default()),

			pending_buffer_syncs: HashSet::with_capacity_and_hasher(128, Default::default()),
			pending_image_syncs: HashSet::with_capacity_and_hasher(128, Default::default()),

			persistent_write_dynamic_buffers: Vec::with_capacity(64),

			tasks: Vec::with_capacity(1024),
			last_started_frame: None,
			completed_frame: None,
			image_groups: crate::image_group::ImageGroups::default(),
			image_group_heaps: Vec::new(),

			#[cfg(debug_assertions)]
			names: HashMap::with_capacity_and_hasher(4096, Default::default()),
		};
		context.descriptor_heaps = Some(context.create_descriptor_heaps());
		Ok(context)
	}

	/// Creates a detached-resource factory backed by this Vulkan device.
	pub fn create_factory(&self) -> Option<crate::implementation::Factory> {
		Some(crate::implementation::Factory::detached_with_resources(
			self.device.device.clone(),
			self.device.descriptor_heap_properties,
		))
	}

	pub(crate) fn create_command_buffer(
		&mut self,
		name: Option<&str>,
		queue_handle: graphics_hardware_interface::QueueHandle,
	) -> graphics_hardware_interface::CommandBufferHandle {
		let command_buffer_handle = graphics_hardware_interface::CommandBufferHandle(self.command_buffers.len() as u64);
		let frames = (0..self.frames)
			.map(|_| self.create_command_buffer_frame(queue_handle, vk::CommandPoolCreateFlags::TRANSIENT, name))
			.collect();
		self.command_buffers.push(CommandBuffer { queue_handle, frames });
		command_buffer_handle
	}

	/// Creates one frame's command pool and primary command buffer for the queue family behind `queue_handle`.
	pub(super) fn create_command_buffer_frame(
		&self,
		queue_handle: graphics_hardware_interface::QueueHandle,
		flags: vk::CommandPoolCreateFlags,
		name: Option<&str>,
	) -> CommandBufferInternal {
		let queue = &self.queues[queue_handle.0 as usize];
		let command_pool_create_info = vk::CommandPoolCreateInfo::default()
			.flags(flags)
			.queue_family_index(queue.queue_family_index);
		let command_pool = unsafe {
			self.device
				.create_command_pool(&command_pool_create_info, None)
				.expect("No command pool")
		};

		let command_buffer_allocate_info = vk::CommandBufferAllocateInfo::default()
			.command_pool(command_pool)
			.level(vk::CommandBufferLevel::PRIMARY)
			.command_buffer_count(1);
		let command_buffer = unsafe {
			self.device
				.allocate_command_buffers(&command_buffer_allocate_info)
				.expect("No command buffer")[0]
		};
		self.set_name(command_buffer, name);

		CommandBufferInternal {
			vk_queue_index: queue.vk_queue_index,
			command_pool,
			command_buffer,
		}
	}

	/// Retains flat descriptor writes and schedules frame-local snapshot refreshes without touching command-visible heap memory.
	pub fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		for &descriptor_write in descriptor_set_writes {
			assert!(
				!matches!(
					descriptor_write.descriptor,
					crate::descriptors::WriteData::StaticSamplers | crate::descriptors::WriteData::CombinedImageSamplerArray
				),
				"Unsupported Vulkan descriptor write. The most likely cause is that a removed legacy descriptor constructor is still in use.",
			);
			let retained = crate::vulkan::descriptor_set::RetainedDescriptor {
				descriptor: descriptor_write.descriptor,
				frame_offset: descriptor_write.frame_offset.unwrap_or(0),
			};
			let descriptor_set = self
				.descriptor_sets
				.get_mut(descriptor_write.descriptor_set.0 as usize)
				.expect(
					"Invalid Vulkan descriptor set. The most likely cause is that the write used a handle from another context.",
				);
			let previous = descriptor_set
				.descriptors
				.entry(descriptor_write.slot)
				.or_default()
				.insert(descriptor_write.array_element, retained);
			if previous == Some(retained) {
				continue;
			}

			descriptor_set.version = descriptor_set.version.wrapping_add(1);
			let expected_set_version = descriptor_set.version;
			self.invalidate_descriptor_set_materializations(descriptor_write.descriptor_set, None);
			self.add_task_to_all_frames(Tasks::UpdateDescriptor {
				descriptor_write,
				expected_set_version,
			});
		}
	}

	/// Creates a recording that belongs to no frame, for transfers submitted outside the render loop.
	pub fn create_command_buffer_recording(
		&mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> crate::vulkan::CommandBufferRecording<'_> {
		let (buffer_copies, images) = self.take_pending_syncs();
		let mut recording = CommandBufferRecording::new(self, command_buffer_handle, None);
		recording.sync_buffers(buffer_copies.into_iter());
		recording.sync_textures(images.into_iter());
		recording
	}

	/// Drains the staging uploads queued by CPU writes, as buffer copies and image uploads.
	pub(crate) fn take_pending_syncs(&mut self) -> (Vec<BufferCopy>, Vec<ImageCopy>) {
		let buffers = &self.buffers;
		let buffer_copies = self
			.pending_buffer_syncs
			.drain()
			.filter_map(|handle| {
				let buffer = buffers.resource(handle);
				Some(BufferCopy::new(buffer.staging?, 0, handle, 0, buffer.size))
			})
			.collect();
		let image_copies = self
			.pending_image_syncs
			.drain()
			.map(|(dst_texture, region)| ImageCopy { dst_texture, region })
			.collect();
		(buffer_copies, image_copies)
	}

	pub(crate) fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.buffers.get_single(buffer_handle).unwrap().device_address
	}

	/// Returns the CPU-visible buffer behind `buffer_handle`, which is its staging buffer when it has one.
	pub(super) fn host_visible_buffer(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> &Buffer {
		let buffer = self.buffers.get_single(buffer_handle).unwrap();
		buffer.staging.map_or(buffer, |staging| self.buffers.resource(staging))
	}

	pub(super) fn typed_buffer_pointer<T: crate::Pod>(
		&self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> *mut T {
		let buffer = self.host_visible_buffer(buffer_handle.into());
		crate::buffer::typed_buffer_pointer::<T>(buffer.pointer.0, buffer.size).expect(
			"Failed to map a typed Vulkan buffer. The most likely cause is that the buffer has no sufficiently large, aligned CPU-visible storage.",
		)
	}

	pub(crate) fn get_mut_buffer_slice<T: crate::Pod>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> &mut T {
		// SAFETY: Typed handles preserve the allocation's type and `&mut self` guarantees exclusive CPU access.
		unsafe { &mut *self.typed_buffer_pointer(buffer_handle) }
	}

	pub(crate) fn sync_buffer(&mut self, buffer_handle: impl Into<crate::BaseBufferHandle>) {
		let handle = BufferHandle(buffer_handle.into().0);
		if self.buffers.resource(handle).staging.is_some() {
			self.pending_buffer_syncs.insert(handle);
		}
	}

	pub(crate) fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::ImageHandle) -> &mut [u8] {
		let texture = &self.images[texture_handle.0.0 as usize];

		assert!(
			texture.staging_buffer.is_some(),
			"Attempted to map an image without a staging buffer. The most likely cause is that the image was created without CPU-visible access but is being written from the CPU."
		);
		let pointer = texture.pointer.map(|pointer| pointer.0).expect(
			"Attempted to map an image without a CPU-visible pointer. The most likely cause is that image resize or creation did not rebuild the host-visible staging allocation."
		);
		assert!(
			texture.size > 0,
			"Attempted to map a zero-sized image. The most likely cause is that the image was used before receiving a valid extent."
		);

		unsafe { std::slice::from_raw_parts_mut(pointer, texture.size) }
	}

	pub(crate) fn sync_texture(&mut self, image_handle: crate::ImageHandle) {
		let image_handle = ImageHandle(image_handle.0.0);
		assert!(
			self.images[image_handle.0 as usize].staging_buffer.is_some(),
			"Attempted to sync an image without a staging buffer. The most likely cause is that CPU-side image uploads are being requested for a GPU-only image."
		);
		self.pending_image_syncs.insert((image_handle, None));
	}

	pub(crate) fn write_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let handle = ImageHandle(image_handle.0.0);
		let texture = handle.access(&self.images);
		f(unsafe { std::slice::from_raw_parts_mut(texture.pointer.unwrap().0, texture.size) });
		self.pending_image_syncs.insert((handle, None));
	}

	pub(crate) fn write_instance(
		&mut self,
		instances_buffer: graphics_hardware_interface::BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		let buffer = self.acceleration_structures[acceleration_structure.0 as usize].buffer;
		let address = unsafe {
			self.device
				.device
				.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer))
		};

		let instance = vk::AccelerationStructureInstanceKHR {
			transform: vk::TransformMatrixKHR {
				matrix: std::array::from_fn(|i| transform[i / 4][i % 4]),
			},
			instance_custom_index_and_mask: vk::Packed24_8::new(custom_index as u32, mask),
			instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
				sbt_record_offset as u32,
				vk::GeometryInstanceFlagsKHR::FORCE_OPAQUE.as_raw() as u8,
			),
			acceleration_structure_reference: vk::AccelerationStructureReferenceKHR { device_handle: address },
		};

		let instance_buffer = self.buffers.get_single(instances_buffer).unwrap();
		let instances = unsafe {
			std::slice::from_raw_parts_mut(
				instance_buffer.pointer.0 as *mut vk::AccelerationStructureInstanceKHR,
				instance_buffer.size / std::mem::size_of::<vk::AccelerationStructureInstanceKHR>(),
			)
		};
		instances[instance_index] = instance;
	}

	pub(crate) fn write_sbt_entry(
		&mut self,
		sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
		shader_handle: graphics_hardware_interface::ShaderHandle,
	) {
		let shader_group_handle = &self.pipelines[pipeline_handle.0 as usize].shader_handles[&shader_handle];
		let buffer = self
			.buffers
			.resource(self.buffers.get_single(sbt_buffer_handle).unwrap().staging.unwrap());

		(unsafe { std::slice::from_raw_parts_mut(buffer.pointer.0, buffer.size) })[sbt_record_offset..sbt_record_offset + 32]
			.copy_from_slice(shader_group_handle);
	}

	pub(crate) fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: graphics_hardware_interface::PresentationModes,
		fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		let (
			vk_surface,
			vk_present_mode,
			min_image_count,
			extent,
			proxy_format,
			uses_proxy_images,
			native_image_usage,
			vk_swapchain,
		) = self
			.device
			.build_swapchain(window_os_handles, presentation_mode, fallback_extent, uses);

		let swapchain_handle = graphics_hardware_interface::SwapchainHandle(self.swapchains.len() as u64);

		let mut acquire_synchronizers = [SynchronizerHandle(!0u64); MAX_FRAMES_IN_FLIGHT];
		for synchronizer in &mut acquire_synchronizers[..self.frames as usize] {
			*synchronizer = self.create_synchronizer_internal(Some("Swapchain Acquire Sync"), true);
		}

		let vk_images = unsafe {
			self.device
				.swapchain
				.get_swapchain_images(vk_swapchain)
				.expect("No swapchain images found.")
		};
		assert!(
			vk_images.len() <= MAX_SWAPCHAIN_IMAGES,
			"Vulkan swapchain returned more images than the backend tracks. The most likely cause is a surface whose minimum image count exceeds MAX_SWAPCHAIN_IMAGES."
		);

		let mut submit_synchronizers = [SynchronizerHandle(!0u64); MAX_SWAPCHAIN_IMAGES];
		for synchronizer in &mut submit_synchronizers[..vk_images.len()] {
			*synchronizer = self.create_synchronizer_internal(Some("Swapchain Submit Sync"), true);
		}

		let native_uses = if uses_proxy_images {
			crate::Uses::TransferDestination
		} else {
			uses
		};
		let mut native_images = [ImageHandle(!0u64); MAX_SWAPCHAIN_IMAGES];
		for (i, &vk_image) in vk_images.iter().enumerate() {
			let previous = i.checked_sub(1).map(|previous| native_images[previous]);
			native_images[i] =
				self.create_swapchain_image(vk_image, crate::Formats::BGRAsRGB, native_uses, native_image_usage, previous);
		}

		let mut images = native_images;
		if uses_proxy_images {
			let proxy_uses = uses | crate::Uses::TransferSource | crate::Uses::TransferDestination;
			for i in 0..vk_images.len() {
				let previous = i.checked_sub(1).map(|previous| images[previous]);
				images[i] = self.create_image_internal(
					None,
					previous,
					Some("Swapchain Proxy Image"),
					proxy_format,
					crate::DeviceAccesses::DeviceOnly,
					None,
					false,
					false,
					Extent::rectangle(extent.width, extent.height),
					proxy_uses,
					1,
				);
			}
		}

		self.swapchains.push(Swapchain {
			surface: vk_surface,
			swapchain: vk_swapchain,
			acquire_synchronizers,
			submit_synchronizers,
			extent,
			images,
			native_images,
			uses_proxy_images,
			proxy_uses: if uses_proxy_images { uses } else { crate::Uses::empty() },
			uses,
			native_image_usage,
			needs_recreation: false,
			acquired_image_indices: [0; MAX_FRAMES_IN_FLIGHT],
			acquire_wait_stages: [vk::PipelineStageFlags2::NONE; MAX_FRAMES_IN_FLIGHT],
			min_image_count,
			max_image_count: vk_images.len() as u32,
			vk_present_mode,
			present_interval: None,
			next_present_slot: None,
		});

		swapchain_handle
	}

	/// Immediately destroys the views of a swapchain image wrapper whose Vulkan image is being replaced.
	fn destroy_swapchain_image_views(&mut self, handle: ImageHandle) {
		for view in std::mem::take(&mut self.images[handle.0 as usize].image_views) {
			unsafe { self.device.destroy_image_view(view, None) };
		}
	}

	/// Rebuilds a swapchain for the surface's current extent while keeping user-facing image handles stable.
	///
	/// Returns `false` without touching the swapchain when the surface has no area, for example while minimized.
	pub(crate) fn recreate_swapchain(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
		capabilities: &vk::SurfaceCapabilitiesKHR,
	) -> bool {
		let swapchain = &self.swapchains[swapchain_handle.0 as usize];
		let extent = InnerDevice::swapchain_extent(capabilities, swapchain.extent);
		if extent.width == 0 || extent.height == 0 {
			return false;
		}

		let (surface, present_mode, old_swapchain) = (swapchain.surface, swapchain.vk_present_mode, swapchain.swapchain);
		let native_image_usage = swapchain.native_image_usage;
		let uses_proxy_images = swapchain.uses_proxy_images;
		let native_uses = if uses_proxy_images {
			crate::Uses::TransferDestination
		} else {
			swapchain.uses
		};
		let proxy_uses = swapchain.uses | crate::Uses::TransferSource | crate::Uses::TransferDestination;
		let old_image_count = swapchain.max_image_count as usize;
		let (mut native_images, mut images, mut submit_synchronizers) =
			(swapchain.native_images, swapchain.images, swapchain.submit_synchronizers);

		// Old swapchain images, their views, and proxies may still be referenced by in-flight frames.
		unsafe {
			self.device.device_wait_idle().expect(
				"Failed to wait for the Vulkan device before recreating a swapchain. The most likely cause is that the device was lost.",
			);
		}

		let new_swapchain =
			self.device
				.create_vulkan_swapchain(surface, present_mode, capabilities, extent, native_image_usage, old_swapchain);
		let vk_images = unsafe {
			self.device.swapchain.destroy_swapchain(old_swapchain, None);
			self.device
				.swapchain
				.get_swapchain_images(new_swapchain)
				.expect("Failed to get recreated Vulkan swapchain images. The most likely cause is that the surface was lost.")
		};
		assert!(
			vk_images.len() <= MAX_SWAPCHAIN_IMAGES,
			"Vulkan swapchain returned more images than the backend tracks. The most likely cause is a surface whose minimum image count exceeds MAX_SWAPCHAIN_IMAGES."
		);

		let proxy_format = self.images[images[0].0 as usize].format_;
		let proxy_extent = Extent::rectangle(extent.width, extent.height);

		for (index, &vk_image) in vk_images.iter().enumerate() {
			if index < old_image_count {
				let native = native_images[index];
				self.destroy_swapchain_image_views(native);
				let mut image = self.swapchain_image(vk_image, crate::Formats::BGRAsRGB, native_uses, native_image_usage);
				image.next = self.images[native.0 as usize].next;
				self.images[native.0 as usize] = image;

				if uses_proxy_images {
					self.resize_image_internal(images[index], proxy_extent, 0);
				}
			} else {
				native_images[index] = self.create_swapchain_image(
					vk_image,
					crate::Formats::BGRAsRGB,
					native_uses,
					native_image_usage,
					Some(native_images[index - 1]),
				);
				submit_synchronizers[index] = self.create_synchronizer_internal(Some("Swapchain Submit Sync"), true);
				images[index] = if uses_proxy_images {
					self.create_image_internal(
						None,
						Some(images[index - 1]),
						Some("Swapchain Proxy Image"),
						proxy_format,
						crate::DeviceAccesses::DeviceOnly,
						None,
						false,
						false,
						proxy_extent,
						proxy_uses,
						1,
					)
				} else {
					native_images[index]
				};
			}

			// Fresh swapchain images start undefined; stale tracked layouts would produce invalid barriers.
			self.states.remove(&crate::vulkan::Handles::Image(native_images[index]));
		}

		// Surplus images from a larger previous swapchain are never indexed again, but their views must not leak.
		for &native in &native_images[vk_images.len()..old_image_count] {
			self.images[native.0 as usize].image = vk::Image::null();
			self.destroy_swapchain_image_views(native);
			self.states.remove(&crate::vulkan::Handles::Image(native));
		}

		// Snapshots may hold views of the replaced images in any sequence.
		for sequence_index in 0..self.frames {
			self.bump_descriptor_sequence_epoch(sequence_index);
		}

		let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
		swapchain.swapchain = new_swapchain;
		swapchain.extent = extent;
		swapchain.native_images = native_images;
		swapchain.images = images;
		swapchain.submit_synchronizers = submit_synchronizers;
		swapchain.min_image_count = capabilities.min_image_count;
		swapchain.max_image_count = vk_images.len() as u32;
		swapchain.acquired_image_indices = [0; MAX_FRAMES_IN_FLIGHT];
		swapchain.acquire_wait_stages = [vk::PipelineStageFlags2::NONE; MAX_FRAMES_IN_FLIGHT];
		swapchain.needs_recreation = false;

		true
	}

	/// Releases one Vulkan readback buffer and its dedicated allocation exactly once.
	pub(super) fn release_texture_readback(&self, readback: &TextureReadbackStorage) {
		unsafe {
			self.device.destroy_buffer(readback.buffer, None);
			if readback.memory != vk::DeviceMemory::null() {
				if !readback.pointer.0.is_null() {
					self.device.unmap_memory(readback.memory);
				}
				self.device.free_memory(readback.memory, None);
			}
		}
	}

	/// Abandons one readback that never reached queue submission and releases its native storage.
	pub(crate) fn cancel_texture_readback(&mut self, handle: graphics_hardware_interface::TextureCopyHandle) {
		if let Some(readback) = self.texture_readbacks.abandon_recorded(handle) {
			self.release_texture_readback(&readback);
		}
	}

	/// Waits for Vulkan work, copies one mapped transfer result, and releases its dedicated staging resources.
	pub(crate) fn get_image_data(
		&mut self,
		texture_copy_handle: graphics_hardware_interface::TextureCopyHandle,
	) -> Result<crate::TextureReadback, crate::TextureTransferError> {
		self.texture_readbacks.submitted(texture_copy_handle)?;
		self.device.wait();
		let readback = self.texture_readbacks.take_submitted(texture_copy_handle)?;
		let result = if readback.memory == vk::DeviceMemory::null() || readback.pointer.0.is_null() {
			Err(crate::TextureTransferError::MappingFailed)
		} else {
			let mapped_range = vk::MappedMemoryRange::default()
				.memory(readback.memory)
				.offset(0)
				.size(vk::WHOLE_SIZE);
			unsafe {
				self.device
					.invalidate_mapped_memory_ranges(&[mapped_range])
					.map(|()| std::slice::from_raw_parts(readback.pointer.0, readback.size).to_vec())
					.map_err(|_| crate::TextureTransferError::MappingFailed)
			}
		};

		// The transfer slot has been consumed, so retire its dedicated native resources before returning owned bytes.
		self.release_texture_readback(&readback);

		result.map(|bytes| crate::TextureReadback {
			bytes,
			extent: readback.extent,
			format: readback.format,
			bytes_per_row: readback.bytes_per_row,
			bytes_per_image: readback.bytes_per_image,
		})
	}

	pub(crate) fn start_frame<'a>(
		&'a mut self,
		index: u64,
		synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> crate::queue::StartedFrame<Frame<'a>> {
		const WAIT_SLICE_MS: u64 = 1;
		const WAIT_WARNING_THRESHOLD_MS: u64 = 8;

		let sequence_index = (index % u64::from(self.frames)) as u8;
		let synchronizer_index = self.get_syncronizer_handles(synchronizer_handle)[sequence_index as usize].0 as usize;
		let synchronizer = &self.synchronizers[synchronizer_index];

		// Waits in short slices so that a stalled fence gets reported instead of hanging silently.
		if synchronizer.armed {
			for timeout_count in 0u64.. {
				match unsafe {
					self.device
						.device
						.wait_for_fences(&[synchronizer.fence], true, WAIT_SLICE_MS * 1000000)
				} {
					Ok(()) => break,
					Err(vk::Result::TIMEOUT) => {
						if timeout_count * WAIT_SLICE_MS >= WAIT_WARNING_THRESHOLD_MS && timeout_count % 500 == 0 {
							let name = self.get_object_debug_name(synchronizer_handle.into());
							println!(
								"Stuck waiting for fence ({}) for {} ms at frame {index}. There is a potential issue with synchronization.",
								name.as_deref().unwrap_or("unknown"),
								WAIT_SLICE_MS * timeout_count
							);
						}
					}
					Err(_) => panic!("Failed to wait for fence"),
				}
			}
		}

		unsafe {
			self.device
				.device
				.reset_fences(&[synchronizer.fence])
				.expect("No fence reset");
		}
		self.synchronizers[synchronizer_index].armed = false;

		let frame_key = FrameKey {
			frame_index: index,
			sequence_index,
		};
		let completed_frame = crate::queue::completed_frame_key(index, self.frames);
		self.last_started_frame = Some(index);
		// The fence orders every earlier submission on the queue, so all frames up to this one have completed.
		// `None` orders below every frame, so the known completed frame only ever advances.
		self.completed_frame = self.completed_frame.max(completed_frame.map(|frame| frame.frame_index));

		// The sequence fence has completed, so immutable snapshots retired by earlier updates can now be reused.
		self.release_retired_descriptor_materializations(sequence_index);
		// Build lazy resources before the frame may need them.
		self.process_tasks(sequence_index);
		// Tasks processed after the fence can retire prior-frame snapshots immediately.
		self.release_retired_descriptor_materializations(sequence_index);

		crate::queue::StartedFrame::new(Frame::new(self, frame_key), completed_frame)
	}

	/// Acquires the swapchain image that `frame` will present before the frame is started.
	///
	/// The sequence fence is waited (not reset) first: the acquire semaphore of this sequence was last waited by the
	/// submission `frames_in_flight` frames ago, and that wait must have executed before the semaphore can be signaled
	/// again. [`Self::start_frame`] later sees the same fence signaled, so its wait returns at once before the reset.
	pub(crate) fn acquire_swapchain_image(
		&mut self,
		frame: crate::queue::FrameRequest<'_>,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		let sequence_index = (frame.index % u64::from(self.frames)) as u8;
		let synchronizer_index = self.get_syncronizer_handles(frame.synchronizer)[sequence_index as usize].0 as usize;
		let synchronizer = &self.synchronizers[synchronizer_index];
		if synchronizer.armed {
			unsafe {
				self.device.device.wait_for_fences(&[synchronizer.fence], true, u64::MAX).expect(
					"Failed to wait for the frame sequence fence before swapchain acquisition. The most likely cause is that the device was lost.",
				);
			}
		}
		self.acquire_swapchain_image_for_sequence(sequence_index, swapchain_handle)
	}

	pub(crate) fn set_present_interval(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
		interval: Option<std::time::Duration>,
	) {
		self.swapchains[swapchain_handle.0 as usize].present_interval = interval;
	}

	/// Acquires the next image of `swapchain_handle` with the acquire synchronizer of `sequence_index`, recreating the
	/// swapchain when it no longer matches its surface.
	///
	/// Returns `None` when no image could be acquired, such as while the window is minimized; callers must skip
	/// rendering and presentation for that swapchain this frame.
	pub(crate) fn acquire_swapchain_image_for_sequence(
		&mut self,
		sequence_index: u8,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		{
			// Vulkan has no timed present in the extensions we enable, so the cap paces acquisition instead.
			let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
			crate::swapchain::pace_present(&mut swapchain.next_present_slot, swapchain.present_interval);
		}

		let capabilities = self.query_swapchain_capabilities(swapchain_handle);
		let swapchain = &self.swapchains[swapchain_handle.0 as usize];
		let extent_changed = capabilities.current_extent.width != u32::MAX && capabilities.current_extent != swapchain.extent;
		if (swapchain.needs_recreation || extent_changed) && !self.recreate_swapchain(swapchain_handle, &capabilities) {
			return None;
		}

		let mut recreated = false;
		let index = loop {
			match self.acquire_next_swapchain_image(sequence_index, swapchain_handle) {
				Ok((index, suboptimal)) => {
					// The acquired image is still presentable, so rebuild on the next acquire instead of discarding it.
					if suboptimal {
						self.swapchains[swapchain_handle.0 as usize].needs_recreation = true;
					}
					break index;
				}
				Err(vk::Result::ERROR_OUT_OF_DATE_KHR) if !recreated => {
					recreated = true;
					let capabilities = self.query_swapchain_capabilities(swapchain_handle);
					if !self.recreate_swapchain(swapchain_handle, &capabilities) {
						return None;
					}
				}
				Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
					self.swapchains[swapchain_handle.0 as usize].needs_recreation = true;
					return None;
				}
				Err(error) => panic!(
					"Failed to acquire a Vulkan swapchain image ({error:?}). The most likely cause is that the surface or the device was lost."
				),
			}
		};

		let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
		swapchain.acquired_image_indices[sequence_index as usize] = index as u8;
		swapchain.acquire_wait_stages[sequence_index as usize] = vk::PipelineStageFlags2::NONE;
		let native_image = swapchain.native_images[index as usize];
		let extent = Extent::rectangle(swapchain.extent.width, swapchain.extent.height);

		// The presentation engine hands the image back with undefined contents and no prior GPU work to order against;
		// recording chains its first barrier to the acquire semaphore instead.
		self.states.insert(
			crate::vulkan::Handles::Image(native_image),
			TransitionState::new(
				vk::PipelineStageFlags2::NONE,
				vk::AccessFlags2::NONE,
				vk::ImageLayout::UNDEFINED,
			),
		);

		Some(crate::frame::SwapchainAcquisition {
			present_key: graphics_hardware_interface::PresentKey {
				image_index: index as u8,
				sequence_index,
				swapchain: swapchain_handle,
			},
			extent,
			present_time: None,
		})
	}

	fn query_swapchain_capabilities(
		&self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> vk::SurfaceCapabilitiesKHR {
		let swapchain = &self.swapchains[swapchain_handle.0 as usize];
		self.device
			.query_swapchain_surface_capabilities(swapchain.surface, swapchain.vk_present_mode)
	}

	/// Runs one `vkAcquireNextImage2KHR` with the acquire synchronizer of `sequence_index`, tracking whether its fence will signal.
	fn acquire_next_swapchain_image(
		&mut self,
		sequence_index: u8,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Result<(u32, bool), vk::Result> {
		let swapchain = &self.swapchains[swapchain_handle.0 as usize];
		let synchronizer_index = swapchain.acquire_synchronizers[sequence_index as usize].0 as usize;
		let synchronizer = &self.synchronizers[synchronizer_index];

		// Only one image can be held at a time when the swapchain has no spare images, so poll instead of blocking in the driver.
		let use_vulkan_timeout = swapchain.max_image_count > swapchain.min_image_count;

		let acquire_info = vk::AcquireNextImageInfoKHR::default()
			.swapchain(swapchain.swapchain)
			.timeout(if use_vulkan_timeout { u64::MAX } else { 0 })
			.semaphore(synchronizer.semaphore)
			.device_mask(1)
			.fence(synchronizer.fence);

		unsafe {
			if synchronizer.armed {
				self.device
					.device
					.wait_for_fences(&[synchronizer.fence], true, u64::MAX)
					.expect(
						"Failed to wait for the Vulkan swapchain acquire fence. The most likely cause is that the device was lost.",
					);
			}
			self.device.device.reset_fences(&[synchronizer.fence]).expect(
				"Failed to reset the Vulkan swapchain acquire fence. The most likely cause is that the device was lost.",
			);
		}

		let result = loop {
			match unsafe { self.device.swapchain.acquire_next_image2(&acquire_info) } {
				Err(vk::Result::NOT_READY | vk::Result::TIMEOUT) if !use_vulkan_timeout => {
					std::thread::sleep(std::time::Duration::from_millis(1))
				}
				result => break result,
			}
		};

		// A failed acquire never signals the fence, so a later wait on it must be skipped.
		self.synchronizers[synchronizer_index].armed = result.is_ok();
		result
	}

	pub(crate) fn get_swapchain_image_for_sequence(
		&self,
		handle: graphics_hardware_interface::ImageHandle,
		sequence_index: usize,
	) -> Option<ImageHandle> {
		self.swapchains.iter().find_map(|swapchain| {
			let acquired_image_index = swapchain.acquired_image_indices[sequence_index] as usize;

			if swapchain.images[0].0 == handle.0.0 {
				Some(swapchain.images[acquired_image_index])
			} else if swapchain.native_images[0].0 == handle.0.0 {
				Some(swapchain.native_images[acquired_image_index])
			} else {
				None
			}
		})
	}

	pub(crate) fn resolve_descriptor_image_handle(
		&self,
		handle: graphics_hardware_interface::ImageHandle,
		sequence_index: usize,
		frame_offset: i32,
	) -> ImageHandle {
		let frame_index = self.frame_index_with_offset(sequence_index, frame_offset);
		self.get_swapchain_image_for_sequence(handle, frame_index)
			.unwrap_or_else(|| self.image_handle_for_sequence(ImageHandle(handle.0.0), frame_index))
	}

	/// Resolves a frame sequence and offset into a valid per-frame resource index.
	pub(crate) fn frame_index_with_offset(&self, sequence_index: usize, frame_offset: i32) -> usize {
		crate::frame_resources::frame_index_with_offset(sequence_index, frame_offset, self.frames as usize)
	}

	/// Selects the frame-local image handle for a chained image resource.
	pub(crate) fn image_handle_for_sequence(&self, handle: ImageHandle, sequence_index: usize) -> ImageHandle {
		let handles = handle.root(&self.images).get_all(&self.images);
		handles[sequence_index.rem_euclid(handles.len())]
	}

	/// Removes cached keys immediately while retaining their immutable bytes until the owning frame sequence completes.
	pub(crate) fn retire_descriptor_materializations(&mut self, predicate: impl Fn(&MaterializationKey) -> bool) {
		for (key, handle) in self.materialization_indices.extract_if(|key, _| predicate(key)) {
			self.retired_materializations[key.sequence_index as usize].push(handle);
		}
	}

	pub(crate) fn invalidate_descriptor_set_materializations(
		&mut self,
		descriptor_set: graphics_hardware_interface::DescriptorSetHandle,
		sequence_index: Option<u8>,
	) {
		self.retire_descriptor_materializations(|key| {
			sequence_index.is_none_or(|sequence_index| key.sequence_index == sequence_index)
				&& key.descriptor_sets.iter().any(|(handle, ..)| *handle == descriptor_set)
		});
	}

	pub(crate) fn bump_descriptor_sequence_epoch(&mut self, sequence_index: u8) {
		let epoch = &mut self.descriptor_sequence_epochs[sequence_index as usize];
		*epoch = epoch.wrapping_add(1);
		self.retire_descriptor_materializations(|key| {
			key.resource_epochs
				.iter()
				.any(|(resource_sequence, _)| *resource_sequence == sequence_index)
		});
	}

	/// Reclaims stale heap ranges only after the sequence fence proves that no command buffer still references them.
	pub(crate) fn release_retired_descriptor_materializations(&mut self, sequence_index: u8) {
		let sequence_index = sequence_index as usize;
		if self.retired_materializations[sequence_index].is_empty() {
			return;
		}
		let mut retired = std::mem::take(&mut self.retired_materializations[sequence_index]);

		let heaps = self.descriptor_heaps.as_mut().expect(
			"Missing Vulkan descriptor heaps. The most likely cause is that snapshot retirement ran before context initialization completed.",
		);
		for handle in retired.drain(..) {
			let Some(materialization) = self.descriptor_materializations[handle.0 as usize].take() else {
				continue;
			};
			heaps
				.resource_mut()
				.release(materialization.resource_heap_offset, materialization.resource_heap_size);
			heaps
				.sampler_mut()
				.release(materialization.sampler_heap_offset, materialization.sampler_heap_size);
			self.free_materialization_handles.push(handle);
		}
		// Hand the emptied vector back so its capacity is reused.
		self.retired_materializations[sequence_index] = retired;
	}

	/// Executes deferred resource work and invalidates only the frame-local immutable descriptor snapshots that may reference it.
	pub(crate) fn process_tasks(&mut self, sequence_index: u8) {
		// Tasks may queue more tasks, such as resizes retiring old storage, so collect those separately and keep them.
		let mut tasks = std::mem::take(&mut self.tasks);
		let completed_frame = self.completed_frame;

		tasks.retain(|task| {
			if task.frame().is_some_and(|frame| frame != sequence_index) || task.is_pending(completed_frame) {
				return true;
			}

			match task.task() {
				Tasks::DeleteVulkanImage { .. }
				| Tasks::DeleteVulkanImageView { .. }
				| Tasks::DeleteVulkanBuffer { .. }
				| Tasks::FreeAllocation { .. } => {
					self.run_destruction_task(task.task());
				}
				Tasks::UpdateDescriptor {
					descriptor_write,
					expected_set_version,
				} => {
					let current = self
						.descriptor_sets
						.get_mut(descriptor_write.descriptor_set.0 as usize)
						.is_some_and(|set| {
							if !descriptor_task_is_current(set, *descriptor_write, *expected_set_version) {
								return false;
							}
							let version = &mut set.sequence_versions[sequence_index as usize];
							*version = version.wrapping_add(1);
							true
						});
					if current {
						self.invalidate_descriptor_set_materializations(descriptor_write.descriptor_set, Some(sequence_index));
					}
				}
				Tasks::BuildBuffer(builder) => {
					let name = self.get_object_debug_name(builder.master.into());
					let previous = self.buffers.resource(builder.previous);
					let new_buffer_handle = self.create_buffer_internal(
						None,
						Some(builder.previous),
						name.as_deref(),
						previous.uses,
						previous.size,
						previous.access,
					);

					// Persistent-write buffers get their own per-frame staging buffer, fed from the shared source buffer.
					if let Some(source_handle) = builder.source {
						let size = self.buffers.resource(new_buffer_handle).size;
						let per_frame_staging = self.create_staging_buffer(name.as_deref(), size);
						let buffer = self.buffers.resource_mut(new_buffer_handle);
						buffer.staging = Some(per_frame_staging);
						buffer.source = Some(source_handle);
					}
					self.bump_descriptor_sequence_epoch(sequence_index);
				}
				Tasks::ResizeImage { handle, extent } => {
					let handle = self.image_handle_for_sequence(*handle, sequence_index as usize);
					self.resize_image_internal(handle, *extent, sequence_index);
				}
			}

			false
		});

		tasks.append(&mut self.tasks);
		self.tasks = tasks;
	}

	/// Destroys every retired object regardless of frame progress; callers must know the device is idle.
	pub(crate) fn destroy_retired_resources(&mut self) {
		for task in std::mem::take(&mut self.tasks) {
			self.run_destruction_task(task.task());
		}
	}

	pub(crate) fn get_syncronizer_handles(
		&self,
		synchroizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> SmallVec<[SynchronizerHandle; MAX_FRAMES_IN_FLIGHT]> {
		SynchronizerHandle(synchroizer_handle.0).get_all(&self.synchronizers)
	}

	pub(crate) fn wait_for_synchronizer(&self, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		for handle in self.get_syncronizer_handles(synchronizer_handle) {
			let synchronizer = &self.synchronizers[handle.0 as usize];
			// Non-frame submissions only signal one sequence's fence, so the other sequences may never have been submitted.
			if synchronizer.armed {
				unsafe {
					self.device
						.wait_for_fences(&[synchronizer.fence], true, u64::MAX)
						.expect("Failed to wait for Vulkan synchronizer. The most likely cause is that the submitted fence is invalid or the device was lost.");
				}
			}
		}
	}
}
