use super::*;

impl Context {
	pub(super) fn new(device: &Device) -> Result<Self, &'static str> {
		let mut device = device.inner.clone().ok_or("Failed to create a Vulkan context. The most likely cause is that a detached device was used as the primary graphics device.")?;
		let memory_properties = device.memory_properties;
		let queues = std::mem::take(&mut device.queues);
		let settings = device.settings.clone();
		let swapchain_native_supports_formatless_storage_write = device.swapchain_native_supports_formatless_storage_write;
		let swapchain_proxy_supports_formatless_storage_write = device.swapchain_proxy_supports_formatless_storage_write;

		let mut context = Context {
			device,

			memory_properties,

			frames: 2, // Assuming double buffering

			queues,
			allocations: Vec::new(),
			buffers: ResourceCollection::with_capacity(1024),
			images: Vec::with_capacity(512),
			samplers: Vec::with_capacity(128),
			pipeline_layouts: Vec::with_capacity(64),
			pipeline_layout_indices: HashMap::with_capacity(64),
			descriptor_sets: Vec::with_capacity(512),
			descriptor_heaps: None,
			descriptor_materializations: Vec::with_capacity(512),
			materialization_indices: HashMap::with_capacity(512),
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

			settings,

			states: HashMap::with_capacity(4096),
			buffer_states: HashMap::with_capacity(4096),

			pending_buffer_syncs: HashSet::with_capacity(128),
			pending_image_syncs: HashSet::with_capacity(128),

			persistent_write_dynamic_buffers: Vec::with_capacity(64),
			swapchain_native_supports_formatless_storage_write,
			swapchain_proxy_supports_formatless_storage_write,

			tasks: Vec::with_capacity(1024),

			#[cfg(debug_assertions)]
			names: HashMap::with_capacity(4096),
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

	/// Creates a detached pipeline-capable factory for compatibility with the previous pipeline factory API.
	pub fn create_pipeline_factory(&self) -> Option<crate::implementation::Factory> {
		self.create_factory()
	}

	pub(crate) fn create_command_buffer(
		&mut self,
		name: Option<&str>,
		queue_handle: graphics_hardware_interface::QueueHandle,
	) -> graphics_hardware_interface::CommandBufferHandle {
		let command_buffer_handle = graphics_hardware_interface::CommandBufferHandle(self.command_buffers.len() as u64);

		let queue = &self.queues[queue_handle.0 as usize];
		let vk_queue = queue.vk_queue.clone();

		let command_buffers = (0..self.frames)
			.map(|_| {
				let command_pool_create_info = vk::CommandPoolCreateInfo::default()
					.flags(vk::CommandPoolCreateFlags::TRANSIENT)
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

				let command_buffers = unsafe {
					self.device
						.allocate_command_buffers(&command_buffer_allocate_info)
						.expect("No command buffer")
				};

				let command_buffer = command_buffers[0];

				self.set_name(command_buffer, name);

				CommandBufferInternal {
					vk_queue: vk_queue.clone(),
					command_pool,
					command_buffer,
				}
			})
			.collect::<Vec<_>>();

		self.command_buffers.push(CommandBuffer {
			queue_handle,
			frames: command_buffers,
		});

		command_buffer_handle
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
			let set_index = descriptor_write.descriptor_set.0 as usize;
			let retained = crate::vulkan::descriptor_set::RetainedDescriptor {
				descriptor: descriptor_write.descriptor,
				frame_offset: descriptor_write.frame_offset.unwrap_or(0),
			};
			let descriptor_set = self.descriptor_sets.get_mut(set_index).expect(
				"Invalid Vulkan descriptor set. The most likely cause is that the write used a handle from another context.",
			);
			let previous = descriptor_set
				.descriptors
				.get(&descriptor_write.slot)
				.and_then(|elements| elements.get(&descriptor_write.array_element))
				.copied();
			if previous == Some(retained) {
				continue;
			}

			descriptor_set
				.descriptors
				.entry(descriptor_write.slot)
				.or_default()
				.insert(descriptor_write.array_element, retained);
			descriptor_set.version = descriptor_set.version.wrapping_add(1);
			let expected_set_version = descriptor_set.version;
			self.invalidate_descriptor_set_materializations(descriptor_write.descriptor_set, None);
			self.add_task_to_all_frames(Tasks::UpdateDescriptor {
				descriptor_write,
				expected_set_version,
			});
		}
	}

