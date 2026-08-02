/// The `Context` struct owns Vulkan device state while presenting the GHI context API.
pub struct Context {
	pub(super) device: InnerDevice,

	pub(super) frames: u8,

	pub(super) queues: Vec<StoredQueue>,
	pub(super) buffers: ResourceCollection<Buffer, graphics_hardware_interface::BaseBufferHandle, BufferHandle>,
	pub(super) images: Vec<Image>,
	pub(super) samplers: Vec<Sampler>,
	pub(super) allocations: Vec<Allocation>,
	pub(super) pipeline_layouts: Vec<PipelineLayout>,
	pipeline_layout_indices: HashMap<PipelineLayoutKey, graphics_hardware_interface::PipelineLayoutHandle>,
	pub(super) descriptor_sets: Vec<DescriptorSet>,
	pub(super) descriptor_heaps: Option<DescriptorHeaps>,
	pub(super) descriptor_materializations: Vec<Option<DescriptorMaterialization>>,
	materialization_indices: HashMap<MaterializationKey, DescriptorMaterializationHandle>,
	retired_materializations: [Vec<DescriptorMaterializationHandle>; MAX_FRAMES_IN_FLIGHT],
	free_materialization_handles: Vec<DescriptorMaterializationHandle>,
	descriptor_sequence_epochs: [u64; MAX_FRAMES_IN_FLIGHT],
	pub(super) meshes: Vec<Mesh>,
	pub(super) acceleration_structures: Vec<AccelerationStructure>,
	pub(super) shaders: Vec<Shader>,
	pub(super) pipelines: Vec<Pipeline>,
	pub(super) command_buffers: Vec<CommandBuffer>,
	pub(super) synchronizers: Vec<Synchronizer>,
	pub(super) swapchains: Vec<Swapchain>,

	pub settings: crate::device::Features,

	pub(super) states: HashMap<super::Handles, TransitionState>,
	pub(super) buffer_states: HashMap<super::Handles, Vec<super::BufferTransitionState>>,

	/// Tracks pending buffer host to device, or device to host synchronization operations.
	pub(super) pending_buffer_syncs: HashSet<BufferHandle>,
	/// Tracks pending image host to device, or device to host synchronization operations.
	pub(super) pending_image_syncs: HashSet<ImageHandle>,

	/// Tracks all dynamic buffer master handles that use the persistent write mode.
	/// These buffers have their source buffer memcpy'd into the per-frame staging
	/// buffer every frame before GPU copies are issued.
	pub(super) persistent_write_dynamic_buffers: Vec<graphics_hardware_interface::BaseBufferHandle>,

	swapchain_native_supports_formatless_storage_write: bool,
	swapchain_proxy_supports_formatless_storage_write: bool,

	memory_properties: vk::PhysicalDeviceMemoryProperties,

	/// Stores the debug names for resources.
	/// Used when inspecting resources from a rendering debugger such as RenderDoc.
	#[cfg(debug_assertions)]
	pub names: HashMap<graphics_hardware_interface::Handles, String>,

	/// A queue of deferred tasks. Usually object deletions and resource updates.
	pub(crate) tasks: Vec<Task>,
}

