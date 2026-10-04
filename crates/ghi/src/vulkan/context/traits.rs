use super::*;
use crate::Next as _;

impl std::ops::Deref for Context {
	type Target = InnerDevice;

	fn deref(&self) -> &Self::Target {
		&self.device
	}
}

impl std::ops::DerefMut for Context {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.device
	}
}

/// Returns the last handle of the chain that follows every resource with a successor.
fn chain_tails<H: HandleLike>(collection: &[H::Item]) -> Vec<H> {
	collection
		.iter()
		.filter_map(|item| item.next()?.get_all(collection).last().copied())
		.collect()
}

impl Context {
	/// Creates an acceleration structure sized for `build_info` and returns its index.
	fn create_acceleration_structure(
		&mut self,
		name: Option<&str>,
		build_info: &vk::AccelerationStructureBuildGeometryInfoKHR,
		primitive_count: u32,
	) -> u64 {
		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();
		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(
				vk::AccelerationStructureBuildTypeKHR::DEVICE,
				build_info,
				Some(&[primitive_count]),
				&mut size_info,
			);
		}

		let (buffer, ..) = self.create_dedicated_buffer(
			None,
			size_info.acceleration_structure_size as usize,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
			crate::DeviceAccesses::GpuWrite,
		);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer.resource)
			.size(size_info.acceleration_structure_size)
			.offset(0)
			.ty(build_info.ty);
		let acceleration_structure = unsafe {
			self.acceleration_structure
				.create_acceleration_structure(&create_info, None)
				.expect("No acceleration structure")
		};
		self.set_name(acceleration_structure, name);

		self.acceleration_structures.push(AccelerationStructure {
			acceleration_structure,
			buffer: buffer.resource,
		});
		(self.acceleration_structures.len() - 1) as u64
	}
}

