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
	/// Queues the upload of a new dynamic-buffer copy's zeroed staging bytes.
	///
	/// A frame uploads a copy only when it writes it, so without this a copy no frame wrote yet would expose
	/// uninitialized device memory to the GPU.
	fn queue_initial_upload(&mut self, handle: BufferHandle) {
		if self.buffers.resource(handle).staging.is_some() {
			self.pending_buffer_syncs.insert(handle);
		}
	}
}

impl crate::context::Context for Context {
	type Queue<'a>
		= crate::vulkan::queue::Queue<'a>
	where
		Self: 'a;

	fn has_errors(&self) -> bool {
		self.device.has_errors()
	}

	fn supports_ray_tracing(&self) -> bool {
		self.device.ray_tracing.is_some()
	}

	fn bottom_level_acceleration_structure_size(
		&self,
		handle: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) -> usize {
		self.acceleration_structures.bottom_level(handle).storage.size as usize
	}

	fn load_pipeline_cache(&self, data: Vec<u8>) {
		if let Some(pipeline_binaries) = &self.device.pipeline_binaries {
			pipeline_binaries.load(data);
		}
	}

	fn pipeline_cache_data<R>(&self, read: impl FnOnce(crate::pipelines::CacheData<'_>) -> R) -> R {
		match &self.device.pipeline_binaries {
			Some(pipeline_binaries) => pipeline_binaries.data(read),
			None => read(crate::pipelines::CacheData { loaded: &[], added: &[] }),
		}
	}

	fn counter_duration(&self, counter: graphics_hardware_interface::CounterHandle) -> Option<std::time::Duration> {
		self.counters.duration(counter)
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
		let mut changed_sets = SmallVec::<[graphics_hardware_interface::DescriptorSetHandle; 8]>::new();
		for &descriptor_write in descriptor_set_writes {
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
			if !changed_sets.contains(&descriptor_write.descriptor_set) {
				changed_sets.push(descriptor_write.descriptor_set);
			}
			self.add_task_to_all_frames(Tasks::UpdateDescriptor {
				descriptor_write,
				expected_set_version,
			});
		}

		// Snapshots of every changed set are retired in one pass over the cache instead of one pass per write.
		if !changed_sets.is_empty() {
			self.retire_descriptor_materializations(|key| {
				key.descriptor_sets.iter().any(|(handle, ..)| changed_sets.contains(handle))
			});
		}
	}

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: graphics_hardware_interface::PresentationModes,
		desired_image_count: Option<u8>,
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
			desired_image_count,
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
			desired_image_count,
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

	/// Grows every frame copy of a dynamic buffer and its staging to `size`.
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

		let frame_copies = self
			.buffers
			.chain(master_handle)
			.collect::<SmallVec<[BufferHandle; MAX_FRAMES_IN_FLIGHT]>>();
		for handle in frame_copies {
			let current = *self.buffers.resource(handle);
			let replacement = self.build_buffer_internal(name, current.uses, size, current.access);

			if let Some(staging_handle) = current.staging {
				self.retire_buffer_storage(staging_handle);
			}
			self.retire_buffer_storage(handle);
			*self.buffers.resource_mut(handle) = replacement;
			self.queue_initial_upload(handle);

			// The replacement has no GPU history; stale ranges would only add barriers against the retired buffer.
			self.states.remove(&crate::vulkan::Handles::Buffer(handle));
			self.buffer_states.remove(&crate::vulkan::Handles::Buffer(handle));
		}

		for sequence_index in 0..self.frames {
			self.bump_descriptor_sequence_epoch(sequence_index);
		}
	}

	fn wait_for_synchronizer(&mut self, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		for handle in self.get_syncronizer_handles(synchronizer_handle) {
			self.wait_for_private_synchronizer(handle);
		}
	}