/// Accepts deferred descriptor work only while both its payload and post-write version remain current.
fn descriptor_task_is_current(
	set: &DescriptorSet,
	descriptor_write: crate::descriptors::DescriptorWrite,
	expected_set_version: u64,
) -> bool {
	if set.version != expected_set_version {
		return false;
	}

	let frame_offset = descriptor_write.frame_offset.unwrap_or(0);
	set.descriptors
		.get(&descriptor_write.slot)
		.and_then(|elements| elements.get(&descriptor_write.array_element))
		.is_some_and(|current| current.descriptor == descriptor_write.descriptor && current.frame_offset == frame_offset)
}

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
		let texture = &self.images[texture_handle.0.0 as usize];
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
		let image_handle = ImageHandle(image_handle.0.0);
		let image = &self.images[image_handle.0 as usize];
		assert!(
			image.staging_buffer.is_some(),
			"Attempted to sync an image without a staging buffer. The most likely cause is that CPU-side image uploads are being requested for a GPU-only image."
		);

		self.pending_image_syncs.insert(image_handle);
	}

	pub(crate) fn write_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let handles = ImageHandle(image_handle.0.0).get_all(&self.images);

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
	fn get_swapchain_image(
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

	pub(crate) fn get_image_data<'a>(
		&'a self,
		texture_copy_handle: graphics_hardware_interface::TextureCopyHandle,
	) -> &'a [u8] {
		let image = &self.images[texture_copy_handle.0 as usize];

		let pointer = image.pointer.unwrap();
		let size = image.size;

		if pointer.is_null() {
			panic!("Texture data was requested but texture has no memory associated.");
		}

		let slice = unsafe { std::slice::from_raw_parts::<'a, u8>(pointer, size) };

		slice
	}

	pub(crate) fn start_frame<'a>(
		&'a mut self,
		index: u32,
		synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	) -> crate::queue::StartedFrame<Frame<'a>> {
		let frame_index = index;
		let sequence_index = (index % self.frames as u32) as u8;

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

	fn swapchain_needs_proxy(
		&self,
		supported_usage_flags: vk::ImageUsageFlags,
		requested_usage: vk::ImageUsageFlags,
		uses: crate::Uses,
	) -> bool {
		!supported_usage_flags.contains(requested_usage)
			|| (uses.contains(crate::Uses::Storage) && !self.swapchain_native_supports_formatless_storage_write)
	}

	fn validate_swapchain_proxy_format(&self, uses: crate::Uses) {
		if uses.contains(crate::Uses::Storage) && !self.swapchain_proxy_supports_formatless_storage_write {
			panic!(
				"Failed to create a Vulkan swapchain proxy image. The most likely cause is that VK_FORMAT_B8G8R8A8_UNORM does not support storage image writes without format."
			);
		}
	}

	fn is_swapchain_image_root(&self, handle: graphics_hardware_interface::ImageHandle) -> bool {
		self.swapchains
			.iter()
			.any(|swapchain| swapchain.images[0].0 == handle.0.0 || swapchain.native_images[0].0 == handle.0.0)
	}

	fn get_swapchain_image_for_sequence(
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

	fn resolve_descriptor_image_handle(
		&self,
		handle: graphics_hardware_interface::ImageHandle,
		sequence_index: usize,
		frame_offset: i32,
	) -> ImageHandle {
		let frame_index = self.frame_index_with_offset(sequence_index, frame_offset);

		if let Some(handle) = self.get_swapchain_image_for_sequence(handle, frame_index) {
			return handle;
		}

		self.image_handle_for_sequence(ImageHandle(handle.0.0), frame_index)
	}

	/// Resolves a frame sequence and offset into a valid per-frame resource index.
	fn frame_index_with_offset(&self, sequence_index: usize, frame_offset: i32) -> usize {
		crate::frame_resources::frame_index_with_offset(sequence_index, frame_offset, self.frames as usize)
	}

	/// Selects the frame-local image handle for a chained image resource.
	fn image_handle_for_sequence(&self, handle: ImageHandle, sequence_index: usize) -> ImageHandle {
		let root_handle = handle.root(&self.images);
		let handles = root_handle.get_all(&self.images);
		handles[sequence_index.rem_euclid(handles.len())]
	}

	/// Removes cached keys immediately while retaining their immutable bytes until the owning frame sequence completes.
	fn retire_descriptor_materializations(&mut self, predicate: impl Fn(&MaterializationKey) -> bool) {
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

	fn invalidate_descriptor_set_materializations(
		&mut self,
		descriptor_set: graphics_hardware_interface::DescriptorSetHandle,
		sequence_index: Option<u8>,
	) {
		self.retire_descriptor_materializations(|key| {
			sequence_index.is_none_or(|sequence_index| key.sequence_index == sequence_index)
				&& key.descriptor_sets.iter().any(|(handle, ..)| *handle == descriptor_set)
		});
	}

	fn bump_descriptor_sequence_epoch(&mut self, sequence_index: u8) {
		let epoch = &mut self.descriptor_sequence_epochs[sequence_index as usize];
		*epoch = epoch.wrapping_add(1);
		self.retire_descriptor_materializations(|key| {
			key.resource_epochs
				.iter()
				.any(|(resource_sequence, _)| *resource_sequence == sequence_index)
		});
	}

	/// Reclaims stale heap ranges only after the sequence fence proves that no command buffer still references them.
	fn release_retired_descriptor_materializations(&mut self, sequence_index: u8) {
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

	/// Builds a graphics pipeline description whose shader stages use flat descriptor-heap mappings.
	fn create_vulkan_graphics_pipeline_create_info<'a, R>(
		&'a mut self,
		builder: crate::pipelines::raster::Builder,
		after_build: impl FnOnce(
			&'a mut Self,
			crate::pipelines::raster::Builder,
			graphics_hardware_interface::PipelineLayoutHandle,
			vk::GraphicsPipelineCreateInfo,
		) -> R,
	) -> R {
		let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
			.render_pass(vk::RenderPass::null()) // We use a null render pass because of VK_KHR_dynamic_rendering
		;

		let pipeline_layout_handle =
			self.get_or_create_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		let pipeline_layout = &self.pipeline_layouts[pipeline_layout_handle.0 as usize];

		let pipeline_create_info = pipeline_create_info.layout(vk::PipelineLayout::null());

		let mut vertex_input_attribute_descriptions = vec![];

		let mut offset_per_binding = [0, 0, 0, 0, 0, 0, 0, 0]; // Assume 8 bindings max

		for (i, vertex_element) in builder.vertex_elements.iter().enumerate() {
			let ve = vk::VertexInputAttributeDescription::default()
				.binding(vertex_element.binding)
				.location(i as u32)
				.format(vertex_element.format.into())
				.offset(offset_per_binding[vertex_element.binding as usize]);

			vertex_input_attribute_descriptions.push(ve);

			offset_per_binding[vertex_element.binding as usize] += vertex_element.format.size() as u32;
		}

		let vertex_binding_descriptions = if let Some(max_binding) = builder.vertex_elements.iter().map(|ve| ve.binding).max() {
			let max_binding = max_binding as usize + 1;

			let mut vertex_binding_descriptions = Vec::with_capacity(max_binding);

			for i in 0..max_binding {
				vertex_binding_descriptions.push(
					vk::VertexInputBindingDescription::default()
						.binding(i as u32)
						.stride(offset_per_binding[i as usize])
						.input_rate(vk::VertexInputRate::VERTEX),
				)
			}

			vertex_binding_descriptions
		} else {
			Vec::new()
		};

		let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
			.vertex_attribute_descriptions(&vertex_input_attribute_descriptions)
			.vertex_binding_descriptions(&vertex_binding_descriptions);

		let pipeline_create_info = pipeline_create_info.vertex_input_state(&vertex_input_state);

		let mut specialization_entries_buffer = Vec::<u8>::with_capacity(256);
		let mut entries = [vk::SpecializationMapEntry::default(); 32];
		let mut entry_count = 0;
		let specilization_info_count = 0;

		let stage_mappings = builder
			.shaders
			.iter()
			.map(|stage| {
				let shader = &self.shaders[stage.handle.0 as usize];
				crate::vulkan::build_shader_mappings(pipeline_layout, &shader.shader_resource_descriptors)
			})
			.collect::<Vec<_>>();
		let mut mapping_infos = stage_mappings
			.iter()
			.map(|mappings| vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(mappings))
			.collect::<Vec<_>>();
		let stages = builder
			.shaders
			.iter()
			.zip(mapping_infos.iter_mut())
			.map(|(stage, mapping_info)| {
				for entry in stage.specialization_map.iter() {
					specialization_entries_buffer.extend_from_slice(entry.get_data());

					entries[entry_count] = vk::SpecializationMapEntry::default()
						.constant_id(entry.get_constant_id())
						.size(entry.get_size())
						.offset(specialization_entries_buffer.len() as u32);

					entry_count += 1;
				}

				let shader = &self.shaders[stage.handle.0 as usize];

				assert!(specilization_info_count == 0);

				vk::PipelineShaderStageCreateInfo::default()
					.push(mapping_info)
					.stage(to_shader_stage_flags(stage.stage))
					.module(shader.shader)
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
			})
			.collect::<Vec<_>>();

		let pipeline_create_info = pipeline_create_info.stages(&stages);

		let pipeline_color_blend_attachments = builder
			.render_targets
			.iter()
			.filter(|a| !a.format.is_depth())
			.map(|attachment| {
				let blend_state =
					vk::PipelineColorBlendAttachmentState::default().color_write_mask(vk::ColorComponentFlags::RGBA);

				match attachment.blend {
					crate::pipelines::raster::BlendMode::None => blend_state
						.blend_enable(false)
						.src_color_blend_factor(vk::BlendFactor::ONE)
						.src_alpha_blend_factor(vk::BlendFactor::ONE)
						.dst_color_blend_factor(vk::BlendFactor::ZERO)
						.dst_alpha_blend_factor(vk::BlendFactor::ZERO)
						.color_blend_op(vk::BlendOp::ADD)
						.alpha_blend_op(vk::BlendOp::ADD),
					crate::pipelines::raster::BlendMode::Alpha => blend_state
						.blend_enable(true)
						.src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
						.src_alpha_blend_factor(vk::BlendFactor::ONE)
						.dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
						.dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
						.color_blend_op(vk::BlendOp::ADD)
						.alpha_blend_op(vk::BlendOp::ADD),
				}
			})
			.collect::<Vec<_>>();

		let color_attachement_formats: Vec<vk::Format> = builder
			.render_targets
			.iter()
			.filter(|a| !a.format.is_depth())
			.map(|a| to_format(a.format))
			.collect::<Vec<_>>();

		let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
			.logic_op_enable(false)
			.logic_op(vk::LogicOp::COPY)
			.attachments(&pipeline_color_blend_attachments)
			.blend_constants([0.0, 0.0, 0.0, 0.0]);

		let has_depth = builder.render_targets.iter().find(|attachment| attachment.format.is_depth());
		let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
			.color_attachment_formats(&color_attachement_formats)
			.depth_attachment_format(if let Some(depth_attachment) = has_depth {
				to_format(depth_attachment.format)
			} else {
				vk::Format::UNDEFINED
			});

		let pipeline_create_info = pipeline_create_info.color_blend_state(&color_blend_state);

		let depth_stencil_state = vk::PipelineDepthStencilStateCreateInfo::default()
			.depth_test_enable(true)
			.depth_write_enable(true)
			.depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL)
			.depth_bounds_test_enable(false)
			.stencil_test_enable(false)
			.front(vk::StencilOpState::default())
			.back(vk::StencilOpState::default());

		let pipeline_create_info = if has_depth {
			pipeline_create_info.depth_stencil_state(&depth_stencil_state)
		} else {
			pipeline_create_info
		};

		let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
			.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
			.primitive_restart_enable(false);

		let pipeline_create_info = pipeline_create_info.input_assembly_state(&input_assembly_state);

		let viewports = [vk::Viewport::default()
			.x(0.0)
			.y(9.0)
			.width(16.0)
			.height(9.0)
			.min_depth(0.0)
			.max_depth(1.0)];

		let scissors = [vk::Rect2D::default()
			.offset(vk::Offset2D { x: 0, y: 0 })
			.extent(vk::Extent2D { width: 16, height: 9 })];

		let viewport_state = vk::PipelineViewportStateCreateInfo::default()
			.viewports(&viewports)
			.scissors(&scissors);

		let dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
			.dynamic_states(&[vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR]);

		let rasterization_state = vk::PipelineRasterizationStateCreateInfo::default()
			.depth_clamp_enable(false)
			.rasterizer_discard_enable(false)
			.polygon_mode(vk::PolygonMode::FILL)
			.cull_mode(vk::CullModeFlags::BACK)
			.front_face(vk::FrontFace::CLOCKWISE)
			.depth_bias_enable(false)
			.depth_bias_constant_factor(0.0)
			.depth_bias_clamp(0.0)
			.depth_bias_slope_factor(0.0)
			.line_width(1.0);

		let multisample_state = vk::PipelineMultisampleStateCreateInfo::default()
			.sample_shading_enable(false)
			.rasterization_samples(vk::SampleCountFlags::TYPE_1)
			.min_sample_shading(1.0)
			.alpha_to_coverage_enable(false)
			.alpha_to_one_enable(false);

		let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
			.topology(vk::PrimitiveTopology::TRIANGLE_LIST)
			.primitive_restart_enable(false);

		let pipeline_create_info = pipeline_create_info
			.viewport_state(&viewport_state)
			.dynamic_state(&dynamic_state)
			.rasterization_state(&rasterization_state)
			.multisample_state(&multisample_state)
			.input_assembly_state(&input_assembly_state);
		let mut descriptor_heap_flags =
			vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
		let pipeline_create_info = pipeline_create_info
			.push(&mut descriptor_heap_flags)
			.push(&mut rendering_info);

		after_build(self, builder, pipeline_layout_handle, pipeline_create_info)
	}

	/// Returns a cached descriptor-heap layout derived from shader resource metadata.
	fn get_or_create_pipeline_layout(
		&mut self,
		shaders: &[crate::pipelines::ShaderParameter],
		push_constant_ranges: &[crate::pipelines::PushConstantRange],
	) -> graphics_hardware_interface::PipelineLayoutHandle {
		let stage_resources = shaders
			.iter()
			.map(|shader_parameter| {
				let shader = &self.shaders[shader_parameter.handle.0 as usize];
				(shader.stage, shader.shader_resource_descriptors.clone())
			})
			.collect::<Vec<_>>();
		let layout = crate::vulkan::build_pipeline_layout(
			&stage_resources,
			push_constant_ranges,
			&self.device.descriptor_heap_properties,
		);
		let key = PipelineLayoutKey::new(&layout);
		if let Some(handle) = self.pipeline_layout_indices.get(&key) {
			return *handle;
		}

		let handle = graphics_hardware_interface::PipelineLayoutHandle(self.pipeline_layouts.len() as u64);
		self.pipeline_layouts.push(layout);
		self.pipeline_layout_indices.insert(key, handle);
		handle
	}

	fn create_vulkan_pipeline(
		&mut self,
		builder: crate::pipelines::raster::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		self.create_vulkan_graphics_pipeline_create_info(
			builder,
			|this, _builder, pipeline_layout_handle, pipeline_create_info| {
				let pipeline_create_infos = [pipeline_create_info];

				let pipelines = unsafe {
					this.device
						.create_graphics_pipelines(vk::PipelineCache::null(), &pipeline_create_infos, None)
						.expect("No pipeline")
				};

				let pipeline = pipelines[0];

				let handle = graphics_hardware_interface::PipelineHandle(this.pipelines.len() as u64);

				this.pipelines.push(Pipeline {
					pipeline,
					layout: pipeline_layout_handle,
					shader_handles: HashMap::new(),
				});

				handle
			},
		)
	}

	pub(super) fn get_image_subresource_layout(
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

	fn bind_vulkan_buffer_memory(
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

	fn bind_host_vulkan_buffer_memory(
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

	fn bind_vulkan_texture_memory(
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
	fn create_swapchain_image(
		&mut self,
		vk_image: vk::Image,
		format: crate::Formats,
		uses: crate::Uses,
		image_usage_flags: vk::ImageUsageFlags,
		previous: Option<ImageHandle>,
	) -> ImageHandle {
		let root_handle = ImageHandle(self.images.len() as u64);
		let root_image = {
			let mut image_views = [vk::ImageView::null(); 8];

			image_views[0] = self.create_vulkan_image_view(None, &vk_image, format, image_usage_flags, 1, 0, None);

			Image {
				next: None,
				size: 0,
				staging_buffer: None,
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
	fn create_allocation_internal(
		&mut self,
		size: usize,
		memory_bits: Option<u32>,
		device_accesses: crate::DeviceAccesses,
	) -> (graphics_hardware_interface::AllocationHandle, Option<*mut u8>) {
		let memory_property_flags = {
			let mut memory_property_flags = vk::MemoryPropertyFlags::empty();

			memory_property_flags |= if device_accesses.contains(crate::DeviceAccesses::CpuRead) {
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

	fn uses_only_host_access(device_accesses: crate::DeviceAccesses) -> bool {
		device_accesses.intersects(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite)
			&& !device_accesses.intersects(crate::DeviceAccesses::GpuRead | crate::DeviceAccesses::GpuWrite)
	}

	/// Creates a Vulkan buffer, allocates memory for it, binds the memory, and returns the tracked buffer object.
	fn create_bound_buffer(
		&mut self,
		name: Option<&str>,
		size: usize,
		vk_usage_flags: vk::BufferUsageFlags,
		allocation_accesses: crate::DeviceAccesses,
		buffer_accesses: crate::DeviceAccesses,
		resource_uses: crate::Uses,
		mip_levels: u32,
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
	fn build_buffer_internal(
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

			let staging_buffer =
				self.create_bound_buffer(name, size, vk_usage_flags, device_access, device_accesses, resource_uses);

			let (_, handle) = self.buffers.add(staging_buffer);

			Some(handle)
		} else {
			None
		};

		buffer.staging = staging;
		buffer
	}

	/// Builds a buffer and returns its handle.
	fn create_buffer_internal(
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
	fn create_staging_buffer(&mut self, name: Option<&str>, size: usize) -> BufferHandle {
		let vk_usage_flags = vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
		let device_access = crate::DeviceAccesses::GpuRead | crate::DeviceAccesses::CpuWrite;

		let buffer = self.create_bound_buffer(name, size, vk_usage_flags, device_access, device_access, crate::Uses::empty());
		let (_, handle) = self.buffers.add(buffer);

		handle
	}

	fn build_image_internal(
		&mut self,
		next: Option<ImageHandle>,
		name: Option<&str>,
		format: crate::Formats,
		device_accesses: crate::DeviceAccesses,
		array_layers: Option<NonZeroU32>,
		extent: Extent,
		resource_uses: crate::Uses,
		mip_levels: u32,
	) -> Image {
		let size = extent.width() as usize * extent.height().max(1) as usize * extent.depth().max(1) as usize * format.size();

		if extent.width() == 0 {
			return Image {
				next,
				size: 0,
				staging_buffer: None,
				pointer: None,
				image: vk::Image::null(),
				full_image_view: vk::ImageView::null(),
				image_views: [vk::ImageView::null(); 8],
				extent,
				access: device_accesses,
				format: to_format(format),
				format_: format,
				uses: resource_uses,
				layers: array_layers,
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

		let texture_creation_result =
			self.create_vulkan_texture(name, extent, format, resource_uses | transfer_uses, mip_levels, array_layers);

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

		let (staging_buffer, pointer) = if uses_cpu_staging {
			let vk_buffer_usage_flags = if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
				vk::BufferUsageFlags::TRANSFER_DST
			} else {
				vk::BufferUsageFlags::TRANSFER_SRC
			};

			let device_accesses = if device_accesses.intersects(crate::DeviceAccesses::CpuRead) {
				crate::DeviceAccesses::DeviceToHost
			} else {
				crate::DeviceAccesses::HostToDevice
			};

			let buffer_creation_result = self.create_vulkan_buffer(name, size, vk_buffer_usage_flags);
			let (allocation_handle, _) = self.create_allocation_internal(
				buffer_creation_result.size,
				buffer_creation_result.memory_flags.into(),
				device_accesses,
			);
			let pointer = self.bind_host_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

			(Some(buffer_creation_result.resource), Some(pointer))
		} else {
			(None, None)
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
			let mut image_views = [vk::ImageView::null(); 8];

			if let Some(l) = array_layers.map(|e| e.get()) {
				for i in 0..l {
					image_views[i as usize] = self.create_vulkan_image_view(
						name,
						&texture_creation_result.resource,
						format,
						image_usage_flags,
						mip_levels,
						i,
						NonZeroU32::new(1),
					);
				}
			} else {
				image_views[0] = self.create_vulkan_image_view(
					name,
					&texture_creation_result.resource,
					format,
					image_usage_flags,
					mip_levels,
					0,
					None,
				);
			}

			image_views
		} else {
			[vk::ImageView::null(); 8]
		};

		Image {
			next,
			size,
			staging_buffer,
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

	fn create_synchronizer_internal(&mut self, name: Option<&str>, signaled: bool) -> SynchronizerHandle {
		let synchronizer_handle = SynchronizerHandle(self.synchronizers.len() as u64);

		self.synchronizers.push(Synchronizer {
			next: None,
			signaled,
			fence: self.create_vulkan_fence(signaled),
			semaphore: self.create_vulkan_semaphore(name, signaled),
		});

		synchronizer_handle
	}

	fn resize_buffer_internal(&mut self, buffer_handle: BufferHandle, size: usize) {
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

		for image_view in image.image_views {
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
			extent,
			image.uses,
			image.mip_levels,
		);

		self.images[image_handle.0 as usize] = new_image;

		if let Some(state) = self.states.get_mut(&super::Handles::Image(image_handle)) {
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

	/// Creates one persistently mapped heap whose retired immutable snapshot ranges can be reused after fence completion.
	fn create_descriptor_heap_arena(
		&mut self,
		name: &str,
		size: u64,
		heap_alignment: u64,
		descriptor_alignment: u64,
		reserved_size: u64,
	) -> DescriptorHeapArena {
		assert!(
			size <= u32::MAX as u64,
			"Vulkan descriptor heap exceeds the 32-bit push-index address space. The most likely cause is an implementation reservation larger than the mapping interface supports.",
		);
		let buffer_size = size
			.checked_add(heap_alignment)
			.expect("Vulkan descriptor heap size overflowed. The most likely cause is invalid descriptor-heap properties.");
		let creation = self.create_vulkan_buffer(
			Some(name),
			usize::try_from(buffer_size).expect(
				"Vulkan descriptor heap exceeds addressable host memory. The most likely cause is invalid device limits.",
			),
			vk::BufferUsageFlags::DESCRIPTOR_HEAP_EXT | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);
		let host_device_local = self
			.memory_properties
			.memory_types
			.iter()
			.enumerate()
			.take(self.memory_properties.memory_type_count as usize)
			.any(|(index, memory_type)| {
				creation.memory_flags & (1 << index) != 0
					&& memory_type.property_flags.contains(
						vk::MemoryPropertyFlags::HOST_VISIBLE
							| vk::MemoryPropertyFlags::HOST_COHERENT
							| vk::MemoryPropertyFlags::DEVICE_LOCAL,
					)
			});
		let device_accesses = crate::DeviceAccesses::CpuWrite
			| if host_device_local {
				crate::DeviceAccesses::GpuRead
			} else {
				crate::DeviceAccesses::empty()
			};
		let (allocation, _) = self.create_allocation_internal(creation.size, creation.memory_flags.into(), device_accesses);
		let (device_address, pointer) = self.bind_vulkan_buffer_memory(&creation, allocation, 0);
		let aligned_address = crate::vulkan::align_up(device_address, heap_alignment);
		let base_offset = aligned_address - device_address;
		assert!(base_offset + size <= buffer_size);

		let application_offset = crate::vulkan::align_up(reserved_size, descriptor_alignment);
		assert!(
			application_offset < size,
			"Vulkan descriptor heap has no application-owned range. The most likely cause is that the implementation reservation consumes the device's maximum heap size.",
		);
		DescriptorHeapArena {
			buffer: creation.resource,
			pointer: unsafe { pointer.add(base_offset as usize) },
			device_address: aligned_address,
			size,
			reserved_size,
			free_ranges: vec![super::DescriptorHeapRange {
				offset: application_offset,
				size: size - application_offset,
			}],
		}
	}

	/// Allocates the long-lived resource and sampler heaps once for the context.
	fn create_descriptor_heaps(&mut self) -> DescriptorHeaps {
		const RESOURCE_HEAP_TARGET_SIZE: u64 = 64 * 1024 * 1024;
		const SAMPLER_HEAP_TARGET_SIZE: u64 = 1024 * 1024;
		let properties = self.device.descriptor_heap_properties;
		let resource_descriptor_alignment = properties
			.buffer_descriptor_alignment
			.max(properties.image_descriptor_alignment);
		let resource_minimum =
			crate::vulkan::align_up(properties.min_resource_heap_reserved_range, resource_descriptor_alignment)
				.checked_add(properties.buffer_descriptor_size.max(properties.image_descriptor_size))
				.unwrap();
		let sampler_minimum = crate::vulkan::align_up(
			properties.min_sampler_heap_reserved_range,
			properties.sampler_descriptor_alignment,
		)
		.checked_add(properties.sampler_descriptor_size)
		.unwrap();
		assert!(
			resource_minimum <= properties.max_resource_heap_size,
			"Vulkan resource heap limits are inconsistent. The most likely cause is that the implementation reservation leaves no room for one resource descriptor.",
		);
		assert!(
			sampler_minimum <= properties.max_sampler_heap_size,
			"Vulkan sampler heap limits are inconsistent. The most likely cause is that the implementation reservation leaves no room for one sampler descriptor.",
		);
		let resource_size = RESOURCE_HEAP_TARGET_SIZE
			.max(resource_minimum)
			.min(properties.max_resource_heap_size);
		let sampler_size = SAMPLER_HEAP_TARGET_SIZE
			.max(sampler_minimum)
			.min(properties.max_sampler_heap_size);

		DescriptorHeaps {
			resource: self.create_descriptor_heap_arena(
				"GHI Resource Descriptor Heap",
				resource_size,
				properties.resource_heap_alignment,
				resource_descriptor_alignment,
				properties.min_resource_heap_reserved_range,
			),
			sampler: self.create_descriptor_heap_arena(
				"GHI Sampler Descriptor Heap",
				sampler_size,
				properties.sampler_heap_alignment,
				properties.sampler_descriptor_alignment,
				properties.min_sampler_heap_reserved_range,
			),
		}
	}

	fn descriptors_at_slot<'a>(
		&'a self,
		sets: &[graphics_hardware_interface::DescriptorSetHandle],
		slot: crate::shader::ResourceSlot,
	) -> Option<&'a HashMap<u32, crate::vulkan::descriptor_set::RetainedDescriptor>> {
		sets.iter()
			.find_map(|set| self.descriptor_sets[set.0 as usize].descriptors.get(&slot))
	}

	fn descriptor_matches_kind(descriptor: crate::descriptors::WriteData, kind: crate::shader::ResourceKind) -> bool {
		match descriptor {
			crate::descriptors::WriteData::Buffer { .. } => matches!(
				kind,
				crate::shader::ResourceKind::UniformBuffer | crate::shader::ResourceKind::StorageBuffer
			),
			crate::descriptors::WriteData::Image { .. } | crate::descriptors::WriteData::Swapchain(_) => matches!(
				kind,
				crate::shader::ResourceKind::SampledImage
					| crate::shader::ResourceKind::StorageImage
					| crate::shader::ResourceKind::InputAttachment
			),
			crate::descriptors::WriteData::CombinedImageSampler { .. } => {
				kind == crate::shader::ResourceKind::CombinedImageSampler
			}
			crate::descriptors::WriteData::Sampler(_) => kind == crate::shader::ResourceKind::Sampler,
			crate::descriptors::WriteData::AccelerationStructure { .. } => {
				kind == crate::shader::ResourceKind::AccelerationStructure
			}
			crate::descriptors::WriteData::StaticSamplers | crate::descriptors::WriteData::CombinedImageSamplerArray => false,
		}
	}

	/// Compares the native resource and image layout consumed by two materialized descriptors.
	fn descriptors_consume_same_resource(left: Descriptor, right: Descriptor) -> bool {
		match (left, right) {
			(Descriptor::Buffer { buffer: left, .. }, Descriptor::Buffer { buffer: right, .. }) => left == right,
			(
				Descriptor::Image {
					image: left,
					layout: left_layout,
				},
				Descriptor::Image {
					image: right,
					layout: right_layout,
				},
			)
			| (
				Descriptor::Image {
					image: left,
					layout: left_layout,
				},
				Descriptor::CombinedImageSampler {
					image: right,
					layout: right_layout,
					..
				},
			)
			| (
				Descriptor::CombinedImageSampler {
					image: left,
					layout: left_layout,
					..
				},
				Descriptor::Image {
					image: right,
					layout: right_layout,
				},
			)
			| (
				Descriptor::CombinedImageSampler {
					image: left,
					layout: left_layout,
					..
				},
				Descriptor::CombinedImageSampler {
					image: right,
					layout: right_layout,
					..
				},
			) => left == right && left_layout == right_layout,
			(Descriptor::AccelerationStructure { handle: left }, Descriptor::AccelerationStructure { handle: right }) => {
				left == right
			}
			_ => false,
		}
	}

	/// Validates one complete logical set union against the active flat pipeline layout.
	fn validate_descriptor_sets(&self, layout: &PipelineLayout, sets: &[graphics_hardware_interface::DescriptorSetHandle]) {
		for set in sets {
			assert!(
				(set.0 as usize) < self.descriptor_sets.len(),
				"Invalid Vulkan descriptor set. The most likely cause is that a bound handle came from another context.",
			);
		}

		for resource in &layout.resources {
			let descriptor = resource.descriptor;
			for set in sets {
				assert!(
					self.descriptor_sets[set.0 as usize]
						.descriptors
						.keys()
						.all(|slot| crate::vulkan::resource_accepts_retained_slot_key(descriptor, *slot)),
					"Invalid retained Vulkan descriptor slot. The most likely cause is that an array element was written as an interior flat slot instead of using array_element at the array base.",
				);
			}

			let owners = sets
				.iter()
				.filter(|set| {
					self.descriptor_sets[set.0 as usize]
						.descriptors
						.contains_key(&descriptor.slot())
				})
				.count();
			assert!(
				owners <= 1,
				"Overlapping retained Vulkan descriptor sets. The most likely cause is that two bound sets write the same active flat resource slot.",
			);

			let elements = self.descriptors_at_slot(sets, descriptor.slot());
			if descriptor.count() == 1 {
				assert!(
					elements.is_some_and(|elements| elements.contains_key(&0)),
					"Missing retained Vulkan descriptor at resource slot {}. The most likely cause is that a scalar pipeline resource was not written before rendering.",
					descriptor.slot().index(),
				);
			}
			if let Some(elements) = elements {
				for (&array_element, retained) in elements {
					assert!(
						array_element < descriptor.count(),
						"Vulkan descriptor array element is out of range. The most likely cause is that a retained write exceeded the shader resource count.",
					);
					assert!(
						Self::descriptor_matches_kind(retained.descriptor, descriptor.kind()),
						"Vulkan descriptor kind mismatch. The most likely cause is that a retained write does not match the active shader resource interface.",
					);
				}
			}
		}
	}

	fn swapchain_key_for_image(
		&self,
		handle: graphics_hardware_interface::BaseImageHandle,
		sequence_index: u8,
	) -> Option<(graphics_hardware_interface::SwapchainHandle, u8)> {
		self.swapchains.iter().enumerate().find_map(|(index, swapchain)| {
			(swapchain.images[0].0 == handle.0 || swapchain.native_images[0].0 == handle.0).then_some((
				graphics_hardware_interface::SwapchainHandle(index as u64),
				swapchain.acquired_image_indices[sequence_index as usize],
			))
		})
	}

	fn materialization_key(
		&self,
		layout_handle: graphics_hardware_interface::PipelineLayoutHandle,
		sets: &[graphics_hardware_interface::DescriptorSetHandle],
		sequence_index: u8,
	) -> MaterializationKey {
		let layout = &self.pipeline_layouts[layout_handle.0 as usize];
		let mut descriptor_sets = sets
			.iter()
			.map(|set| {
				let descriptor_set = &self.descriptor_sets[set.0 as usize];
				(
					*set,
					descriptor_set.version,
					descriptor_set.sequence_versions[sequence_index as usize],
				)
			})
			.collect::<SmallVec<[_; 4]>>();
		descriptor_sets.sort_unstable_by_key(|(set, ..)| set.0);
		let mut resource_epochs = SmallVec::<[_; MAX_FRAMES_IN_FLIGHT]>::new();
		let mut swapchain_images = SmallVec::<[_; 4]>::new();
		for resource in &layout.resources {
			let Some(elements) = self.descriptors_at_slot(sets, resource.descriptor.slot()) else {
				continue;
			};
			for retained in elements.values() {
				let target_sequence = self.frame_index_with_offset(sequence_index as usize, retained.frame_offset) as u8;
				let key = match retained.descriptor {
					crate::descriptors::WriteData::Buffer { .. } => {
						resource_epochs.push((target_sequence, self.descriptor_sequence_epochs[target_sequence as usize]));
						None
					}
					crate::descriptors::WriteData::Swapchain(handle) => {
						resource_epochs.push((target_sequence, self.descriptor_sequence_epochs[target_sequence as usize]));
						Some((
							handle,
							self.swapchains[handle.0 as usize].acquired_image_indices[target_sequence as usize],
						))
					}
					crate::descriptors::WriteData::Image { handle, .. }
					| crate::descriptors::WriteData::CombinedImageSampler {
						image_handle: handle, ..
					} => {
						resource_epochs.push((target_sequence, self.descriptor_sequence_epochs[target_sequence as usize]));
						self.swapchain_key_for_image(handle, target_sequence)
					}
					_ => None,
				};
				if let Some(key) = key {
					swapchain_images.push(key);
				}
			}
		}
		resource_epochs.sort_unstable_by_key(|(sequence, _)| *sequence);
		resource_epochs.dedup();
		swapchain_images.sort_unstable_by_key(|(handle, image)| (handle.0, *image));
		swapchain_images.dedup();

		MaterializationKey {
			layout: layout_handle,
			descriptor_sets,
			sequence_index,
			resource_epochs,
			swapchain_images,
		}
	}

	fn resolve_retained_descriptor(
		&self,
		retained: crate::vulkan::descriptor_set::RetainedDescriptor,
		sequence_index: u8,
	) -> Descriptor {
		let resource_sequence = self.frame_index_with_offset(sequence_index as usize, retained.frame_offset);
		match retained.descriptor {
			crate::descriptors::WriteData::Buffer { handle, size } => Descriptor::Buffer {
				buffer: self.buffers.nth_handle(handle, resource_sequence).expect(
					"Missing deferred Vulkan buffer. The most likely cause is that frame resource tasks were not processed before descriptor materialization.",
				),
				size,
			},
			crate::descriptors::WriteData::Image { handle, layout } => Descriptor::Image {
				image: self.resolve_descriptor_image_handle(
					graphics_hardware_interface::ImageHandle(handle),
					sequence_index as usize,
					retained.frame_offset,
				),
				layout,
			},
			crate::descriptors::WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				layer,
			} => Descriptor::CombinedImageSampler {
				image: self.resolve_descriptor_image_handle(
					graphics_hardware_interface::ImageHandle(image_handle),
					sequence_index as usize,
					retained.frame_offset,
				),
				sampler: sampler_handle,
				layout,
				layer,
			},
			crate::descriptors::WriteData::Sampler(sampler) => Descriptor::Sampler { sampler },
			crate::descriptors::WriteData::AccelerationStructure { handle } => Descriptor::AccelerationStructure {
				handle: TopLevelAccelerationStructureHandle(handle.0),
			},
			crate::descriptors::WriteData::Swapchain(handle) => {
				let swapchain = &self.swapchains[handle.0 as usize];
				let image_index = swapchain.acquired_image_indices[resource_sequence] as usize;
				Descriptor::Image {
					image: swapchain.images[image_index],
					layout: crate::Layouts::General,
				}
			}
			crate::descriptors::WriteData::StaticSamplers
			| crate::descriptors::WriteData::CombinedImageSamplerArray => unreachable!(
				"Legacy Vulkan descriptor write reached materialization. The most likely cause is that write validation was bypassed."
			),
		}
	}

	fn descriptor_image_view_create_info(
		&self,
		image: &Image,
		view_type: crate::TextureViewTypes,
		layer: Option<u32>,
	) -> vk::ImageViewCreateInfo<'static> {
		let (vk_view_type, base_array_layer, layer_count) = match view_type {
			crate::TextureViewTypes::Texture2D => {
				assert!(
					layer.is_none() && image.layers.is_none() && image.extent.depth().max(1) == 1,
					"Vulkan 2D descriptor view mismatch. The most likely cause is that a layered or 3D image was written to a Texture2D shader resource."
				);
				(vk::ImageViewType::TYPE_2D, 0, 1)
			}
			crate::TextureViewTypes::Texture2DArray => {
				let layers = image.layers.map(|layers| layers.get()).expect(
					"Vulkan array descriptor view mismatch. The most likely cause is that a non-array image was written to a Texture2DArray shader resource.",
				);
				let base = layer.unwrap_or(0);
				assert!(
					base < layers,
					"Vulkan image layer is out of range. The most likely cause is an invalid combined-image-sampler layer."
				);
				(
					vk::ImageViewType::TYPE_2D_ARRAY,
					base,
					if layer.is_some() { 1 } else { layers },
				)
			}
			crate::TextureViewTypes::Texture3D => {
				assert!(
					layer.is_none() && image.layers.is_none() && image.extent.depth() > 1,
					"Vulkan 3D descriptor view mismatch. The most likely cause is that a 2D image was written to a Texture3D shader resource."
				);
				(vk::ImageViewType::TYPE_3D, 0, 1)
			}
		};
		vk::ImageViewCreateInfo::default()
			.image(image.image)
			.view_type(vk_view_type)
			.format(image.format)
			.components(vk::ComponentMapping::default())
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: if image.format_.is_depth() {
					vk::ImageAspectFlags::DEPTH
				} else {
					vk::ImageAspectFlags::COLOR
				},
				base_mip_level: 0,
				level_count: 1,
				base_array_layer,
				layer_count,
			})
	}

	/// Writes one immutable logical-set union into previously unused descriptor-heap memory and caches it.
	pub(crate) fn materialize_descriptor_sets(
		&mut self,
		layout_handle: graphics_hardware_interface::PipelineLayoutHandle,
		sets: &[graphics_hardware_interface::DescriptorSetHandle],
		sequence_index: u8,
	) -> DescriptorMaterializationHandle {
		let key = self.materialization_key(layout_handle, sets, sequence_index);
		if let Some(handle) = self.materialization_indices.get(&key) {
			return *handle;
		}

		let layout = self.pipeline_layouts[layout_handle.0 as usize].clone();
		self.validate_descriptor_sets(&layout, sets);
		let mut resolved = Vec::<(PipelineResourceDescriptor, u32, Descriptor)>::new();
		for resource in &layout.resources {
			let Some(elements) = self.descriptors_at_slot(sets, resource.descriptor.slot()) else {
				continue;
			};
			let mut elements = elements.iter().collect::<SmallVec<[_; 16]>>();
			elements.sort_unstable_by_key(|(array_element, _)| **array_element);
			for (&array_element, &retained) in elements {
				resolved.push((
					*resource,
					array_element,
					self.resolve_retained_descriptor(retained, sequence_index),
				));
			}
		}

		let properties = self.device.descriptor_heap_properties;
		let resource_alignment = properties
			.buffer_descriptor_alignment
			.max(properties.image_descriptor_alignment);
		let (resource_heap_offset, sampler_heap_offset) = {
			let heaps = self.descriptor_heaps.as_mut().unwrap();
			let resource_offset = (layout.resource_heap_size > 0)
				.then(|| heaps.resource_mut().allocate(layout.resource_heap_size, resource_alignment))
				.unwrap_or(0);
			let sampler_offset = (layout.sampler_heap_size > 0)
				.then(|| {
					heaps
						.sampler_mut()
						.allocate(layout.sampler_heap_size, properties.sampler_descriptor_alignment)
				})
				.unwrap_or(0);
			(resource_offset, sampler_offset)
		};

		let mut address_writes = Vec::<(vk::DescriptorType, vk::DeviceAddressRangeEXT, u32, u64)>::new();
		let mut image_writes = Vec::<(vk::DescriptorType, vk::ImageViewCreateInfo, vk::ImageLayout, u32, u64)>::new();
		let mut sampler_writes = Vec::<(graphics_hardware_interface::SamplerHandle, u32, u64)>::new();
		let mut materialized_resources = SmallVec::<[ResolvedPipelineDescriptor; 128]>::new();
		for (resource, array_element, descriptor) in resolved {
			let resource_offset = resource
				.resource_heap_offset
				.map(|offset| resource_heap_offset + offset + array_element * resource.resource_stride);
			let sampler_offset = resource
				.sampler_heap_offset
				.map(|offset| sampler_heap_offset + offset + array_element * resource.sampler_stride);
			match descriptor {
				Descriptor::Buffer { buffer, size } => {
					let buffer = self.buffers.resource(buffer);
					let size = match size {
						graphics_hardware_interface::Ranges::Whole => buffer.size as u64,
						graphics_hardware_interface::Ranges::Size(size) => size as u64,
					};
					assert!(
						size > 0 && size <= buffer.size as u64,
						"Invalid Vulkan buffer descriptor range. The most likely cause is that a descriptor exceeds its backing buffer."
					);
					address_writes.push((
						crate::vulkan::descriptor_type(resource.descriptor.kind()).unwrap(),
						vk::DeviceAddressRangeEXT::default().address(buffer.device_address).size(size),
						resource_offset.unwrap(),
						resource.resource_stride as u64,
					));
				}
				Descriptor::Image {
					image,
					layout: image_layout,
				} => {
					let image = &self.images[image.0 as usize];
					let vk_layout = texture_format_and_resource_use_to_image_layout(image.format_, image_layout, None);
					image_writes.push((
						crate::vulkan::descriptor_type(resource.descriptor.kind()).unwrap(),
						self.descriptor_image_view_create_info(image, resource.descriptor.texture_view(), None),
						vk_layout,
						resource_offset.unwrap(),
						resource.resource_stride as u64,
					));
				}
				Descriptor::CombinedImageSampler {
					image,
					sampler,
					layout: image_layout,
					layer,
				} => {
					let image = &self.images[image.0 as usize];
					let vk_layout = texture_format_and_resource_use_to_image_layout(image.format_, image_layout, None);
					image_writes.push((
						vk::DescriptorType::SAMPLED_IMAGE,
						self.descriptor_image_view_create_info(image, resource.descriptor.texture_view(), layer),
						vk_layout,
						resource_offset.unwrap(),
						resource.resource_stride as u64,
					));
					sampler_writes.push((sampler, sampler_offset.unwrap(), resource.sampler_stride as u64));
				}
				Descriptor::Sampler { sampler } => {
					sampler_writes.push((sampler, sampler_offset.unwrap(), resource.sampler_stride as u64));
				}
				Descriptor::AccelerationStructure { handle } => {
					let acceleration_structure = &self.acceleration_structures[handle.0 as usize];
					let address = unsafe {
						self.acceleration_structure.get_acceleration_structure_device_address(
							&vk::AccelerationStructureDeviceAddressInfoKHR::default()
								.acceleration_structure(acceleration_structure.acceleration_structure),
						)
					};
					address_writes.push((
						vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
						vk::DeviceAddressRangeEXT::default().address(address).size(0),
						resource_offset.unwrap(),
						resource.resource_stride as u64,
					));
				}
			}
			if !matches!(descriptor, Descriptor::Sampler { .. }) {
				if let Some(existing) = materialized_resources
					.iter_mut()
					.find(|existing| Self::descriptors_consume_same_resource(existing.descriptor, descriptor))
				{
					existing.stages |= resource.stages;
					existing.access |= resource.descriptor.access();
				} else {
					materialized_resources.push(ResolvedPipelineDescriptor {
						descriptor,
						stages: resource.stages,
						access: resource.descriptor.access(),
					});
				}
			}
		}

		let heaps = self.descriptor_heaps.as_ref().unwrap();
		if !address_writes.is_empty() {
			let ranges = address_writes.iter().map(|(_, range, ..)| *range).collect::<Box<[_]>>();
			let infos = address_writes
				.iter()
				.enumerate()
				.map(|(index, (ty, ..))| {
					vk::ResourceDescriptorInfoEXT::default()
						.ty(*ty)
						.data(vk::ResourceDescriptorDataEXT {
							p_address_range: &ranges[index],
						})
				})
				.collect::<Box<[_]>>();
			let destinations = address_writes
				.iter()
				.map(|(_, _, offset, size)| heaps.resource().host_range(*offset, *size))
				.collect::<Box<[_]>>();
			unsafe {
				self.device
					.descriptor_heap
					.write_resource_descriptors(&infos, &destinations)
					.expect("Vulkan buffer descriptor write failed. The most likely cause is an invalid device address range.");
			}
		}
		if !image_writes.is_empty() {
			let views = image_writes.iter().map(|(_, view, ..)| *view).collect::<Box<[_]>>();
			let images = image_writes
				.iter()
				.enumerate()
				.map(|(index, (_, _, layout, ..))| vk::ImageDescriptorInfoEXT::default().view(&views[index]).layout(*layout))
				.collect::<Box<[_]>>();
			let infos = image_writes
				.iter()
				.enumerate()
				.map(|(index, (ty, ..))| {
					vk::ResourceDescriptorInfoEXT::default()
						.ty(*ty)
						.data(vk::ResourceDescriptorDataEXT { p_image: &images[index] })
				})
				.collect::<Box<[_]>>();
			let destinations = image_writes
				.iter()
				.map(|(_, _, _, offset, size)| heaps.resource().host_range(*offset, *size))
				.collect::<Box<[_]>>();
			unsafe {
				self.device
					.descriptor_heap
					.write_resource_descriptors(&infos, &destinations)
					.expect(
						"Vulkan image descriptor write failed. The most likely cause is an invalid image view description or layout.",
					);
			}
		}
		if !sampler_writes.is_empty() {
			let reductions = sampler_writes
				.iter()
				.map(|(handle, ..)| {
					vk::SamplerReductionModeCreateInfo::default()
						.reduction_mode(self.samplers[handle.0 as usize].reduction_mode)
				})
				.collect::<Box<[_]>>();
			let mut samplers = sampler_writes
				.iter()
				.map(|(handle, ..)| self.samplers[handle.0 as usize].create_info())
				.collect::<Box<[_]>>();
			for (sampler, reduction) in samplers.iter_mut().zip(reductions.iter()) {
				sampler.p_next = (reduction as *const vk::SamplerReductionModeCreateInfo).cast();
			}
			let destinations = sampler_writes
				.iter()
				.map(|(_, offset, size)| heaps.sampler().host_range(*offset, *size))
				.collect::<Box<[_]>>();
			unsafe {
				self.device
					.descriptor_heap
					.write_sampler_descriptors(&samplers, &destinations)
					.expect(
						"Vulkan sampler descriptor write failed. The most likely cause is an unsupported sampler description.",
					);
			}
		}

		let materialization = DescriptorMaterialization {
			resource_heap_offset,
			resource_heap_size: layout.resource_heap_size,
			sampler_heap_offset,
			sampler_heap_size: layout.sampler_heap_size,
			resources: materialized_resources,
		};
		let handle = self
			.free_materialization_handles
			.pop()
			.unwrap_or_else(|| DescriptorMaterializationHandle(self.descriptor_materializations.len() as u64));
		if handle.0 as usize == self.descriptor_materializations.len() {
			self.descriptor_materializations.push(Some(materialization));
		} else {
			self.descriptor_materializations[handle.0 as usize] = Some(materialization);
		}
		self.materialization_indices.insert(key, handle);
		handle
	}

	pub(crate) fn descriptor_materialization(&self, handle: DescriptorMaterializationHandle) -> &DescriptorMaterialization {
		self.descriptor_materializations[handle.0 as usize].as_ref().expect(
			"Retired Vulkan descriptor materialization was reused. The most likely cause is that a command buffer outlived its frame sequence fence.",
		)
	}
	#[inline]
	fn set_object_debug_name(&mut self, name: Option<&str>, handle: graphics_hardware_interface::Handles) {
		#[cfg(debug_assertions)]
		if let Some(name) = name {
			self.names.insert(handle, name.to_string());
		}
	}

	#[inline]
	fn get_object_debug_name(&self, handle: graphics_hardware_interface::Handles) -> Option<String> {
		#[cfg(debug_assertions)]
		let name = self.names.get(&handle).map(|e| e.clone());

		#[cfg(not(debug_assertions))]
		let name: Option<String> = None;

		name
	}
}

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

impl crate::context::Context for Context {
	type Queue = crate::vulkan::queue::Queue;
	type QueueReference<'a>
		= crate::vulkan::queue::QueueReference<'a>
	where
		Self: 'a;
	type CommandBuffer<'a>
		= crate::vulkan::command_buffer::CommandBufferReference<'a>
	where
		Self: 'a;

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		self.device.has_errors()
	}

	fn supports_bc_texture_compression(&self) -> bool {
		true
	}

	fn queue(&mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> Self::Queue {
		let queue = &self.queues[queue_handle.0 as usize];
		let vk_queue = queue.vk_queue.clone();
		let queue_family_index = queue.queue_family_index;
		let queue_index = queue._queue_index;
		crate::vulkan::queue::Queue {
			device: std::ptr::NonNull::from(self),
			queue_handle,
			vk_queue,
			queue_family_index,
			_queue_index: queue_index,
		}
	}

	fn queue_reference<'a>(&'a mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> Self::QueueReference<'a> {
		crate::vulkan::queue::QueueReference {
			device: self,
			queue_handle,
		}
	}

	fn command_buffer<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> Self::CommandBuffer<'a> {
		crate::vulkan::command_buffer::CommandBufferReference {
			device: self,
			command_buffer_handle,
		}
	}

	fn set_frames_in_flight(&mut self, frames: u8) {
		if self.frames == frames {
			return;
		}

		if frames > MAX_FRAMES_IN_FLIGHT as u8 {
			panic!("Cannot set frames in flight to more than {}", MAX_FRAMES_IN_FLIGHT);
		}

		let current_frames = self.frames;
		let target_frames = frames;
		let delta_frames = target_frames as i8 - current_frames as i8;

		if delta_frames > 0 {
			let to_extend = self
				.images
				.iter()
				.filter_map(|image| {
					let next = image.next?;

					let mut handle = next;

					while let Some(h) = self.images[handle.0 as usize].next {
						handle = h;
					}

					handle.into()
				})
				.collect::<Vec<_>>();

			for image_handle in to_extend {
				let current_image = &self.images[image_handle.0 as usize];

				#[cfg(debug_assertions)]
				let name: Option<&str> = None;

				#[cfg(not(debug_assertions))]
				let name = None;

				let next = current_image.next;
				let format = current_image.format_;
				let access = current_image.access;
				let array_layers = current_image.layers;
				let extent = current_image.extent;
				let resource_uses = current_image.uses;
				let mip_levels = current_image.mip_levels;

				let new_image = self.create_image_internal(
					next,
					None,
					name,
					format,
					access,
					array_layers,
					extent,
					resource_uses,
					mip_levels,
				);

				let current_image = &mut self.images[image_handle.0 as usize];
				current_image.next = Some(new_image);
			}

			let to_extend = self
				.synchronizers
				.iter()
				.filter_map(|synchronizer| {
					let next = synchronizer.next?;

					let mut handle = next;

					while let Some(h) = self.synchronizers[handle.0 as usize].next {
						handle = h;
					}

					handle.into()
				})
				.collect::<Vec<_>>();

			for synchronizer_handle in to_extend {
				let current_synchronizer = &self.synchronizers[synchronizer_handle.0 as usize];

				#[cfg(debug_assertions)]
				let name_owned = self
					.names
					.get(
						&graphics_hardware_interface::SynchronizerHandle(synchronizer_handle.root(&self.synchronizers).0)
							.into(),
					)
					.cloned();

				#[cfg(not(debug_assertions))]
				let name_owned: Option<String> = None;

				let name = name_owned.as_deref();
				let signaled = current_synchronizer.signaled;

				let new_synchronizer = self.create_synchronizer_internal(name, signaled);

				let current_synchronizer = &mut self.synchronizers[synchronizer_handle.0 as usize];
				current_synchronizer.next = Some(new_synchronizer);
			}

			for command_buffer in &mut self.command_buffers {
				let queue = &self.queues[command_buffer.queue_handle.0 as usize];
				let vk_queue = queue.vk_queue.clone();
				let command_pool_create_info =
					vk::CommandPoolCreateInfo::default().queue_family_index(queue.queue_family_index);

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

				let vk_command_buffer = command_buffers[0];

				// self.set_name(vk_command_buffer, name);

				command_buffer.frames.push(CommandBufferInternal {
					vk_queue: vk_queue.clone(),
					command_pool,
					command_buffer: vk_command_buffer,
				});
			}
		} else {
			unimplemented!()
		}

		self.frames = target_frames;
	}

	fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.get_buffer_address(buffer_handle)
	}

	fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		self.get_buffer_slice(buffer_handle)
	}

	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		self.get_mut_buffer_slice(buffer_handle)
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		self.sync_buffer(buffer_handle);
	}

	fn get_texture_slice_mut(&self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
		self.get_texture_slice_mut(texture_handle)
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle) {
		self.sync_texture(image_handle);
	}

	fn write_texture(&mut self, texture_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		self.write_texture(texture_handle, f);
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		Context::write(self, descriptor_set_writes);
	}

	fn write_instance(
		&mut self,
		instances_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		self.write_instance(
			instances_buffer_handle,
			instance_index,
			transform,
			custom_index,
			mask,
			sbt_record_offset,
			acceleration_structure,
		);
	}

	fn write_sbt_entry(
		&mut self,
		sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		sbt_record_offset: usize,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
		shader_handle: graphics_hardware_interface::ShaderHandle,
	) {
		self.write_sbt_entry(sbt_buffer_handle, sbt_record_offset, pipeline_handle, shader_handle);
	}

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: graphics_hardware_interface::PresentationModes,
		fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		self.bind_to_window(window_os_handles, presentation_mode, fallback_extent, uses)
	}

	fn get_image_data<'a>(&'a mut self, texture_copy_handle: graphics_hardware_interface::TextureCopyHandle) -> &'a [u8] {
		Context::get_image_data(self, texture_copy_handle)
	}

	fn resize_buffer<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>, size: usize) {
		self.resize_buffer(buffer_handle, size);
	}

	fn start_frame_capture(&mut self) {
		self.device.start_frame_capture();
	}

	fn end_frame_capture(&mut self) {
		self.device.end_frame_capture();
	}

	fn wait(&self) {
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
		let vertex_buffer_size = vertices.len();
		let index_buffer_size = indices.len();

		let buffer_size = vertex_buffer_size.next_multiple_of(16) + index_buffer_size;

		let buffer_creation_result = self.create_vulkan_buffer(
			None,
			buffer_size,
			vk::BufferUsageFlags::VERTEX_BUFFER
				| vk::BufferUsageFlags::INDEX_BUFFER
				| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, pointer) = self.create_allocation_internal(
			buffer_creation_result.size,
			buffer_creation_result.memory_flags.into(),
			crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		);

		self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		unsafe {
			let vertex_buffer_pointer = pointer.expect("No pointer");
			std::ptr::copy_nonoverlapping(vertices.as_ptr(), vertex_buffer_pointer, vertex_buffer_size);
			let index_buffer_pointer = vertex_buffer_pointer.add(vertex_buffer_size.next_multiple_of(16));
			std::ptr::copy_nonoverlapping(indices.as_ptr(), index_buffer_pointer, index_buffer_size);
		}

		let mesh_handle = graphics_hardware_interface::MeshHandle(self.meshes.len() as u64);

		self.meshes.push(Mesh {
			buffer: buffer_creation_result.resource,
			vertex_count,
			index_count,
			vertex_size: vertex_layout.size(),
		});

		mesh_handle
	}

	/// Creates a shader.
	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let shader = match shader_source_type {
			crate::shader::Sources::SPIRV(spirv) => {
				if !spirv.as_ptr().is_aligned_to(align_of::<u32>()) {
					return Err(());
				}

				// SAFETY: shader was checked to be aligned to 4 bytes.
				Cow::Borrowed(unsafe { std::slice::from_raw_parts(spirv.as_ptr() as *const u32, spirv.len() / 4) })
			}
			crate::shader::Sources::DXIL(_)
			| crate::shader::Sources::HLSL { .. }
			| crate::shader::Sources::MTL { .. }
			| crate::shader::Sources::MTLB { .. } => return Err(()),
		};

		let shader_module_create_info = vk::ShaderModuleCreateInfo::default().code(&shader);

		let shader_module = unsafe { self.device.create_shader_module(&shader_module_create_info, None).unwrap() };

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
			descriptors: HashMap::new(),
		});
		self.set_object_debug_name(name, graphics_hardware_interface::Handles::DescriptorSet(handle));
		handle
	}

	fn create_raster_pipeline(
		&mut self,
		builder: crate::pipelines::raster::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		self.create_vulkan_pipeline(builder)
	}

	fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let shader_parameter = builder.shader;
		let pipeline_layout_handle =
			self.get_or_create_pipeline_layout(std::slice::from_ref(&shader_parameter), builder.push_constant_ranges);
		let mut specialization_entries_buffer = Vec::<u8>::with_capacity(256);

		let mut specialization_map_entries = Vec::with_capacity(48);

		for specialization_map_entry in shader_parameter.specialization_map {
			// TODO: accumulate offset
			match specialization_map_entry.get_type().as_str() {
				"bool" | "u32" | "f32" => {
					specialization_map_entries.push(
						vk::SpecializationMapEntry::default()
							.constant_id(specialization_map_entry.get_constant_id())
							.offset(specialization_entries_buffer.len() as u32)
							.size(4),
					);

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				"vec2f" => {
					for i in 0..2 {
						specialization_map_entries.push(
							vk::SpecializationMapEntry::default()
								.constant_id(specialization_map_entry.get_constant_id() + i)
								.offset(specialization_entries_buffer.len() as u32 + i * 4)
								.size(4),
						);
					}

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				"vec3f" => {
					for i in 0..3 {
						specialization_map_entries.push(
							vk::SpecializationMapEntry::default()
								.constant_id(specialization_map_entry.get_constant_id() + i)
								.offset(specialization_entries_buffer.len() as u32 + i * 4)
								.size(4),
						);
					}

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				"vec4f" => {
					for i in 0..4 {
						specialization_map_entries.push(
							vk::SpecializationMapEntry::default()
								.constant_id(specialization_map_entry.get_constant_id() + i)
								.offset(specialization_entries_buffer.len() as u32 + i * 4)
								.size(4),
						);
					}

					assert_eq!(specialization_map_entry.get_size(), 16);

					specialization_entries_buffer.extend_from_slice(specialization_map_entry.get_data());
				}
				_ => {
					panic!("Unknown specialization map entry type");
				}
			}
		}

		let specialization_info = vk::SpecializationInfo::default()
			.data(&specialization_entries_buffer)
			.map_entries(&specialization_map_entries);

		let pipeline_layout = &self.pipeline_layouts[pipeline_layout_handle.0 as usize];
		let shader = &self.shaders[shader_parameter.handle.0 as usize];
		let mappings = crate::vulkan::build_shader_mappings(pipeline_layout, &shader.shader_resource_descriptors);
		let mut mapping_info = vk::ShaderDescriptorSetAndBindingMappingInfoEXT::default().mappings(&mappings);
		let stage = vk::PipelineShaderStageCreateInfo::default()
			.push(&mut mapping_info)
			.stage(vk::ShaderStageFlags::COMPUTE)
			.module(shader.shader)
			.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
			.specialization_info(&specialization_info);
		let mut descriptor_heap_flags =
			vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
		let create_infos = [vk::ComputePipelineCreateInfo::default()
			.push(&mut descriptor_heap_flags)
			.stage(stage)
			.layout(vk::PipelineLayout::null())];

		let pipeline_handle = unsafe {
			self.device
				.create_compute_pipelines(vk::PipelineCache::null(), &create_infos, None)
				.expect("No compute pipeline")[0]
		};

		let handle = graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64);

		self.pipelines.push(Pipeline {
			pipeline: pipeline_handle,
			layout: pipeline_layout_handle,
			shader_handles: HashMap::new(),
		});

		handle
	}

	fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let pipeline_layout_handle =
			self.get_or_create_pipeline_layout(builder.shaders.as_ref(), builder.push_constant_ranges.as_ref());
		let shaders = builder.shaders;
		let mut groups = Vec::with_capacity(1024);

		let pipeline_layout = &self.pipeline_layouts[pipeline_layout_handle.0 as usize];
		let stage_mappings = shaders
			.iter()
			.map(|stage| {
				let shader = &self.shaders[stage.handle.0 as usize];
				crate::vulkan::build_shader_mappings(pipeline_layout, &shader.shader_resource_descriptors)
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
				let shader = &self.shaders[stage.handle.0 as usize];

				vk::PipelineShaderStageCreateInfo::default()
					.push(mapping_info)
					.stage(to_shader_stage_flags(stage.stage))
					.module(shader.shader)
					.name(std::ffi::CStr::from_bytes_with_nul(b"main\0").unwrap())
			})
			.collect::<Vec<_>>();

		for (i, shader) in shaders.iter().enumerate() {
			match shader.stage {
				crate::ShaderTypes::RayGen | crate::ShaderTypes::Miss | crate::ShaderTypes::Callable => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
							.general_shader(i as u32)
							.closest_hit_shader(vk::SHADER_UNUSED_KHR)
							.any_hit_shader(vk::SHADER_UNUSED_KHR)
							.intersection_shader(vk::SHADER_UNUSED_KHR),
					);
				}
				crate::ShaderTypes::ClosestHit => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
							.general_shader(vk::SHADER_UNUSED_KHR)
							.closest_hit_shader(i as u32)
							.any_hit_shader(vk::SHADER_UNUSED_KHR)
							.intersection_shader(vk::SHADER_UNUSED_KHR),
					);
				}
				crate::ShaderTypes::AnyHit => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
							.general_shader(vk::SHADER_UNUSED_KHR)
							.closest_hit_shader(vk::SHADER_UNUSED_KHR)
							.any_hit_shader(i as u32)
							.intersection_shader(vk::SHADER_UNUSED_KHR),
					);
				}
				crate::ShaderTypes::Intersection => {
					groups.push(
						vk::RayTracingShaderGroupCreateInfoKHR::default()
							.ty(vk::RayTracingShaderGroupTypeKHR::PROCEDURAL_HIT_GROUP)
							.general_shader(vk::SHADER_UNUSED_KHR)
							.closest_hit_shader(vk::SHADER_UNUSED_KHR)
							.any_hit_shader(vk::SHADER_UNUSED_KHR)
							.intersection_shader(i as u32),
					);
				}
				_ => {
					// warn!("Fed shader of type '{:?}' to ray tracing pipeline", shader.stage)
				}
			}
		}

		let mut descriptor_heap_flags =
			vk::PipelineCreateFlags2CreateInfo::default().flags(vk::PipelineCreateFlags2::DESCRIPTOR_HEAP_EXT);
		let create_info = vk::RayTracingPipelineCreateInfoKHR::default()
			.push(&mut descriptor_heap_flags)
			.layout(vk::PipelineLayout::null())
			.stages(&stages)
			.groups(&groups)
			.max_pipeline_ray_recursion_depth(1);

		let mut handles: HashMap<graphics_hardware_interface::ShaderHandle, [u8; 32]> = HashMap::with_capacity(shaders.len());

		let pipeline_handle = unsafe {
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

			for (i, shader) in shaders.iter().enumerate() {
				let mut h = [0u8; 32];
				h.copy_from_slice(&handle_buffer[i * 32..(i + 1) * 32]);

				handles.insert(*shader.handle, h);
			}

			pipeline
		};

		let handle = graphics_hardware_interface::PipelineHandle(self.pipelines.len() as u64);

		self.pipelines.push(Pipeline {
			pipeline: pipeline_handle,
			layout: pipeline_layout_handle,
			shader_handles: handles,
		});

		handle
	}

	fn build_image(&mut self, builder: image::Builder) -> graphics_hardware_interface::ImageHandle {
		let root_image_handle = self.create_image_internal(
			None,
			None,
			builder.name,
			builder.format,
			builder.device_accesses,
			builder.array_layers,
			builder.extent,
			builder.resource_uses,
			builder.mip_levels,
		);

		let handle =
			graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(root_image_handle.0));

		let instances = match builder.use_case {
			crate::UseCases::DYNAMIC => self.frames,
			crate::UseCases::STATIC => 1,
		};

		let mut previous = root_image_handle;
		for _ in 1..instances {
			previous = self.create_image_internal(
				None,
				Some(previous),
				builder.name,
				builder.format,
				builder.device_accesses,
				builder.array_layers,
				builder.extent,
				builder.resource_uses,
				builder.mip_levels,
			);
		}

		self.set_object_debug_name(builder.name, handle.into());

		handle
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
		let size = max_instance_count as usize * std::mem::size_of::<vk::AccelerationStructureInstanceKHR>();

		let buffer_creation_result = self.create_vulkan_buffer(
			name,
			size,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
				| vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, _) = self.create_allocation_internal(
			buffer_creation_result.size,
			buffer_creation_result.memory_flags.into(),
			crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		);

		let (address, pointer) = self.bind_vulkan_buffer_memory(&buffer_creation_result, allocation_handle, 0);

		let (buffer_handle, _) = self.buffers.add(Buffer {
			staging: None,
			source: None,
			buffer: buffer_creation_result.resource,
			size: buffer_creation_result.size,
			device_address: address,
			pointer,
			uses: crate::Uses::empty(),
			access: crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead,
		});

		buffer_handle
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		let geometry = vk::AccelerationStructureGeometryKHR::default()
			.geometry_type(vk::GeometryTypeKHR::INSTANCES)
			.geometry(vk::AccelerationStructureGeometryDataKHR {
				instances: vk::AccelerationStructureGeometryInstancesDataKHR::default(),
			});

		let geometries = [geometry];

		let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.geometries(&geometries);

		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();

		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(
				vk::AccelerationStructureBuildTypeKHR::DEVICE,
				&build_info,
				Some(&[max_instance_count]),
				&mut size_info,
			);
		}

		let acceleration_structure_size = size_info.acceleration_structure_size as usize;
		let _ = size_info.build_scratch_size as usize;

		let buffer = self.create_vulkan_buffer(
			None,
			acceleration_structure_size,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, _) =
			self.create_allocation_internal(buffer.size, buffer.memory_flags.into(), crate::DeviceAccesses::GpuWrite);

		let (..) = self.bind_vulkan_buffer_memory(&buffer, allocation_handle, 0);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer.resource)
			.size(acceleration_structure_size as u64)
			.offset(0)
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL);

		let handle =
			graphics_hardware_interface::TopLevelAccelerationStructureHandle(self.acceleration_structures.len() as u64);

		{
			let handle = unsafe {
				self.acceleration_structure
					.create_acceleration_structure(&create_info, None)
					.expect("No acceleration structure")
			};

			self.acceleration_structures.push(AccelerationStructure {
				acceleration_structure: handle,
				buffer: buffer.resource,
			});

			self.set_name(handle, name);
		}

		handle
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

		let mut size_info = vk::AccelerationStructureBuildSizesInfoKHR::default();

		unsafe {
			self.acceleration_structure.get_acceleration_structure_build_sizes(
				vk::AccelerationStructureBuildTypeKHR::DEVICE,
				&build_info,
				Some(&[primitive_count]),
				&mut size_info,
			);
		}

		let acceleration_structure_size = size_info.acceleration_structure_size as usize;
		let _ = size_info.build_scratch_size as usize;

		let buffer_descriptor = self.create_vulkan_buffer(
			None,
			acceleration_structure_size,
			vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);

		let (allocation_handle, _) = self.create_allocation_internal(
			buffer_descriptor.size,
			buffer_descriptor.memory_flags.into(),
			crate::DeviceAccesses::GpuWrite,
		);

		let (..) = self.bind_vulkan_buffer_memory(&buffer_descriptor, allocation_handle, 0);

		let create_info = vk::AccelerationStructureCreateInfoKHR::default()
			.buffer(buffer_descriptor.resource)
			.size(acceleration_structure_size as u64)
			.offset(0)
			.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);

		let handle =
			graphics_hardware_interface::BottomLevelAccelerationStructureHandle(self.acceleration_structures.len() as u64);

		{
			let handle = unsafe {
				self.acceleration_structure
					.create_acceleration_structure(&create_info, None)
					.expect("No acceleration structure")
			};

			self.acceleration_structures.push(AccelerationStructure {
				acceleration_structure: handle,
				buffer: buffer_descriptor.resource,
			});
		}

		handle
	}

	fn build_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> graphics_hardware_interface::BufferHandle<T> {
		let size = std::mem::size_of::<T>();

		let buffer_handle =
			self.create_buffer_internal(None, None, builder.name, builder.resource_uses, size, builder.device_accesses);
		let handle = graphics_hardware_interface::BufferHandle::<T>(
			graphics_hardware_interface::BaseBufferHandle::new(buffer_handle.0),
			std::marker::PhantomData::<T> {},
		);

		return handle;
	}

	fn build_dynamic_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> crate::DynamicBufferHandle<T> {
		let size = std::mem::size_of::<T>();

		let buffer_handle =
			self.create_buffer_internal(None, None, builder.name, builder.resource_uses, size, builder.device_accesses);
		let handle = graphics_hardware_interface::DynamicBufferHandle::<T>(
			graphics_hardware_interface::BaseBufferHandle::new(buffer_handle.0),
			std::marker::PhantomData::<T> {},
		);

		if super::buffer::PERSISTENT_WRITE
			&& builder.device_accesses.intersects(crate::DeviceAccesses::CpuWrite)
			&& !Self::uses_only_host_access(builder.device_accesses)
		{
			// The master buffer's existing staging buffer becomes the shared, persistent
			// CPU-writable source buffer. We create a new per-frame staging buffer for
			// frame 0 and store the source handle on the master buffer.

			let source_handle = self
				.buffers
				.resource(buffer_handle)
				.staging
				.expect("CpuWrite dynamic buffer must have a staging buffer");

			// Create a new per-frame staging buffer for frame 0
			let frame0_staging = self.create_staging_buffer(builder.name, size);

			// Reassign: the master's staging now points to the new per-frame staging,
			// and source points to the original (persistent) CPU-writable buffer.
			let buffer = self.buffers.resource_mut(buffer_handle);
			buffer.staging = Some(frame0_staging);
			buffer.source = Some(source_handle);

			// Track this dynamic buffer for automatic per-frame memcpy
			self.persistent_write_dynamic_buffers.push(handle.into());

			for i in 1..self.frames {
				assert!(i < 2, "This does not support more than one deferred buffer!");
				self.tasks.push(Task::new(
					Tasks::BuildBuffer(BuildBuffer {
						previous: buffer_handle,
						master: handle.into(),
						source: Some(source_handle),
					}),
					Some(i),
				));
			}
		} else {
			for i in 1..self.frames {
				assert!(i < 2, "This does not support more than one deferred buffer!");
				self.tasks.push(Task::new(
					Tasks::BuildBuffer(BuildBuffer {
						previous: buffer_handle,
						master: handle.into(),
						source: None,
					}),
					Some(i),
				));
			}
		}

		handle
	}

	fn build_dynamic_image(&mut self, builder: crate::image::Builder) -> crate::DynamicImageHandle {
		let handle = self.build_image(builder.use_case(crate::UseCases::DYNAMIC));

		crate::DynamicImageHandle(handle.0)
	}

	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> graphics_hardware_interface::SynchronizerHandle {
		let synchronizer_handle = graphics_hardware_interface::SynchronizerHandle(self.synchronizers.len() as u64);

		{
			let mut previous: Option<SynchronizerHandle> = None;

			for _ in 0..self.frames {
				let synchronizer_handle = self.create_synchronizer_internal(name, signaled);

				if let Some(pr) = previous {
					self.synchronizers[pr.0 as usize].next = Some(synchronizer_handle);
				}

				previous = Some(synchronizer_handle);
			}
		}

		self.set_object_debug_name(name, synchronizer_handle.into());

		synchronizer_handle
	}
}