impl crate::context::Context for Context {
	type Queue<'a>
		= crate::vulkan::queue::Queue<'a>
	where
		Self: 'a;

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		self.device.has_errors()
	}

	fn supports_bc_texture_compression(&self) -> bool {
		true
	}

	fn queue<'a>(&'a mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> Self::Queue<'a> {
		crate::vulkan::queue::Queue {
			device: self,
			queue_handle,
		}
	}

	fn create_command_buffer_recording(
		&mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> impl crate::command_buffer::CommandBufferRecording + crate::command_buffer::CommonCommandBufferMode {
		Context::create_command_buffer_recording(self, command_buffer_handle)
	}

	fn set_frames_in_flight(&mut self, frames: u8) {
		if self.frames == frames {
			return;
		}

		assert!(
			frames <= MAX_FRAMES_IN_FLIGHT as u8,
			"Cannot set frames in flight to more than {MAX_FRAMES_IN_FLIGHT}"
		);
		assert!(
			frames > self.frames,
			"Failed to reduce the frames in flight. The most likely cause is that shrinking per-frame resources is not implemented."
		);

		for image_handle in chain_tails::<ImageHandle>(&self.images) {
			let builder = self.images[image_handle.0 as usize].builder();
			self.create_image_internal(Some(image_handle), &builder);
		}

		for synchronizer_handle in chain_tails::<SynchronizerHandle>(&self.synchronizers) {
			let root = synchronizer_handle.root(&self.synchronizers);
			let name = self.get_object_debug_name(graphics_hardware_interface::SynchronizerHandle(root.0).into());
			let signaled = self.synchronizers[synchronizer_handle.0 as usize].signaled;
			let new_synchronizer = self.create_synchronizer_internal(name.as_deref(), signaled);
			self.synchronizers[synchronizer_handle.0 as usize].next = Some(new_synchronizer);
		}

		for i in 0..self.command_buffers.len() {
			let queue_handle = self.command_buffers[i].queue_handle;
			let frame = self.create_command_buffer_frame(queue_handle, vk::CommandPoolCreateFlags::empty(), None);
			self.command_buffers[i].frames.push(frame);
		}

		self.frames = frames;
	}

	fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.buffers.get_single(buffer_handle).unwrap().device_address
	}

	fn get_buffer_slice<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> &T {
		// SAFETY: Typed handles preserve the allocation's type and the buffer remains mapped while the context lives.
		unsafe { &*self.typed_buffer_pointer(buffer_handle) }
	}

	fn get_mut_buffer_slice<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> &mut T {
		// SAFETY: Typed handles preserve the allocation's type and `&mut self` guarantees exclusive CPU access.
		unsafe { &mut *self.typed_buffer_pointer(buffer_handle) }
	}

	unsafe fn transfer_buffer_mapping<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> crate::buffer::Mapping {
		let pointer = self.typed_buffer_pointer(buffer_handle);
		// SAFETY: The caller accepts the lifetime and exclusivity requirements documented by this method.
		unsafe { crate::buffer::Mapping::from_raw_parts(pointer.cast::<u8>(), T::byte_count(pointer)) }
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<crate::BaseBufferHandle>) {
		let handle = BufferHandle(buffer_handle.into().0);
		if self.buffers.resource(handle).staging.is_some() {
			self.pending_buffer_syncs.insert(handle);
		}
	}

	fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::ImageHandle) -> &mut [u8] {
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

	fn sync_texture(&mut self, image_handle: crate::ImageHandle) {
		let image_handle = ImageHandle(image_handle.0.0);
		assert!(
			self.images[image_handle.0 as usize].staging_buffer.is_some(),
			"Attempted to sync an image without a staging buffer. The most likely cause is that CPU-side image uploads are being requested for a GPU-only image."
		);
		self.pending_image_syncs.insert((image_handle, None));
	}

	fn write_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let handle = ImageHandle(image_handle.0.0);
		let texture = handle.access(&self.images);
		f(unsafe { std::slice::from_raw_parts_mut(texture.pointer.unwrap().0, texture.size) });
		self.pending_image_syncs.insert((handle, None));
	}

	/// Retains flat descriptor writes and schedules frame-local snapshot refreshes without touching command-visible heap memory.
	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
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

	fn write_instance(
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

	fn write_sbt_entry(
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

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: graphics_hardware_interface::PresentationModes,
		fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		let vk_surface = self.create_vulkan_surface(window_os_handles);
		let vk_present_mode = match presentation_mode {
			graphics_hardware_interface::PresentationModes::FIFO => vk::PresentModeKHR::FIFO,
			graphics_hardware_interface::PresentationModes::Inmediate => vk::PresentModeKHR::IMMEDIATE,
			graphics_hardware_interface::PresentationModes::Mailbox => vk::PresentModeKHR::MAILBOX,
		};
		let vk_surface_capabilities = self.query_swapchain_surface_capabilities(vk_surface, vk_present_mode);
		let extent = InnerDevice::swapchain_extent(
			&vk_surface_capabilities,
			vk::Extent2D::default()
				.width(fallback_extent.width())
				.height(fallback_extent.height()),
		);

		// Native images that cannot take the requested uses are written through proxy images and copied before present.
		let requested_image_usage = into_vk_image_usage_flags(uses, crate::Formats::BGRAsRGB);
		let supported_image_usage = vk_surface_capabilities.supported_usage_flags;
		let uses_storage = uses.contains(crate::Uses::Storage);
		let uses_proxy_images = !supported_image_usage.contains(requested_image_usage)
			|| uses_storage && !self.swapchain_native_supports_formatless_storage_write;
		let native_image_usage = if uses_proxy_images {
			assert!(
				!uses_storage || self.swapchain_proxy_supports_formatless_storage_write,
				"Failed to create swapchain storage proxy image. The most likely cause is that the selected Vulkan device does not support storage writes without format for the swapchain proxy format."
			);
			assert!(
				supported_image_usage.contains(vk::ImageUsageFlags::TRANSFER_DST),
				"Failed to create swapchain fallback copy path. The most likely cause is that the surface does not support transfer destination usage for swapchain images."
			);
			vk::ImageUsageFlags::TRANSFER_DST
		} else {
			requested_image_usage
		};
		let vk_swapchain = self.create_vulkan_swapchain(
			vk_surface,
			vk_present_mode,
			&vk_surface_capabilities,
			extent,
			native_image_usage,
			vk::SwapchainKHR::null(),
		);

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
			let proxy = image::Builder::new(
				crate::Formats::BGRAu8,
				uses | crate::Uses::TransferSource | crate::Uses::TransferDestination,
			)
			.name("Swapchain Proxy Image")
			.extent(Extent::rectangle(extent.width, extent.height));
			for i in 0..vk_images.len() {
				let previous = i.checked_sub(1).map(|previous| images[previous]);
				images[i] = self.create_image_internal(previous, &proxy);
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
			uses,
			native_image_usage,
			needs_recreation: false,
			acquired_image_indices: [0; MAX_FRAMES_IN_FLIGHT],
			acquire_wait_stages: [vk::PipelineStageFlags2::NONE; MAX_FRAMES_IN_FLIGHT],
			min_image_count: vk_surface_capabilities.min_image_count,
			max_image_count: vk_images.len() as u32,
			vk_present_mode,
			present_interval: None,
			next_present_slot: None,
		});

		swapchain_handle
	}

	/// Acquires the swapchain image that `frame` will present before the frame is started.
	///
	/// The sequence fence is waited (not reset) first: the acquire semaphore of this sequence was last waited by the
	/// submission `frames_in_flight` frames ago, and that wait must have executed before the semaphore can be signaled
	/// again. [`crate::queue::Queue::start_frame`] later sees the same fence signaled, so its wait returns at once before
	/// the reset.
	fn acquire_swapchain_image(
		&mut self,
		frame: crate::queue::FrameRequest<'_>,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		let sequence_index = (frame.index % u64::from(self.frames)) as u8;
		self.wait_for_private_synchronizer(self.get_syncronizer_handles(frame.synchronizer)[sequence_index as usize]);
		self.acquire_swapchain_image_for_sequence(sequence_index, swapchain_handle)
	}

	fn set_present_interval(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
		interval: Option<std::time::Duration>,
	) {
		self.swapchains[swapchain_handle.0 as usize].present_interval = interval;
	}

	/// Waits for the transfer's submission, copies one mapped transfer result, and releases its dedicated staging resources.
	fn get_image_data(
		&mut self,
		texture_copy_handle: graphics_hardware_interface::TextureCopyHandle,
	) -> Result<crate::TextureReadback, crate::TextureTransferError> {
		// Only the transfer's own submission has to finish; other work in flight on this context keeps running.
		let (_, synchronizer) = self.texture_readbacks.submitted(texture_copy_handle)?;
		match synchronizer {
			Some(synchronizer) => self.wait_for_private_synchronizer(synchronizer),
			None => self.device.wait(),
		}
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

	/// Grows every frame copy of a dynamic buffer, its staging, and its persistent source to `size`.
	///
	/// Contents are discarded, matching the other backends. Replaced storage is destroyed only after the frames that
	/// may still read it have completed, so the resize is safe while earlier frames are in flight.
	fn resize_buffer<T: crate::Pod>(
		&mut self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
		size: usize,
	) {
		let master_handle: graphics_hardware_interface::BaseBufferHandle = buffer_handle.into();
		if self.buffers.resource(BufferHandle(master_handle.0)).size >= size {
			return;
		}

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

	fn start_frame_capture(&mut self) {}

	fn end_frame_capture(&mut self) {}

	fn wait_for_synchronizer(&mut self, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		for handle in self.get_syncronizer_handles(synchronizer_handle) {
			self.wait_for_private_synchronizer(handle);
		}
	}

	/// Returns whether every armed fence of the synchronizer has signaled, without blocking.
	fn poll_synchronizer(&mut self, synchronizer: graphics_hardware_interface::SynchronizerHandle) -> bool {
		self.get_syncronizer_handles(synchronizer).into_iter().all(|handle| {
			let synchronizer = &self.synchronizers[handle.0 as usize];
			// Non-frame submissions only signal one sequence's fence, so the other sequences may never have been submitted.
			!synchronizer.armed
				|| unsafe { self.device.get_fence_status(synchronizer.fence) }.expect(
					"Failed to query a Vulkan fence. The most likely cause is that the fence is invalid or the device was lost.",
				)
		})
	}

	fn wait(&mut self) {
		self.device.wait();
	}
}

impl crate::context::ContextCreate for Context {
	/// Creates a new allocation from a managed allocator for the underlying GPU allocations.
	fn create_allocation(
		&mut self,
		size: usize,
		_resource_uses: crate::Uses,
		resource_device_accesses: crate::DeviceAccesses,
	) -> graphics_hardware_interface::AllocationHandle {
		self.create_allocation_internal(size, None, resource_device_accesses).0
	}

	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> graphics_hardware_interface::MeshHandle {
		let index_offset = vertices.len().next_multiple_of(16);
		let (buffer, _, _, pointer) = self.create_dedicated_buffer(
			None,
			index_offset + indices.len(),
			vk::BufferUsageFlags::VERTEX_BUFFER
				| vk::BufferUsageFlags::INDEX_BUFFER
				| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
			crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		);

		unsafe {
			std::ptr::copy_nonoverlapping(vertices.as_ptr(), pointer, vertices.len());
			std::ptr::copy_nonoverlapping(indices.as_ptr(), pointer.add(index_offset), indices.len());
		}

		self.meshes.push(Mesh {
			buffer: buffer.resource,
			vertex_count,
			index_count,
			vertex_size: vertex_layout.iter().map(|element| element.format.size()).sum(),
		});
		graphics_hardware_interface::MeshHandle(self.meshes.len() as u64 - 1)
	}

	/// Creates a shader.
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let crate::shader::Sources::SPIRV(spirv) = shader_source_type else {
			return Err(());
		};
		if !spirv.as_ptr().is_aligned_to(align_of::<u32>()) {
			return Err(());
		}
		// SAFETY: shader was checked to be aligned to 4 bytes.
		let code = unsafe { std::slice::from_raw_parts(spirv.as_ptr() as *const u32, spirv.len() / 4) };

		let shader_module = unsafe {
			self.device
				.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(code), None)
				.unwrap()
		};

		let handle = graphics_hardware_interface::ShaderHandle(self.shaders.len() as u64);
		self.shaders.push(Shader {
			shader: shader_module,
			stage: stage.into(),
			shader_resource_descriptors: shader_resource_descriptors.into_iter().collect(),
		});
		self.set_name(shader_module, name);

		Ok(handle)
	}

	fn create_descriptor_set(&mut self, name: Option<&str>) -> graphics_hardware_interface::DescriptorSetHandle {
		let handle = graphics_hardware_interface::DescriptorSetHandle(self.descriptor_sets.len() as u64);
		self.descriptor_sets.push(DescriptorSet {
			next: None,
			version: 0,
			sequence_versions: [0; MAX_FRAMES_IN_FLIGHT],
			descriptors: HashMap::default(),
		});
		self.set_object_debug_name(name, graphics_hardware_interface::Handles::DescriptorSet(handle));
		handle
	}

	fn create_raster_pipeline(
		&mut self,
		builder: crate::pipelines::raster::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let (pipeline, layout) =
			build_raster_pipeline(&self.device, &self.device.descriptor_heap_properties, &self.shaders, builder);
		self.add_pipeline(pipeline, layout, HashMap::default())
	}

	fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let pipeline = build_compute_pipeline(&self.device, &self.device.descriptor_heap_properties, &self.shaders, builder);
		self.add_pipeline(pipeline.pipeline, pipeline.layout, HashMap::default())
	}

	fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let shaders = builder.shaders;
		let pipeline_layout = pipelines::shader_pipeline_layout(
			&self.shaders,
			shaders,
			builder.push_constant_ranges,
			&self.device.descriptor_heap_properties,
		);
		let stage_mappings = shaders
			.iter()
			.map(|stage| {
				let shader = &self.shaders[stage.handle.0 as usize];
				crate::vulkan::build_shader_mappings(&pipeline_layout, &shader.shader_resource_descriptors)
			})
			.collect::<Vec<_>>();
		let mut mapping_infos = stage_mappings
			.iter()
			.map(|mappings| vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(mappings))
			.collect::<Vec<_>>();
		let stages = shaders
			.iter()
			.zip(mapping_infos.iter_mut())
			.map(|(stage, mapping_info)| {
				vk::PipelineShaderStageCreateInfo::default()
					.push(mapping_info)
					.stage(stage.stage.into())
					.module(self.shaders[stage.handle.0 as usize].shader)
					.name(c"main")
			})
			.collect::<Vec<_>>();

		let groups = shaders
			.iter()
			.enumerate()
			.filter_map(|(i, shader)| {
				use vk::RayTracingShaderGroupTypeKHR as Group;

				use crate::ShaderTypes;

				let (i, unused) = (i as u32, vk::SHADER_UNUSED_KHR);
				let group = vk::RayTracingShaderGroupCreateInfoKHR::default()
					.general_shader(unused)
					.closest_hit_shader(unused)
					.any_hit_shader(unused)
					.intersection_shader(unused);
				Some(match shader.stage {
					ShaderTypes::RayGen | ShaderTypes::Miss | ShaderTypes::Callable => {
						group.ty(Group::GENERAL).general_shader(i)
					}
					ShaderTypes::ClosestHit => group.ty(Group::TRIANGLES_HIT_GROUP).closest_hit_shader(i),
					ShaderTypes::AnyHit => group.ty(Group::TRIANGLES_HIT_GROUP).any_hit_shader(i),
					ShaderTypes::Intersection => group.ty(Group::PROCEDURAL_HIT_GROUP).intersection_shader(i),
					_ => return None,
				})
			})
			.collect::<Vec<_>>();

		let mut descriptor_heap_flags =
			vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
		let create_info = vk::RayTracingPipelineCreateInfoKHR::default()
			.push(&mut descriptor_heap_flags)
			.layout(vk::PipelineLayout::null())
			.stages(&stages)
			.groups(&groups)
			.max_pipeline_ray_recursion_depth(1);

		let (pipeline, handle_buffer) = unsafe {
			let pipeline = self
				.ray_tracing_pipeline
				.create_ray_tracing_pipelines(
					vk::DeferredOperationKHR::null(),
					vk::PipelineCache::null(),
					&[create_info],
					None,
				)
				.expect("No ray tracing pipeline")[0];
			let handle_buffer = self
				.ray_tracing_pipeline
				.get_ray_tracing_shader_group_handles(pipeline, 0, groups.len() as u32, 32 * groups.len())
				.expect("Could not get ray tracing shader group handles");
			(pipeline, handle_buffer)
		};

		let shader_handles = shaders
			.iter()
			.enumerate()
			.map(|(i, shader)| (*shader.handle, handle_buffer[i * 32..(i + 1) * 32].try_into().unwrap()))
			.collect();

		self.add_pipeline(pipeline, pipeline_layout, shader_handles)
	}

	fn build_image(&mut self, builder: image::Builder) -> graphics_hardware_interface::ImageHandle {
		if builder.group.is_some() {
			crate::image_group::ImageGroups::validate_member(&builder);
		}

		let root_image_handle = self.create_image_internal(None, &builder);
		let instances = match builder.use_case {
			crate::UseCases::DYNAMIC => self.frames,
			crate::UseCases::STATIC => 1,
		};
		let mut previous = root_image_handle;
		for _ in 1..instances {
			previous = self.create_image_internal(Some(previous), &builder);
		}

		let handle =
			graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(root_image_handle.0));
		self.set_object_debug_name(builder.name, handle.into());
		// A member built without an extent has no memory until its group is placed.
		if let Some(group) = builder.group {
			self.image_groups.add_member(group, handle.into());
		}
		handle
	}

	fn create_image_group(&mut self, name: Option<&str>) -> crate::ImageGroupHandle {
		self.image_group_heaps.push(smallvec::SmallVec::new());
		self.image_groups.create(name)
	}

	fn build_sampler(&mut self, builder: sampler::Builder) -> crate::SamplerHandle {
		let filtering_mode = match builder.filtering_mode {
			crate::FilteringModes::Closest => vk::Filter::NEAREST,
			crate::FilteringModes::Linear => vk::Filter::LINEAR,
		};

		let mip_map_filter = match builder.mip_map_mode {
			crate::FilteringModes::Closest => vk::SamplerMipmapMode::NEAREST,
			crate::FilteringModes::Linear => vk::SamplerMipmapMode::LINEAR,
		};

		let address_mode = match builder.addressing_mode {
			crate::SamplerAddressingModes::Repeat => vk::SamplerAddressMode::REPEAT,
			crate::SamplerAddressingModes::Mirror => vk::SamplerAddressMode::MIRRORED_REPEAT,
			crate::SamplerAddressingModes::Clamp => vk::SamplerAddressMode::CLAMP_TO_EDGE,
			crate::SamplerAddressingModes::Border { .. } => vk::SamplerAddressMode::CLAMP_TO_BORDER,
		};

		let reduction_mode = match builder.reduction_mode {
			crate::SamplingReductionModes::WeightedAverage => vk::SamplerReductionMode::WEIGHTED_AVERAGE,
			crate::SamplingReductionModes::Min => vk::SamplerReductionMode::MIN,
			crate::SamplingReductionModes::Max => vk::SamplerReductionMode::MAX,
		};

		let handle = graphics_hardware_interface::SamplerHandle(self.samplers.len() as u64);
		self.samplers.push(Sampler {
			mag_filter: filtering_mode,
			min_filter: filtering_mode,
			mipmap_mode: mip_map_filter,
			address_mode,
			reduction_mode,
			anisotropy: builder.anisotropy,
			min_lod: builder.min_lod,
			max_lod: builder.max_lod,
		});
		handle
	}

	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::BaseBufferHandle {
		let access = crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead;
		let (buffer, allocation, device_address, pointer) = self.create_dedicated_buffer(
			name,
			max_instance_count as usize * std::mem::size_of::<vk::AccelerationStructureInstanceKHR>(),
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
				| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
			access,
		);

		self.buffers
			.add(Buffer {
				staging: None,
				source: None,
				buffer: buffer.resource,
				size: buffer.size,
				device_address,
				pointer: crate::vulkan::MappedMemoryPointer(pointer),
				allocation: Some(allocation),
				uses: crate::Uses::empty(),
				access,
			})
			.0
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		let geometries = [vk::AccelerationStructureGeometryKHR::default()
			.geometry_type(vk::GeometryTypeKHR::INSTANCES)
			.geometry(vk::AccelerationStructureGeometryDataKHR {
				instances: vk::AccelerationStructureGeometryInstancesDataKHR::default(),
			})];
		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.geometries(&geometries);

		graphics_hardware_interface::TopLevelAccelerationStructureHandle(self.create_acceleration_structure(
			name,
			&build_info,
			max_instance_count,
		))
	}

	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &graphics_hardware_interface::BottomLevelAccelerationStructure,
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		let (geometry, primitive_count) = match &description.description {
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_count,
				vertex_position_encoding,
				triangle_count,
				index_format,
			} => (
				vk::AccelerationStructureGeometryKHR::default()
					.flags(vk::GeometryFlagsKHR::OPAQUE)
					.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						triangles: vk::AccelerationStructureGeometryTrianglesDataKHR::default()
							.vertex_format(match vertex_position_encoding {
								crate::Encodings::FloatingPoint => vk::Format::R32G32B32_SFLOAT,
								_ => panic!("Invalid vertex position format"),
							})
							.max_vertex(*vertex_count - 1)
							.index_type(match index_format {
								crate::DataTypes::U8 => vk::IndexType::UINT8_EXT,
								crate::DataTypes::U16 => vk::IndexType::UINT16,
								crate::DataTypes::U32 => vk::IndexType::UINT32,
								_ => panic!("Invalid index format"),
							}),
					}),
				*triangle_count,
			),
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::AABB { transform_count } => (
				vk::AccelerationStructureGeometryKHR::default()
					.flags(vk::GeometryFlagsKHR::OPAQUE)
					.geometry_type(vk::GeometryTypeKHR::AABBS)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						aabbs: vk::AccelerationStructureGeometryAabbsDataKHR::default(),
					}),
				*transform_count,
			),
		};

		let geometries = [geometry];
		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
			.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
			.geometries(&geometries);

		graphics_hardware_interface::BottomLevelAccelerationStructureHandle(self.create_acceleration_structure(
			None,
			&build_info,
			primitive_count,
		))
	}

	fn build_buffer<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		builder: crate::buffer::Builder,
	) -> graphics_hardware_interface::BufferHandle<T> {
		let buffer_handle = self.create_buffer_internal(
			None,
			builder.name,
			builder.resource_uses,
			T::layout(builder.length).size(),
			builder.device_accesses,
		);
		graphics_hardware_interface::BufferHandle(
			graphics_hardware_interface::BaseBufferHandle::new(buffer_handle.0),
			std::marker::PhantomData,
		)
	}

	fn build_dynamic_buffer<T: crate::Pod>(&mut self, builder: crate::buffer::Builder) -> crate::DynamicBufferHandle<T> {
		let size = <T as crate::buffer::BufferContents>::layout(builder.length).size();
		let buffer_handle =
			self.create_buffer_internal(None, builder.name, builder.resource_uses, size, builder.device_accesses);
		let handle = graphics_hardware_interface::DynamicBufferHandle::<T>(
			graphics_hardware_interface::BaseBufferHandle::new(buffer_handle.0),
			std::marker::PhantomData,
		);

		let source = if crate::vulkan::buffer::PERSISTENT_WRITE
			&& builder.device_accesses.intersects(crate::DeviceAccesses::CpuWrite)
			&& !Self::uses_only_host_access(builder.device_accesses)
		{
			// The master buffer's existing staging buffer becomes the shared, persistent CPU-writable source buffer,
			// and a new per-frame staging buffer takes its place for frame 0.
			let source_handle = self
				.buffers
				.resource(buffer_handle)
				.staging
				.expect("CpuWrite dynamic buffer must have a staging buffer");
			let frame0_staging = self.create_staging_buffer(builder.name, size);
			let buffer = self.buffers.resource_mut(buffer_handle);
			buffer.staging = Some(frame0_staging);
			buffer.source = Some(source_handle);

			// Track this dynamic buffer for automatic per-frame memcpy.
			self.persistent_write_dynamic_buffers.push(handle.into());
			Some(source_handle)
		} else {
			None
		};

		for i in 1..self.frames {
			assert!(i < 2, "This does not support more than one deferred buffer!");
			self.tasks.push(Task::new(
				Tasks::BuildBuffer(BuildBuffer {
					previous: buffer_handle,
					master: handle.into(),
					source,
				}),
				Some(i),
			));
		}

		handle
	}

	fn build_dynamic_image(&mut self, builder: crate::image::Builder) -> crate::DynamicImageHandle {
		crate::DynamicImageHandle(self.build_image(builder.use_case(crate::UseCases::DYNAMIC)).0)
	}

	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> graphics_hardware_interface::SynchronizerHandle {
		let synchronizer_handle = graphics_hardware_interface::SynchronizerHandle(self.synchronizers.len() as u64);

		let mut previous: Option<SynchronizerHandle> = None;
		for _ in 0..self.frames {
			let handle = self.create_synchronizer_internal(name, signaled);
			if let Some(previous) = previous {
				self.synchronizers[previous.0 as usize].next = Some(handle);
			}
			previous = Some(handle);
		}

		self.set_object_debug_name(name, synchronizer_handle.into());
		synchronizer_handle
	}
}
