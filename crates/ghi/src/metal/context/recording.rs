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
			descriptor_sources: HashMap::default(),
			settings,
			pending_buffer_syncs: VecDeque::new(),
			pending_image_syncs: VecDeque::new(),
			tasks: Vec::new(),
			upload_arenas: (0..=MAX_FRAMES_IN_FLIGHT)
				.map(|_| command_buffer::UploadArena::new(settings.debug_labels))
				.collect(),
			argument_tables: command_buffer::CommandArgumentTables::default(),
			image_groups: crate::image_group::ImageGroups::default(),
			next_group_heap_serial: 0,
		};
		context.internal_upload_synchronizer = Some(context.create_synchronizer(Some("Metal Internal Upload Sync"), true));

		Ok(context)
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
		if size != 0 && !pointer.is_null() {
			// Typed buffer APIs expose the mapped bytes as zeroable POD values, so initialize the complete representation.
			unsafe { std::ptr::write_bytes(pointer, 0, size) };
		}
		let gpu_address = buffer.gpuAddress();
		let staging = staging.map(|staging| {
			let mut creator = self.buffers.creator();

			creator.add(buffer::Buffer {
				name: name.as_ref().map(|name| format!("{name}_staging")),
				staging: None,
				buffer: staging,
				size,
				gpu_address: 0,
				pointer,
				uses: resource_uses,
				access: crate::DeviceAccesses::HostToDevice,
			})
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

	/// Returns the typed CPU mapping of a buffer. A device-only buffer maps its staging copy.
	pub(super) fn typed_buffer_pointer<T: crate::Pod>(
		&self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> *mut T {
		let buffer = self.buffers.get_single(buffer_handle.into()).unwrap();
		crate::buffer::typed_buffer_pointer::<T>(buffer.pointer, buffer.size).expect(
			"Failed to map a typed Metal buffer. The most likely cause is that the buffer has no sufficiently large, aligned CPU-visible storage.",
		)
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
			if copy_size != 0 {
				assert!(
					!previous_buffer.pointer.is_null() && !buffer.pointer.is_null(),
					"Failed to preserve a resized Metal buffer. The most likely cause is that the old or replacement allocation is missing mapped storage.",
				);
				// SAFETY: The old and replacement buffers are distinct live allocations, and `copy_size` is bounded by both.
				unsafe {
					std::ptr::copy_nonoverlapping(previous_buffer.pointer, buffer.pointer, copy_size);
				}
			}
		}
		let (_, handle) = self.buffers.add(buffer);

		if let Some(previous) = previous {
			self.buffers.set_next(previous, Some(handle));
		}

		handle
	}

	/// Creates a Metal image and optionally links it after an existing private frame resource.
	pub(super) fn create_image_internal(
		&mut self,
		previous: Option<ImageHandle>,
		name: Option<&str>,
		description: image::ImageDescription,
	) -> ImageHandle {
		let image = build_image(&self.device, name, description, self.settings.debug_labels);
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
		let descriptor_set = &mut self.descriptor_sets[set_handle.0 as usize];
		let previous = descriptor_set
			.descriptors
			.entry(slot)
			.or_default()
			.insert(array_element, descriptor);
		if previous == Some(descriptor) {
			return;
		}
		descriptor_set.version = descriptor_set.version.wrapping_add(1);

		// Keep the reverse index in step so replacing a resource's backing can invalidate every set that binds it.
		let binding = (set_handle, slot, array_element, frame_index);
		if let Some(resource) = previous.and_then(Descriptor::tracked_resource)
			&& let Some(bindings) = self.resource_to_descriptor.get_mut(&resource)
		{
			bindings.remove(&binding);
			if bindings.is_empty() {
				self.resource_to_descriptor.remove(&resource);
			}
		}
		if let Some(resource) = descriptor.tracked_resource() {
			self.resource_to_descriptor.entry(resource).or_default().insert(binding);
		}
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

	/// Invalidates every retained set that references a resource whose native backing changed.
	pub(crate) fn rewrite_descriptors_for_handle(&mut self, handle: PrivateHandles) {
		let Some(bindings) = self.resource_to_descriptor.get(&handle) else {
			return;
		};

		for (set_handle, ..) in bindings {
			let descriptor_set = &mut self.descriptor_sets[set_handle.0 as usize];
			descriptor_set.version = descriptor_set.version.wrapping_add(1);
		}
	}

	/// Re-resolves retained descriptor writes after a deferred frame resource extends its chain.
	///
	/// `candidates` are the chain's private handles; a set that resolved to one of them may now resolve to the new one.
	pub(super) fn rewrite_deferred_descriptors(&mut self, candidates: &[PrivateHandles]) {
		let descriptor_bindings = candidates
			.iter()
			.filter_map(|candidate| self.resource_to_descriptor.get(candidate))
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
		let mut resized = false;
		for image_handle in self.swapchains[swapchain_handle.0 as usize].images.into_iter().flatten() {
			resized |= self.resize_image_internal(image_handle, extent);
		}

		if resized {
			// Swapchain descriptors resolve through the stable proxy handles, so only backing replacement invalidates them.
			self.rewrite_descriptors_for_handle(PrivateHandles::Swapchain(crate::swapchain::SwapchainHandle(
				swapchain_handle.0,
			)));
		}
	}

	/// Runs the tasks scheduled for `sequence_index` and keeps every other task for its own frame.
	pub(crate) fn process_tasks(&mut self, sequence_index: u8) {
		for task in std::mem::take(&mut self.tasks) {
			if task.frame != sequence_index {
				self.tasks.push(task);
				continue;
			}

			let next_frame = sequence_index + 1;
			match task.task {
				Tasks::BuildImage { previous, master } => {
					let previous_image = self.images.resource(previous);
					let (name, description) = (previous_image.name.clone(), previous_image.description);
					let handle = self.create_image_internal(Some(previous), name.as_deref(), description);

					let candidates = (0..self.frames as usize)
						.filter_map(|frame| self.images.nth_handle(master, frame).map(PrivateHandles::Image))
						.collect::<SmallVec<[_; MAX_FRAMES_IN_FLIGHT]>>();
					self.rewrite_deferred_descriptors(&candidates);

					if next_frame < self.frames {
						self.tasks.push(Task {
							task: Tasks::BuildImage {
								previous: handle,
								master,
							},
							frame: next_frame,
						});
					}
				}
				Tasks::BuildBuffer { previous, master } => {
					let previous_buffer = self.buffers.resource(previous);
					let name = previous_buffer.name.clone();
					let size = previous_buffer.size;
					let uses = previous_buffer.uses;
					let access = previous_buffer.access;
					let handle = self.create_buffer_internal(Some(previous), name.as_deref(), size, uses, access);

					let candidates = (0..self.frames as usize)
						.filter_map(|frame| self.buffers.nth_handle(master, frame).map(PrivateHandles::Buffer))
						.collect::<SmallVec<[_; MAX_FRAMES_IN_FLIGHT]>>();
					self.rewrite_deferred_descriptors(&candidates);

					if next_frame < self.frames {
						self.tasks.push(Task {
							task: Tasks::BuildBuffer {
								previous: handle,
								master,
							},
							frame: next_frame,
						});
					}
				}
				Tasks::ResizeImage { handle, extent } => {
					let handle = self
						.images
						.nth_handle(handle, sequence_index as usize)
						.expect("Missing Metal frame-local image. The most likely cause is an invalid dynamic image handle.");
					self.resize_image_internal(handle, extent);
				}
			}
		}
	}

	/// Replaces one frame-local image while preserving its private handle and descriptor references.
	///
	/// Returns `true` when the backing image changed.
	pub(crate) fn resize_image_internal(&mut self, handle: ImageHandle, extent: Extent) -> bool {
		let image = self.images.resource(handle);

		if image.description.extent == extent {
			return false;
		}

		let description = image::ImageDescription {
			extent,
			..image.description
		};
		let replacement = build_image(&self.device, image.name.as_deref(), description, self.settings.debug_labels);
		*self.images.resource_mut(handle) = replacement;
		self.rewrite_descriptors_for_handle(PrivateHandles::Image(handle));
		true
	}

	/// Recreates every member of an image group in heaps that members with disjoint lifetimes share.
	///
	/// Does nothing when the group is already placed from the same requests. Descriptors that reference a member
	/// keep working, since each member keeps its handle.
	pub(crate) fn place_image_group(
		&mut self,
		group: graphics_hardware_interface::ImageGroupHandle,
		requests: &[crate::ImageGroupMember],
	) {
		use objc2_metal::MTLHeap as _;

		let Some(requests) = self.image_groups.requests_in_member_order(group, requests) else {
			return;
		};

		// Describe each member at its requested extent and ask Metal how much heap memory it needs.
		let members = requests
			.iter()
			.map(|request| {
				let handle = ImageHandle(request.image.0);
				let description = image::ImageDescription {
					extent: request.extent,
					..self.images.resource(handle).description
				};
				(handle, description, build_texture_descriptor(description))
			})
			.collect::<Vec<_>>();
		let requirements = members
			.iter()
			.zip(&requests)
			.map(|((_, _, descriptor), request)| {
				let size_and_align = self.device.heapTextureSizeAndAlignWithDescriptor(descriptor);
				(
					crate::image_group::MemoryRequirements {
						size: size_and_align.size as u64,
						alignment: size_and_align.align as u64,
						// Every member lives in private storage, so any member can share a heap with any other.
						category: 0,
					},
					request.lifetime.clone(),
				)
			})
			.collect::<Vec<_>>();
		let placement = crate::image_group::pack(&requirements);

		let group_name = self.image_groups.group(group).name.clone();
		let heaps = placement
			.heaps
			.iter()
			.map(|layout| {
				let descriptor = mtl::MTLHeapDescriptor::new();
				descriptor.setType(mtl::MTLHeapType::Placement);
				descriptor.setStorageMode(mtl::MTLStorageMode::Private);
				// Recording tracks hazards by the heap bytes each member occupies, so Metal's own tracking is not needed.
				descriptor.setHazardTrackingMode(mtl::MTLHazardTrackingMode::Untracked);
				descriptor.setSize(layout.size as usize);
				let heap = self.device.newHeapWithDescriptor(&descriptor).expect(
					"Metal heap creation failed. The most likely cause is that the device is out of memory for the image group.",
				);
				#[cfg(debug_assertions)]
				if let Some(name) = group_name.as_deref().filter(|_| self.settings.debug_labels) {
					heap.setLabel(Some(&objc2_foundation::NSString::from_str(name)));
				}
				let serial = self.next_group_heap_serial;
				self.next_group_heap_serial += 1;
				(heap, serial)
			})
			.collect::<SmallVec<[_; 2]>>();

		for ((handle, description, descriptor), slot) in members.into_iter().zip(&placement.slots) {
			let (heap, heap_serial) = &heaps[slot.heap];
			// SAFETY: `pack` placed the member inside the heap, at an offset aligned to the alignment Metal reported
			// for this descriptor.
			let texture = unsafe { heap.newTextureWithDescriptor_offset(&descriptor, slot.offset as usize) }.expect(
				"Metal image-group texture creation failed. The most likely cause is a heap that is not a placement heap.",
			);
			let image = self.images.resource_mut(handle);
			#[cfg(debug_assertions)]
			if let Some(name) = image.name.as_deref().filter(|_| self.settings.debug_labels) {
				texture.setLabel(Some(&objc2_foundation::NSString::from_str(name)));
			}
			// In-flight commands retained the previous texture and heap, so replacing them here is safe.
			*image = image::Image {
				name: image.name.take(),
				texture,
				description,
				staging: None,
				slot: Some(image::GroupSlot {
					heap: heap.clone(),
					heap_serial: *heap_serial,
					offset: slot.offset as usize,
					size: slot.size as usize,
				}),
			};
			self.rewrite_descriptors_for_handle(PrivateHandles::Image(handle));
		}

		self.image_groups.commit(group, requests, placement);
	}

	/// Defers resize work until each other frame-local image can be replaced safely.
	pub(crate) fn resize_image_on_other_frames(
		&mut self,
		handle: graphics_hardware_interface::BaseImageHandle,
		extent: Extent,
		current_frame: u8,
	) {
		for offset in 1..self.frames {
			self.tasks.push(Task {
				task: Tasks::ResizeImage { handle, extent },
				frame: (current_frame + offset) % self.frames,
			});
		}
	}
}