impl Drop for Context {
	fn drop(&mut self) {
		unsafe {
			self.device.device_wait_idle().expect(
				"Failed to wait for the Vulkan device during context destruction. The most likely cause is that the device was lost.",
			);
			self.command_buffers.iter().for_each(|command_buffer| {
				command_buffer.frames.iter().for_each(|command_buffer| {
					self.device.destroy_command_pool(command_buffer.command_pool, None);
				});
			});

			self.synchronizers.iter().for_each(|synchronizer| {
				self.device.destroy_semaphore(synchronizer.semaphore, None);
				self.device.destroy_fence(synchronizer.fence, None);
			});

			self.pipelines.iter().for_each(|pipeline| {
				self.device.destroy_pipeline(pipeline.pipeline, None);
			});

			self.meshes.iter().for_each(|mesh| {
				self.device.destroy_buffer(mesh.buffer, None);
			});

			self.buffers.iter().for_each(|buffer| {
				self.device.destroy_buffer(buffer.buffer, None);
			});
			if let Some(heaps) = &self.descriptor_heaps {
				self.device.destroy_buffer(heaps.resource().buffer, None);
				self.device.destroy_buffer(heaps.sampler().buffer, None);
			}

			self.images.iter().for_each(|image| {
				if let Some(staging_buffer) = image.staging_buffer {
					self.device.destroy_buffer(staging_buffer, None);
				}

				if !image.full_image_view.is_null() {
					self.device.destroy_image_view(image.full_image_view, None);
				}

				for vk_image_view in image.image_views {
					self.device.destroy_image_view(vk_image_view, None);
				}
			});

			self.swapchains.iter().for_each(|swapchain| {
				self.swapchain.destroy_swapchain(swapchain.swapchain, None);
				self.surface.destroy_surface(swapchain.surface, None);
			});

			self.images.iter().for_each(|image| {
				if image.owns_image {
					self.device.destroy_image(image.image, None);
				}
			});

			self.shaders.iter().for_each(|shader| {
				self.device.destroy_shader_module(shader.shader, None);
			});

			self.allocations.iter().for_each(|allocation| {
				self.device.free_memory(allocation.memory, None);
			});
		}
	}
}

