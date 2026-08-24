use super::*;

impl Context {
	// Acquires one reusable native command from the selected queue's context-local pool.
	pub(super) fn create_metal_command_buffer(
		&mut self,
		queue_handle: graphics_hardware_interface::QueueHandle,
		label: Option<&str>,
	) -> queue::NativeCommand {
		let queue = self.queues.get_mut(queue_handle.0 as usize).expect(
			"Metal command queue is missing. The most likely cause is that the queue handle came from another context.",
		);
		queue.acquire_native_command(label, self.settings.debug_labels)
	}

	pub(crate) fn synchronizer_for_sequence(
		&self,
		synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		sequence_index: u8,
	) -> crate::synchronizer::SynchronizerHandle {
		self.synchronizers
			.nth_handle(synchronizer_handle, sequence_index as usize)
			.expect(
				"Missing Metal synchronizer. The most likely cause is that the synchronizer handle came from another context.",
			)
	}

	/// Returns the frame-local synchronizer that owns internal upload submissions.
	pub(super) fn internal_upload_synchronizer(&self, sequence_index: u8) -> crate::synchronizer::SynchronizerHandle {
		let synchronizer = self.internal_upload_synchronizer.expect(
			"Metal internal upload synchronizer is missing. The most likely cause is that the context was not initialized correctly.",
		);
		self.synchronizer_for_sequence(synchronizer, sequence_index)
	}

	/// Waits for and releases one upload slot during frame retirement or a cross-queue handoff.
	pub(super) fn retire_internal_uploads(&mut self, sequence_index: u8) {
		if self.internal_upload_queues[sequence_index as usize].take().is_none() {
			return;
		}
		let synchronizer = self.internal_upload_synchronizer(sequence_index);
		self.wait_for_private_synchronizer(synchronizer);
	}

	/// Waits only for outstanding internal uploads submitted to another Metal queue.
	pub(super) fn synchronize_internal_upload_queue(&mut self, queue_handle: graphics_hardware_interface::QueueHandle) {
		for sequence_index in 0..self.internal_upload_queues.len() {
			if self.internal_upload_queues[sequence_index].is_some_and(|owner| owner != queue_handle) {
				self.retire_internal_uploads(sequence_index as u8);
			}
		}
	}

	pub fn new(
		settings: crate::device::Features,
		device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
		queues: Vec<queue::StoredQueue>,
	) -> Result<Context, &'static str> {
		let compiler = create_metal4_compiler(device.as_ref(), settings.debug_labels)?;
		let mut context = Context {
			device,
			compiler,
			frames: MAX_FRAMES_IN_FLIGHT as u8,
			queues,
			buffers: ResourceCollection::with_capacity(1024),
			images: ResourceCollection::with_capacity(1024),
			samplers: Vec::new(),
			allocations: Vec::new(),
			pipeline_layouts: Vec::new(),
			vertex_layouts: Vec::new(),
			vertex_layout_indices: HashMap::default(),
			descriptor_sets: Vec::new(),
			meshes: Vec::new(),
			acceleration_structures: Vec::new(),
			shaders: Vec::new(),
			pipelines: Vec::new(),
			command_buffers: Vec::new(),
			synchronizers: ResourceCollection::with_capacity(32),
			internal_upload_synchronizer: None,
			internal_upload_queues: vec![None; MAX_FRAMES_IN_FLIGHT],
			swapchains: Vec::new(),
			texture_readbacks: crate::context::TextureReadbackRegistry::new(),
			resource_to_descriptor: HashMap::default(),
			descriptor_set_to_resource: HashMap::default(),
			descriptor_sources: HashMap::default(),
			settings,
			pending_buffer_syncs: VecDeque::new(),
			pending_image_syncs: VecDeque::new(),
			tasks: Vec::new(),

			#[cfg(debug_assertions)]
			names: HashMap::default(),
		};
		context.internal_upload_synchronizer = Some(context.create_synchronizer(Some("Metal Internal Upload Sync"), true));