	pub(crate) fn create_command_buffer_recording(
		&mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> crate::vulkan::CommandBufferRecording<'_> {
		let pending_buffers = &mut self.pending_buffer_syncs;

		let buffer_copies: Vec<BufferCopy> = pending_buffers
			.drain()
			.filter_map(|e| {
				let dst_buffer_handle = e;

				let dst_buffer = self.buffers.resource(dst_buffer_handle);

				let src_buffer_handle = dst_buffer.staging?;

				Some(BufferCopy::new(src_buffer_handle, 0, dst_buffer_handle, 0, dst_buffer.size))
			})
			.collect();

		let pending_images = &mut self.pending_image_syncs;

		let image_copies: Vec<ImageCopy> = pending_images
			.drain()
			.map(|e| {
				let dst_image_handle = e;

				let dst_image = &self.images[dst_image_handle.0 as usize];

				ImageCopy::new(dst_image_handle, 0, dst_image_handle, 0, dst_image.size)
			})
			.collect();

		let mut recording = CommandBufferRecording::new(self, command_buffer_handle, None);

		recording.sync_buffers(buffer_copies.iter().copied());
		recording.sync_textures(image_copies.iter().copied());

		recording
	}

	pub(crate) fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.buffers.get_single(buffer_handle).unwrap().device_address
	}

	pub(crate) fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		let buffer = self.buffers.get_single(buffer_handle.into()).unwrap();
		let buffer = buffer.staging.map(|staging| self.buffers.resource(staging)).unwrap_or(buffer);
		unsafe { std::mem::transmute(buffer.pointer) }
	}

	pub(crate) fn get_mut_buffer_slice<T: Copy>(
		&self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> &'static mut T {
		let buffer = self.buffers.get_single(buffer_handle.into()).unwrap();
		let buffer = buffer.staging.map(|staging| self.buffers.resource(staging)).unwrap_or(buffer);

		unsafe { std::mem::transmute(buffer.pointer) }
	}

	pub(crate) fn sync_buffer(&mut self, buffer_handle: impl Into<crate::BaseBufferHandle>) {
		let buffer_handle = buffer_handle.into();
		let handle = BufferHandle(buffer_handle.0);

		if self.buffers.resource(handle).staging.is_some() {
			self.pending_buffer_syncs.insert(handle);
		}
	}

	pub(crate) fn get_texture_slice_mut(&self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
		let texture = &self.images[texture_handle.0 .0 as usize];
		let size = texture.size;
		assert!(
			texture.staging_buffer.is_some(),
			"Attempted to map an image without a staging buffer. The most likely cause is that the image was created without CPU-visible access but is being written from the CPU."
		);
		let pointer = texture.pointer.expect(
			"Attempted to map an image without a CPU-visible pointer. The most likely cause is that image resize or creation did not rebuild the host-visible staging allocation."
		);
		assert!(
			size > 0,
			"Attempted to map a zero-sized image. The most likely cause is that the image was used before receiving a valid extent."
		);

		unsafe { std::slice::from_raw_parts_mut(pointer, size) }
	}

	pub(crate) fn sync_texture(&mut self, image_handle: crate::ImageHandle) {
		let image_handle = ImageHandle(image_handle.0 .0);
		let image = &self.images[image_handle.0 as usize];
		assert!(
			image.staging_buffer.is_some(),
			"Attempted to sync an image without a staging buffer. The most likely cause is that CPU-side image uploads are being requested for a GPU-only image."
		);

		self.pending_image_syncs.insert(image_handle);
	}

	pub(crate) fn write_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let handles = ImageHandle(image_handle.0 .0).get_all(&self.images);

		let handle = handles[0];

		let texture = handle.access(&self.images);

		let pointer = texture.pointer.unwrap();
		let size = texture.size;

		let slice = unsafe { std::slice::from_raw_parts_mut(pointer, size) };

		f(slice);

		self.pending_image_syncs.insert(handle);
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
				matrix: [
					transform[0][0],
					transform[0][1],
					transform[0][2],
					transform[0][3],
					transform[1][0],
					transform[1][1],
					transform[1][2],
					transform[1][3],
					transform[2][0],
					transform[2][1],
					transform[2][2],
					transform[2][3],
				],
			},
			instance_custom_index_and_mask: vk::Packed24_8::new(custom_index as u32, mask),
			instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
				sbt_record_offset as u32,
				vk::GeometryInstanceFlagsKHR::FORCE_OPAQUE.as_raw() as u8,
			),
			acceleration_structure_reference: vk::AccelerationStructureReferenceKHR { device_handle: address },
		};

		let instance_buffer = self.buffers.get_single(instances_buffer).unwrap();

		let instance_buffer_slice = unsafe {
			std::slice::from_raw_parts_mut(
				instance_buffer.pointer as *mut vk::AccelerationStructureInstanceKHR,
				instance_buffer.size / std::mem::size_of::<vk::AccelerationStructureInstanceKHR>(),
			)
		};

		instance_buffer_slice[instance_index] = instance;
	}

	pub(crate) fn write_sbt_entry(
		&mut self,
		sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
		shader_handle: graphics_hardware_interface::ShaderHandle,
	) {
		let pipeline = &self.pipelines[pipeline_handle.0 as usize];
		let shader_handles = pipeline.shader_handles.clone();

		let buffer = self.buffers.get_single(sbt_buffer_handle).unwrap();
		let buffer = self.buffers.resource(buffer.staging.unwrap());

		(unsafe { std::slice::from_raw_parts_mut(buffer.pointer, buffer.size) })[sbt_record_offset..sbt_record_offset + 32]
			.copy_from_slice(shader_handles.get(&shader_handle).unwrap());
	}

	pub(crate) fn resize_buffer<T: Copy>(
		&mut self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
		size: usize,
	) {
		let buffer_handle: graphics_hardware_interface::BaseBufferHandle = buffer_handle.into();
		let buffer_handle = BufferHandle(buffer_handle.0);

		self.resize_buffer_internal(buffer_handle, size);
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
			format,
			proxy_format,
			supported_image_usage,
			uses_proxy_images,
			native_image_usage,
			vk_swapchain,
		) = self
			.device
			.build_swapchain(window_os_handles, presentation_mode, fallback_extent, uses);

		let swapchain_handle = graphics_hardware_interface::SwapchainHandle(self.swapchains.len() as u64);

		let mut acquire_synchronizers = [SynchronizerHandle(!0u64); MAX_FRAMES_IN_FLIGHT];

		for i in 0..self.frames {
			let synchronizer = self.create_synchronizer_internal(Some("Swapchain Acquire Sync"), true);
			acquire_synchronizers[i as usize] = synchronizer;
		}

		let vk_images = unsafe {
			self.device
				.swapchain
				.get_swapchain_images(vk_swapchain)
				.expect("No swapchain images found.")
		};
		let image_count = vk_images.len() as u32;

		let mut submit_synchronizers = [SynchronizerHandle(!0u64); MAX_SWAPCHAIN_IMAGES];

		for i in 0..image_count {
			let synchronizer = self.create_synchronizer_internal(Some("Swapchain Submit Sync"), true);
			submit_synchronizers[i as usize] = synchronizer;
		}

		let mut native_images = [ImageHandle(!0u64); MAX_SWAPCHAIN_IMAGES];
		let native_uses = if uses_proxy_images {
			crate::Uses::TransferDestination
		} else {
			uses
		};

		for (i, vk_image) in vk_images.iter().enumerate() {
			let previous = if i > 0 { Some(native_images[i - 1]) } else { None };
			native_images[i] =
				self.create_swapchain_image(*vk_image, crate::Formats::BGRAsRGB, native_uses, native_image_usage, previous);
		}

		let mut images = native_images;

		if uses_proxy_images {
			let proxy_extent = Extent::rectangle(extent.width, extent.height);
			let proxy_uses = uses | crate::Uses::TransferSource | crate::Uses::TransferDestination;

			for i in 0..image_count as usize {
				let previous = if i > 0 { Some(images[i - 1]) } else { None };
				images[i] = self.create_image_internal(
					None,
					previous,
					Some("Swapchain Proxy Image"),
					proxy_format,
					crate::DeviceAccesses::DeviceOnly,
					None,
					false,
					proxy_extent,
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
			format,
			supported_usage_flags: supported_image_usage,
			acquired_image_indices: [0; MAX_FRAMES_IN_FLIGHT],
			min_image_count,
			max_image_count: image_count,
			vk_present_mode,
		});

		swapchain_handle
	}

	#[cfg(any())]
	pub(super) fn get_swapchain_image(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
		uses: crate::Uses,
	) -> (graphics_hardware_interface::ImageHandle, crate::Formats) {
		let (format, supported_usage_flags, fallback_extent) = {
			let swapchain = &self.swapchains[swapchain_handle.0 as usize];
			(swapchain.format, swapchain.supported_usage_flags, swapchain.extent)
		};
		let proxy_format = crate::Formats::BGRAu8;

		let requested_usage = into_vk_image_usage_flags(uses, format);
		let use_proxy = self.swapchain_needs_proxy(supported_usage_flags, requested_usage, uses);

		let (image, format) = if use_proxy {
			self.validate_swapchain_proxy_format(uses);

			let proxy_uses = uses | crate::Uses::TransferSource | crate::Uses::TransferDestination;
			let (needs_rebuild, native_images, max_image_count) = {
				let swapchain = &self.swapchains[swapchain_handle.0 as usize];
				(
					!swapchain.uses_proxy_images || !swapchain.proxy_uses.contains(uses),
					swapchain.native_images,
					swapchain.max_image_count,
				)
			};

			if needs_rebuild {
				let extent = Extent::rectangle(fallback_extent.width, fallback_extent.height);
				let mut proxies = native_images;

				for image_index in 0..max_image_count as usize {
					let previous = if image_index > 0 {
						Some(proxies[image_index - 1])
					} else {
						None
					};
					proxies[image_index] = self.create_image_internal(
						None,
						previous,
						Some("Swapchain Proxy Image"),
						proxy_format,
						crate::DeviceAccesses::DeviceOnly,
						None,
						false,
						extent,
						proxy_uses,
						1,
					);
				}

				let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
				swapchain.images = proxies;
				swapchain.uses_proxy_images = true;
				swapchain.proxy_uses = uses;
			}

			let swapchain = &self.swapchains[swapchain_handle.0 as usize];
			(
				graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle(swapchain.images[0].0)),
				proxy_format,
			)
		} else {
			let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
			swapchain.images = swapchain.native_images;
			swapchain.uses_proxy_images = false;
			swapchain.proxy_uses = crate::Uses::empty();
			(
				graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle(
					swapchain.native_images[0].0,
				)),
				format,
			)
		};

		(image, format)
	}

	/// Invalidates one completed image readback allocation before exposing its mapped bytes.
	pub(crate) fn get_image_data<'a>(
		&'a self,
		texture_copy_handle: graphics_hardware_interface::TextureCopyHandle,
	) -> &'a [u8] {
		let image = &self.images[texture_copy_handle.0 as usize];
		let pointer = image.pointer.expect(
			"Texture data is unavailable. The most likely cause is that the image was not created with CPU read access.",
		);
		let allocation_handle = image.staging_allocation.expect(
			"Texture readback allocation is unavailable. The most likely cause is that the image staging buffer was not created.",
		);
		let allocation = &self.allocations[allocation_handle.0 as usize];
		let mapped_range = vk::MappedMemoryRange::default()
			.memory(allocation.memory)
			.offset(0)
			.size(vk::WHOLE_SIZE);
		unsafe {
			self.device.invalidate_mapped_memory_ranges(&[mapped_range]).expect(
				"Vulkan image readback invalidation failed. The most likely cause is device loss or an invalid staging allocation.",
			);
		}

		assert!(
			!pointer.is_null(),
			"Texture data pointer is null. The most likely cause is that Vulkan failed to map the readback allocation."
		);
		unsafe { std::slice::from_raw_parts::<'a, u8>(pointer, image.size) }
	}

	pub(crate) fn start_frame<'a>(
		&'a mut self,
		index: u64,
		synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> crate::queue::StartedFrame<Frame<'a>> {
		let frame_index = index;
		let sequence_index = (index % u64::from(self.frames)) as u8;

		let synchronizer_handles = self.get_syncronizer_handles(synchronizer_handle);
		let synchronizer = &self.synchronizers[synchronizer_handles[sequence_index as usize].0 as usize];

		let per_cycle_wait_ms = 1;
		let wait_warning_time_threshold = 8;
		let mut timeout_count = 0;

		loop {
			match unsafe {
				self.device
					.device
					.wait_for_fences(&[synchronizer.fence], true, per_cycle_wait_ms * 1000000)
			} {
				Ok(_) => break,
				Err(vk::Result::TIMEOUT) => {
					let name = self.get_object_debug_name(synchronizer_handle.into());

					if timeout_count * per_cycle_wait_ms >= wait_warning_time_threshold && timeout_count % 500 == 0 {
						println!(
							"Stuck waiting for fence ({}) for {} ms at frame {index}. There is a potential issue with synchronization.",
							name.as_deref().unwrap_or("unknown"),
							per_cycle_wait_ms * timeout_count
						);
					}

					timeout_count += 1;

					continue;
				}
				Err(_) => panic!("Failed to wait for fence"),
			}
		}

		unsafe {
			self.device
				.device
				.reset_fences(&[synchronizer.fence])
				.expect("No fence reset");
		}

		let frame_key = FrameKey {
			frame_index,
			sequence_index,
		};
		let completed_frame = crate::queue::completed_frame_key(index, self.frames);

		// The sequence fence has completed, so immutable snapshots retired by earlier updates can now be reused.
		self.release_retired_descriptor_materializations(frame_key.sequence_index);

		// Build lazy resources before the frame may need them.
		self.process_tasks(frame_key.sequence_index);
		// Tasks processed after the fence can retire prior-frame snapshots immediately.
		self.release_retired_descriptor_materializations(frame_key.sequence_index);

		crate::queue::StartedFrame::new(Frame::new(self, frame_key), completed_frame)
	}

	pub(super) fn swapchain_needs_proxy(
		&self,
		supported_usage_flags: vk::ImageUsageFlags,
		requested_usage: vk::ImageUsageFlags,
		uses: crate::Uses,
	) -> bool {
		!supported_usage_flags.contains(requested_usage)
			|| (uses.contains(crate::Uses::Storage) && !self.swapchain_native_supports_formatless_storage_write)
	}

	pub(super) fn validate_swapchain_proxy_format(&self, uses: crate::Uses) {
		if uses.contains(crate::Uses::Storage) && !self.swapchain_proxy_supports_formatless_storage_write {
			panic!(
				"Failed to create a Vulkan swapchain proxy image. The most likely cause is that VK_FORMAT_B8G8R8A8_UNORM does not support storage image writes without format."
			);
		}
	}

	pub(super) fn is_swapchain_image_root(&self, handle: graphics_hardware_interface::ImageHandle) -> bool {
		self.swapchains
			.iter()
			.any(|swapchain| swapchain.images[0].0 == handle.0 .0 || swapchain.native_images[0].0 == handle.0 .0)
	}

	pub(super) fn get_swapchain_image_for_sequence(
		&self,
		handle: graphics_hardware_interface::ImageHandle,
		sequence_index: usize,
	) -> Option<ImageHandle> {
		self.swapchains.iter().find_map(|swapchain| {
			let acquired_image_index = swapchain.acquired_image_indices[sequence_index] as usize;

			if swapchain.images[0].0 == handle.0 .0 {
				Some(swapchain.images[acquired_image_index])
			} else if swapchain.native_images[0].0 == handle.0 .0 {
				Some(swapchain.native_images[acquired_image_index])
			} else {
				None
			}
		})
	}

	pub(super) fn resolve_descriptor_image_handle(
		&self,
		handle: graphics_hardware_interface::ImageHandle,
		sequence_index: usize,
		frame_offset: i32,
	) -> ImageHandle {
		let frame_index = self.frame_index_with_offset(sequence_index, frame_offset);

		if let Some(handle) = self.get_swapchain_image_for_sequence(handle, frame_index) {
			return handle;
		}

		self.image_handle_for_sequence(ImageHandle(handle.0 .0), frame_index)
	}

	/// Resolves a frame sequence and offset into a valid per-frame resource index.
	pub(super) fn frame_index_with_offset(&self, sequence_index: usize, frame_offset: i32) -> usize {
		crate::frame_resources::frame_index_with_offset(sequence_index, frame_offset, self.frames as usize)
	}

	/// Selects the frame-local image handle for a chained image resource.
	pub(super) fn image_handle_for_sequence(&self, handle: ImageHandle, sequence_index: usize) -> ImageHandle {
		let root_handle = handle.root(&self.images);
		let handles = root_handle.get_all(&self.images);
		handles[sequence_index.rem_euclid(handles.len())]
	}

	/// Removes cached keys immediately while retaining their immutable bytes until the owning frame sequence completes.
	pub(super) fn retire_descriptor_materializations(&mut self, predicate: impl Fn(&MaterializationKey) -> bool) {
		let stale = self
			.materialization_indices
			.iter()
			.filter(|(key, _)| predicate(key))
			.map(|(key, handle)| (key.clone(), *handle))
			.collect::<Vec<_>>();
		for (key, handle) in stale {
			self.materialization_indices.remove(&key);
			self.retired_materializations[key.sequence_index as usize].push(handle);
		}
	}

	pub(super) fn invalidate_descriptor_set_materializations(
		&mut self,
		descriptor_set: graphics_hardware_interface::DescriptorSetHandle,
		sequence_index: Option<u8>,
	) {
		self.retire_descriptor_materializations(|key| {
			sequence_index.is_none_or(|sequence_index| key.sequence_index == sequence_index)
				&& key.descriptor_sets.iter().any(|(handle, ..)| *handle == descriptor_set)
		});
	}

	pub(super) fn bump_descriptor_sequence_epoch(&mut self, sequence_index: u8) {
		let epoch = &mut self.descriptor_sequence_epochs[sequence_index as usize];
		*epoch = epoch.wrapping_add(1);
		self.retire_descriptor_materializations(|key| {
			key.resource_epochs
				.iter()
				.any(|(resource_sequence, _)| *resource_sequence == sequence_index)
		});
	}

	/// Reclaims stale heap ranges only after the sequence fence proves that no command buffer still references them.
	pub(super) fn release_retired_descriptor_materializations(&mut self, sequence_index: u8) {
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
		retired.clear();
		self.retired_materializations[sequence_index] = retired;
	}

	/// Executes deferred resource work and invalidates only the frame-local immutable descriptor snapshots that may reference it.
	pub(crate) fn process_tasks(&mut self, sequence_index: u8) {
		let mut tasks = self.tasks.split_off(0);

		// TODO: optimize consecutive tasks such as two resize tasks

		tasks.retain(|e| {
			if let Some(e) = e.frame() {
				if e != sequence_index {
					return true;
				}
			}

			// Helps debug issues related to use after delete cases.
			let disable_deletions = false;

			match e.task() {
				Tasks::DeleteVulkanImage { handle } => {
					if disable_deletions {
						return true;
					}
					unsafe {
						self.device.destroy_image(*handle, None);
					}
				}
				Tasks::DeleteVulkanImageView { handle } => {
					if disable_deletions {
						return true;
					}
					unsafe {
						self.device.destroy_image_view(*handle, None);
					}
				}
				Tasks::DeleteVulkanBuffer { handle } => {
					if disable_deletions {
						return true;
					}
					unsafe {
						self.device.destroy_buffer(*handle, None);
					}
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
				Tasks::BuildImage(builder) => {
					let name = self.get_object_debug_name(builder.master.into());

					let previous_image = builder.previous.access(&self.images);

					self.create_image_internal(
						None,
						Some(builder.previous),
						name.as_ref().map(|e| e.as_str()),
						previous_image.format_,
						previous_image.access,
						previous_image.layers,
						previous_image.cube_compatible,
						previous_image.cube_array_compatible,
						previous_image.extent,
						previous_image.uses,
						previous_image.mip_levels,
					);
					self.bump_descriptor_sequence_epoch(sequence_index);
				}
				Tasks::BuildBuffer(builder) => {
					let name = self.get_object_debug_name(builder.master.into());

					let previous_buffer = self.buffers.resource(builder.previous);

					let new_buffer_handle = self.create_buffer_internal(
						None,
						Some(builder.previous),
						name.as_ref().map(|e| e.as_str()),
						previous_buffer.uses,
						previous_buffer.size,
						previous_buffer.access,
					);

					// When PERSISTENT_WRITE is enabled and this buffer has a source,
					// create a per-frame staging buffer and point the new buffer's
					// staging and source fields accordingly.
					if let Some(source_handle) = builder.source {
						let size = self.buffers.resource(new_buffer_handle).size;
						let per_frame_staging = self.create_staging_buffer(name.as_ref().map(|e| e.as_str()), size);
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

		self.tasks = tasks;
	}

	pub(super) fn get_syncronizer_handles(
		&self,
		synchroizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> SmallVec<[SynchronizerHandle; MAX_FRAMES_IN_FLIGHT]> {
		SynchronizerHandle(synchroizer_handle.0).get_all(&self.synchronizers)
	}

	pub(crate) fn wait_for_synchronizer(&self, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		let handles = self.get_syncronizer_handles(synchronizer_handle);
		for handle in handles {
			let synchronizer = &self.synchronizers[handle.0 as usize];
			unsafe {
				self.device
					.wait_for_fences(&[synchronizer.fence], true, u64::MAX)
					.expect("Failed to wait for Vulkan synchronizer. The most likely cause is that the submitted fence is invalid or the device was lost.");
			}
		}
	}
}