use std::{borrow::Cow, num::NonZeroU32, u64};

use ash::vk::{self, Handle as _, TaggedStructure as _};
use smallvec::SmallVec;
use utils::hash::{HashSet, HashSetExt};
use utils::{
	Extent,
	hash::{HashMap, HashMapExt},
};

use super::{
	AccelerationStructure, Allocation, Buffer, BufferHandle, CommandBuffer, CommandBufferInternal, DescriptorHeapArena,
	DescriptorHeaps, DescriptorMaterialization, DescriptorMaterializationHandle, DescriptorSet, Image, MAX_FRAMES_IN_FLIGHT,
	MaterializationKey, MemoryBackedResourceCreationResult, Mesh, Pipeline, PipelineLayout, PipelineLayoutKey,
	PipelineResourceDescriptor, ResolvedPipelineDescriptor, Sampler, Shader, Swapchain, Synchronizer,
	TopLevelAccelerationStructureHandle, TransitionState,
	utils::{
		into_vk_image_usage_flags, texture_format_and_resource_use_to_image_layout, to_format, to_shader_stage_flags,
		uses_to_vk_usage_flags,
	},
};
use crate::vulkan::{Device, InnerDevice, StoredQueue};
use crate::{
	FrameKey, HandleLike, MasterHandle as _, ResourceCollection, Size, graphics_hardware_interface, image, sampler,
	synchronizer::SynchronizerHandle,
	vulkan::{
		BufferCopy, BuildBuffer, CommandBufferRecording, Descriptor, Frame, ImageCopy, ImageHandle, MAX_SWAPCHAIN_IMAGES, Task,
		Tasks,
	},
	window,
};