		Ok(context)
	}

	pub fn create_factory(&self) -> Option<crate::metal::factory::Factory> {
		Some(crate::metal::factory::Factory::new(
			self.device.clone(),
			self.compiler.clone(),
			self.settings,
		))
	}

	pub(super) fn create_buffer_resource(
		&mut self,
		name: Option<&str>,
		size: usize,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
	) -> buffer::Buffer {
		let options = utils::resource_options_from_access(device_accesses);
		let name = crate::debug_name(name);
		let buffer = self
			.device
			.newBufferWithLength_options(size as _, options)
			.expect("Metal buffer creation failed. The most likely cause is that the device is out of memory.");

		let staging = if device_accesses == crate::DeviceAccesses::DeviceOnly {
			Some(
				self.device
					.newBufferWithLength_options(size as _, mtl::MTLResourceOptions::StorageModeShared)
					.expect("Metal staging buffer creation failed. The most likely cause is that the device is out of memory."),
			)
		} else {
			None
		};

		#[cfg(debug_assertions)]
		if self.settings.debug_labels {
			if let Some(name) = name.as_deref() {
				buffer.setLabel(Some(&NSString::from_str(name)));
				if let Some(staging) = staging.as_ref() {
					staging.setLabel(Some(&NSString::from_str(&format!("{name}_staging"))));
				}
			}
		}

		let pointer = staging
			.as_ref()
			.map(|staging| staging.contents().as_ptr() as *mut u8)
			.unwrap_or_else(|| buffer.contents().as_ptr() as *mut u8);
		let gpu_address = buffer.gpuAddress();
		let staging = staging.map(|staging| {
			let mut creator = self.buffers.creator();
			let handle = creator.add(buffer::Buffer {
				name: name.as_ref().map(|name| format!("{name}_staging")),
				staging: None,
				buffer: staging,
				size,
				gpu_address: 0,
				pointer,
				uses: resource_uses,
				access: crate::DeviceAccesses::HostToDevice,
			});
			handle
		});

		buffer::Buffer {
			name,
			buffer,
			staging,
			size,
			gpu_address,
			pointer,
			uses: resource_uses,
			access: device_accesses,
		}
	}

	/// Creates a Metal buffer and optionally links it after an existing private frame resource.
	pub(super) fn create_buffer_internal(
		&mut self,
		previous: Option<BufferHandle>,
		name: Option<&str>,
		size: usize,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
	) -> BufferHandle {
		let buffer = self.create_buffer_resource(name, size, resource_uses, device_accesses);
		if let Some(previous) = previous {
			let previous_buffer = self.buffers.resource(previous);
			let copy_size = previous_buffer.size.min(buffer.size);
			unsafe {
				std::ptr::copy_nonoverlapping(previous_buffer.pointer, buffer.pointer, copy_size);
			}
		}
		let (_, handle) = self.buffers.add(buffer);

		if let Some(previous) = previous {
			self.buffers.set_next(previous, Some(handle));
		}

		handle
	}

	pub(super) fn create_image_resource(
		&self,
		name: Option<&str>,
		extent: Extent,
		format: crate::Formats,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
		array_layers: u32,
		cube_compatible: bool,
		cube_array_compatible: bool,
		mip_levels: u32,
	) -> image::Image {
		let name = crate::debug_name(name);

		let descriptor = build_texture_descriptor(
			format,
			extent,
			resource_uses,
			device_accesses,
			array_layers,
			cube_compatible,
			cube_array_compatible,
			mip_levels,
		);

		let texture = self
			.device
			.newTextureWithDescriptor(&descriptor)
			.expect("Metal texture creation failed. The most likely cause is that the device is out of memory.");

		#[cfg(debug_assertions)]
		if self.settings.debug_labels {
			if let Some(name) = name.as_deref() {
				texture.setLabel(Some(&NSString::from_str(name)));
			}
		}

		let staging = utils::texture_upload_layout(format, extent).map(|(_, _, bytes_per_image)| {
			let depth = extent.depth().max(1) as usize;
			let size = bytes_per_image * depth * array_layers as usize;
			vec![0u8; size]
		});

		image::Image {
			name,
			texture,
			extent,
			format,
			uses: resource_uses,
			access: device_accesses,
			array_layers,
			cube_compatible,
			cube_array_compatible,
			mip_levels,
			staging,
		}
	}

	/// Creates a Metal image and optionally links it after an existing private frame resource.
	pub(super) fn create_image_internal(
		&mut self,
		previous: Option<ImageHandle>,
		name: Option<&str>,
		extent: Extent,
		format: crate::Formats,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
		array_layers: u32,
		cube_compatible: bool,
		cube_array_compatible: bool,
		mip_levels: u32,
	) -> ImageHandle {
		let image = self.create_image_resource(
			name,
			extent,
			format,
			resource_uses,
			device_accesses,
			array_layers,
			cube_compatible,
			cube_array_compatible,
			mip_levels,
		);
		let (_, handle) = self.images.add(image);

		if let Some(previous) = previous {
			self.images.set_next(previous, Some(handle));
		}

		handle
	}

	/// Stores one resolved retained descriptor and advances the set version used by immutable native snapshots.
	pub(crate) fn update_descriptor_slot(
		&mut self,
		set_handle: DescriptorSetHandle,
		slot: crate::shader::ResourceSlot,
		descriptor: Descriptor,
		frame_index: u8,
		array_element: u32,
	) {
		let previous = self.descriptor_sets[set_handle.0 as usize]
			.descriptors
			.get(&slot)
			.and_then(|descriptors| descriptors.get(&array_element))
			.copied();
		if previous == Some(descriptor) {
			return;
		}

		self.clear_descriptor_tracking(set_handle, slot, array_element, frame_index);
		let descriptor_set = &mut self.descriptor_sets[set_handle.0 as usize];
		descriptor_set
			.descriptors
			.entry(slot)
			.or_default()
			.insert(array_element, descriptor);
		descriptor_set.version = descriptor_set.version.wrapping_add(1);
		self.register_descriptor_tracking(set_handle, slot, descriptor, array_element, frame_index);
	}

	/// Removes reverse-tracking entries for the descriptor currently associated with one binding element in one frame.
	pub(super) fn clear_descriptor_tracking(
		&mut self,
		set_handle: DescriptorSetHandle,
		slot: crate::shader::ResourceSlot,
		array_element: u32,
		frame_index: u8,
	) {
		let key = (set_handle, slot, array_element, frame_index);
		let Some(resources) = self.descriptor_set_to_resource.remove(&key) else {
			return;
		};

		for resource in resources {
			let should_remove = if let Some(descriptor_bindings) = self.resource_to_descriptor.get_mut(&resource) {
				descriptor_bindings.remove(&(set_handle, slot, array_element, frame_index));
				descriptor_bindings.is_empty()
			} else {
				false
			};

			if should_remove {
				self.resource_to_descriptor.remove(&resource);
			}
		}
	}

	/// Registers reverse-tracking for resource-backed descriptors so later resource changes can re-encode the affected bindings.
	pub(super) fn register_descriptor_tracking(
		&mut self,
		set_handle: DescriptorSetHandle,
		slot: crate::shader::ResourceSlot,
		descriptor: Descriptor,
		array_element: u32,
		frame_index: u8,
	) {
		let Some(resource) = descriptor.tracked_resource() else {
			return;
		};

		self.descriptor_set_to_resource
			.entry((set_handle, slot, array_element, frame_index))
			.or_default()
			.insert(resource);
		self.resource_to_descriptor
			.entry(resource)
			.or_default()
			.insert((set_handle, slot, array_element, frame_index));
	}

	/// Resolves a descriptor write into the concrete per-frame Metal resources referenced by the current sequence.
	pub(super) fn resolve_descriptor_for_frame(
		&self,
		descriptor: crate::descriptors::WriteData,
		sequence_index: u8,
		frame_offset: i32,
	) -> Option<Descriptor> {
		let resource_frame_index =
			crate::frame_resources::frame_index_with_offset(sequence_index as usize, frame_offset, self.frames as usize);

		match descriptor {
			crate::descriptors::WriteData::Buffer { handle, size } => {
				let handle = self.buffers.nth_handle(handle, resource_frame_index)?;
				Some(Descriptor::Buffer { buffer: handle, size })
			}
			crate::descriptors::WriteData::Image {
				handle,
				layout,
				mip_level,
			} => {
				let handle = self.images.nth_handle(handle, resource_frame_index)?;
				Some(Descriptor::Image {
					image: handle,
					layout,
					mip_level,
				})
			}
			crate::descriptors::WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				..
			} => {
				let handle = self.images.nth_handle(image_handle, resource_frame_index)?;
				Some(Descriptor::CombinedImageSampler {
					image: handle,
					sampler: SamplerHandle(sampler_handle.0),
					layout,
				})
			}
			crate::descriptors::WriteData::Sampler(handle) => Some(Descriptor::Sampler {
				sampler: SamplerHandle(handle.0),
			}),
			crate::descriptors::WriteData::StaticSamplers => None,
			crate::descriptors::WriteData::CombinedImageSamplerArray => None,
			crate::descriptors::WriteData::AccelerationStructure { handle } => Some(Descriptor::AccelerationStructure {
				handle: TopLevelAccelerationStructureHandle(handle.0),
			}),
			crate::descriptors::WriteData::Swapchain(swapchain_handle) => Some(Descriptor::Swapchain {
				handle: crate::swapchain::SwapchainHandle(swapchain_handle.0),
			}),
		}
	}

	/// Resolves and applies a descriptor write for a single frame when the referenced resources are available.
	pub(super) fn apply_descriptor_write_for_frame(
		&mut self,
		set_handle: DescriptorSetHandle,
		slot: crate::shader::ResourceSlot,
		descriptor: crate::descriptors::WriteData,
		array_element: u32,
		frame_offset: i32,
		sequence_index: u8,
	) {
		self.descriptor_sources
			.insert((set_handle, slot, array_element, sequence_index), (descriptor, frame_offset));
		if let Some(descriptor) = self.resolve_descriptor_for_frame(descriptor, sequence_index, frame_offset) {
			self.update_descriptor_slot(set_handle, slot, descriptor, sequence_index, array_element);
		}
	}

	/// Applies the same descriptor write across every frame tracked by the Metal device.
	/// Call this to update a descriptor binding for all frames.
	pub(super) fn apply_descriptor_write_to_all_frames(
		&mut self,
		set_handle: DescriptorSetHandle,
		slot: crate::shader::ResourceSlot,
		descriptor: crate::descriptors::WriteData,
		array_element: u32,
		frame_offset: i32,
	) {
		let set_handles = set_handle.root(&self.descriptor_sets).get_all(&self.descriptor_sets);

		for (sequence_index, &set_handle) in set_handles.iter().enumerate() {
			self.apply_descriptor_write_for_frame(
				set_handle,
				slot,
				descriptor,
				array_element,
				frame_offset,
				sequence_index as u8,
			);
		}
	}

	/// Invalidates every retained set that references a resource whose native backing changed.
	pub(crate) fn rewrite_descriptors_for_handle(&mut self, handle: PrivateHandles) {
		let Some(descriptor_bindings) = self.resource_to_descriptor.get(&handle).cloned() else {
			return;
		};

		for (set_handle, ..) in descriptor_bindings {
			let descriptor_set = &mut self.descriptor_sets[set_handle.0 as usize];
			descriptor_set.version = descriptor_set.version.wrapping_add(1);
		}
	}

	/// Returns the private buffer handles currently known for one master buffer chain.
	pub(super) fn buffer_chain_handles(&self, master: graphics_hardware_interface::BaseBufferHandle) -> Vec<PrivateHandles> {
		let mut handles = Vec::with_capacity(self.frames as usize);

		for frame_index in 0..self.frames as usize {
			let Some(handle) = self.buffers.nth_handle(master, frame_index) else {
				continue;
			};
			let handle = PrivateHandles::Buffer(handle);

			if !handles.contains(&handle) {
				handles.push(handle);
			}
		}

		handles
	}

	/// Returns the private image handles currently known for one master image chain.
	pub(super) fn image_chain_handles(&self, master: graphics_hardware_interface::BaseImageHandle) -> Vec<PrivateHandles> {
		let mut handles = Vec::with_capacity(self.frames as usize);

		for frame_index in 0..self.frames as usize {
			let Some(handle) = self.images.nth_handle(master, frame_index) else {
				continue;
			};
			let handle = PrivateHandles::Image(handle);

			if !handles.contains(&handle) {
				handles.push(handle);
			}
		}

		handles
	}

	/// Re-resolves retained descriptor writes after a deferred frame resource extends its chain.
	pub(super) fn rewrite_deferred_descriptors(&mut self, candidates: &[PrivateHandles]) {
		let descriptor_bindings = candidates
			.iter()
			.copied()
			.filter_map(|candidate| self.resource_to_descriptor.get(&candidate))
			.flat_map(|bindings| bindings.iter().copied())
			.collect::<HashSet<_>>();

		for (set_handle, slot, array_element, frame_index) in descriptor_bindings {
			let Some((source, frame_offset)) = self
				.descriptor_sources
				.get(&(set_handle, slot, array_element, frame_index))
				.copied()
			else {
				continue;
			};
			let Some(descriptor) = self.resolve_descriptor_for_frame(source, frame_index, frame_offset) else {
				continue;
			};

			self.update_descriptor_slot(set_handle, slot, descriptor, frame_index, array_element);
		}
	}

	/// Resizes every swapchain proxy image in place so existing descriptors can keep their image handles.
	pub(crate) fn resize_swapchain_images(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
		extent: Extent,
	) {
		let image_handles = self.swapchains[swapchain_handle.0 as usize].images;
		let mut resized = false;

		for image_handle in image_handles.into_iter().flatten() {
			let (current_extent, format, uses, access, array_layers, cube_compatible, cube_array_compatible, mip_levels) = {
				let image = self.images.resource(image_handle);
				(
					image.extent,
					image.format,
					image.uses,
					image.access,
					image.array_layers,
					image.cube_compatible,
					image.cube_array_compatible,
					image.mip_levels,
				)
			};

			if current_extent == extent {
				continue;
			}

			let name = self.images.resource(image_handle).name.clone();
			let replacement = self.create_image_resource(
				name.as_deref(),
				extent,
				format,
				uses,
				access,
				array_layers,
				cube_compatible,
				cube_array_compatible,
				mip_levels,
			);
			*self.images.resource_mut(image_handle) = replacement;
			self.rewrite_descriptors_for_handle(PrivateHandles::Image(image_handle));
			resized = true;
		}

		if resized {
			// Swapchain descriptors resolve through the stable proxy handles, so only backing replacement invalidates them.
			self.rewrite_descriptors_for_handle(PrivateHandles::Swapchain(crate::swapchain::SwapchainHandle(
				swapchain_handle.0,
			)));
		}
	}

	pub(crate) fn process_tasks(&mut self, sequence_index: u8) {
		let mut tasks = std::mem::take(&mut self.tasks);
		let mut deferred_frame_tasks = SmallVec::<[Task; 16]>::new(); // TODO: use frame allocator

		tasks.retain(|task| {
			if let Some(frame) = task.frame() {
				if frame != sequence_index {
					return true;
				}
			}

			match task.task() {
				Tasks::UpdateBufferDescriptors { handle } => {
					self.rewrite_descriptors_for_handle(PrivateHandles::Buffer(*handle));
				}
				Tasks::UpdateImageDescriptors { handle } => {
					self.rewrite_descriptors_for_handle(PrivateHandles::Image(*handle));
				}
				Tasks::BuildImage(builder) => {
					let previous = self.images.resource(builder.previous);
					let name = previous.name.clone();
					let extent = previous.extent;
					let format = previous.format;
					let uses = previous.uses;
					let access = previous.access;
					let array_layers = previous.array_layers;
					let cube_compatible = previous.cube_compatible;
					let cube_array_compatible = previous.cube_array_compatible;
					let mip_levels = previous.mip_levels;
					let handle = self.create_image_internal(
						Some(builder.previous),
						name.as_deref(),
						extent,
						format,
						uses,
						access,
						array_layers,
						cube_compatible,
						cube_array_compatible,
						mip_levels,
					);

					let candidates = self.image_chain_handles(builder.master.0);
					self.rewrite_deferred_descriptors(&candidates);

					let next_frame = sequence_index + 1;
					if next_frame < self.frames {
						deferred_frame_tasks.push(Task::new(
							Tasks::BuildImage(BuildImage {
								previous: handle,
								master: builder.master,
							}),
							Some(next_frame),
						));
					}
				}
				Tasks::BuildBuffer(builder) => {
					let previous = self.buffers.resource(builder.previous);
					let name = previous.name.clone();
					let size = previous.size;
					let uses = previous.uses;
					let access = previous.access;
					let handle = self.create_buffer_internal(Some(builder.previous), name.as_deref(), size, uses, access);

					let candidates = self.buffer_chain_handles(builder.master);
					self.rewrite_deferred_descriptors(&candidates);

					let next_frame = sequence_index + 1;
					if next_frame < self.frames {
						deferred_frame_tasks.push(Task::new(
							Tasks::BuildBuffer(BuildBuffer {
								previous: handle,
								master: builder.master,
							}),
							Some(next_frame),
						));
					}
				}
				Tasks::ResizeImage { handle, extent } => {
					let handle = self
						.images
						.nth_handle(*handle, sequence_index as usize)
						.expect("Missing Metal frame-local image. The most likely cause is an invalid dynamic image handle.");
					self.resize_image_internal(handle, *extent);
				}
				Tasks::DeleteMetalTexture { .. } | Tasks::DeleteMetalBuffer { .. } => {}
			}

			false
		});

		tasks.extend(deferred_frame_tasks);
		self.tasks = tasks;
	}

	/// Replaces one frame-local image while preserving its private handle and descriptor references.
	///
	/// Returns `true` when the backing image changed.
	pub(crate) fn resize_image_internal(&mut self, handle: ImageHandle, extent: Extent) -> bool {
		let image = self.images.resource(handle);

		if image.extent == extent {
			return false;
		}

		let replacement = self.create_image_resource(
			image.name.as_deref(),
			extent,
			image.format,
			image.uses,
			image.access,
			image.array_layers,
			image.cube_compatible,
			image.cube_array_compatible,
			image.mip_levels,
		);
		*self.images.resource_mut(handle) = replacement;
		self.rewrite_descriptors_for_handle(PrivateHandles::Image(handle));
		true
	}

	/// Defers resize work until each other frame-local image can be replaced safely.
	pub(crate) fn resize_image_on_other_frames(
		&mut self,
		handle: graphics_hardware_interface::BaseImageHandle,
		extent: Extent,
		current_frame: u8,
	) {
		for offset in 1..self.frames {
			let frame = (current_frame + offset).rem_euclid(self.frames);
			self.tasks.push(Task::new(Tasks::ResizeImage { handle, extent }, Some(frame)));
		}
	}
}