	/// Returns whether every armed fence of the synchronizer has signaled, without blocking.
	fn poll_synchronizer(&mut self, synchronizer: graphics_hardware_interface::SynchronizerHandle) -> bool {
		// Non-frame submissions only signal one sequence's fence, so the other sequences may never have been submitted.
		self.get_syncronizer_handles(synchronizer)
			.into_iter()
			.all(|handle| self.private_synchronizer_signaled(handle))
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
		self.create_allocation_internal(size, None, resource_device_accesses, None).0
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
		// A shader is no native object anymore, so there is nothing to name.
		_name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		descriptor_sets: impl IntoIterator<Item = crate::shader::ShaderDescriptorSet>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let shader = Shader::new(
			shader_source_type,
			stage,
			descriptor_sets,
			self.device.pipeline_binaries.as_deref(),
		)?;
		self.shaders.push(shader);
		Ok(graphics_hardware_interface::ShaderHandle(self.shaders.len() as u64 - 1))
	}

	/// Creates a retained group of flat slots. This backend materializes native descriptors per pipeline, so it does
	/// not lay the set out from `layout` yet.
	fn create_descriptor_set(
		&mut self,
		name: Option<&str>,
		_layout: &crate::shader::DescriptorSetLayout,
	) -> graphics_hardware_interface::DescriptorSetHandle {
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
		let (pipeline, layout) = build_raster_pipeline(
			&self.device,
			&self.device.descriptor_heap_properties,
			self.device.pipeline_binaries.as_deref(),
			&self.shaders,
			builder,
		);
		self.add_pipeline(pipeline, layout)
	}

	fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let pipeline = build_compute_pipeline(
			&self.device,
			&self.device.descriptor_heap_properties,
			self.device.pipeline_binaries.as_deref(),
			&self.shaders,
			builder,
		);
		self.add_pipeline(pipeline.pipeline, pipeline.layout)
	}

	fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		self.create_ray_tracing_pipeline_internal(builder)
	}

	fn create_shader_binding_table(&mut self, name: Option<&str>) -> graphics_hardware_interface::ShaderBindingTableHandle {
		self.create_shader_binding_table_internal(name)
	}

	fn write_shader_binding_table(
		&mut self,
		table: graphics_hardware_interface::ShaderBindingTableHandle,
		pipeline: graphics_hardware_interface::PipelineHandle,
		hit_groups: &[u32],
	) {
		self.write_shader_binding_table_internal(table, pipeline, hit_groups);
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

	fn create_bottom_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		geometries: &[crate::rt::TriangleGeometry],
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		self.create_bottom_level(name, geometries)
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		self.create_top_level(name, max_instance_count)
	}

	fn destroy_bottom_level_acceleration_structure(
		&mut self,
		handle: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		self.destroy_bottom_level(handle);
	}

	fn destroy_top_level_acceleration_structure(
		&mut self,
		handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle,
	) {
		self.destroy_top_level(handle);
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

		// Create every frame sequence's copy up front, so a buffer created mid-frame never shares a copy with another
		// frame in flight, which the CPU could rewrite while that frame still reads it.
		let root = self.create_buffer_internal(None, builder.name, builder.resource_uses, size, builder.device_accesses);
		self.queue_initial_upload(root);
		let mut previous = root;
		for _ in 1..self.frames {
			previous = self.create_buffer_internal(
				Some(previous),
				builder.name,
				builder.resource_uses,
				size,
				builder.device_accesses,
			);
			self.queue_initial_upload(previous);
		}

		let handle = graphics_hardware_interface::DynamicBufferHandle::<T>(
			graphics_hardware_interface::BaseBufferHandle::new(root.0),
			std::marker::PhantomData,
		);
		self.set_object_debug_name(
			builder.name,
			graphics_hardware_interface::BaseBufferHandle::from(handle).into(),
		);
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

	/// Vulkan counters share one timestamp query pool, so a counter is only a slot owner and needs no native object.
	fn create_counter(&mut self, _name: Option<&str>) -> graphics_hardware_interface::CounterHandle {
		self.counters.create()
	}
}