#[cfg(test)]
mod descriptor_task_tests {
	use super::*;

	fn retained_set(write: crate::descriptors::DescriptorWrite, version: u64) -> DescriptorSet {
		let mut descriptors = HashMap::new();
		descriptors.entry(write.slot).or_insert_with(HashMap::new).insert(
			write.array_element,
			crate::vulkan::descriptor_set::RetainedDescriptor {
				descriptor: write.descriptor,
				frame_offset: write.frame_offset.unwrap_or(0),
			},
		);
		DescriptorSet {
			next: None,
			version,
			sequence_versions: [0; MAX_FRAMES_IN_FLIGHT],
			descriptors,
		}
	}

	fn buffer_write(buffer: u64) -> crate::descriptors::DescriptorWrite {
		crate::descriptors::DescriptorWrite::buffer(
			graphics_hardware_interface::DescriptorSetHandle(0),
			crate::shader::ResourceSlot::new(3),
			graphics_hardware_interface::BaseBufferHandle(buffer),
		)
	}

	#[test]
	fn deferred_descriptor_task_ignores_an_overwritten_payload() {
		let old = buffer_write(4);
		let current = buffer_write(5);
		let set = retained_set(current, 2);

		assert!(!descriptor_task_is_current(&set, old, 1));
		assert!(descriptor_task_is_current(&set, current, 2));
	}

	#[test]
	fn deferred_descriptor_task_uses_version_to_reject_aba_writes() {
		let value = buffer_write(7);
		let set = retained_set(value, 3);

		assert!(!descriptor_task_is_current(&set, value, 1));
		assert!(descriptor_task_is_current(&set, value, 3));
	}
}
