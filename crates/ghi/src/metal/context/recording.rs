use super::*;

impl Context {
	/// Waits for and releases one upload slot during frame retirement or a cross-queue handoff.
	pub(super) fn retire_internal_uploads(&mut self, sequence_index: u8) {
		if self.internal_upload_queues[sequence_index as usize].take().is_none() {
			return;
		}
		let synchronizer = synchronizer_for_sequence(&self.synchronizers, self.internal_upload_synchronizer, sequence_index);
		self.wait_for_private_synchronizer(synchronizer);
	}

	/// Releases the internal upload submissions that already completed, without waiting.
	///
	/// A frame retires its uploads when its sequence is reused, but detached recordings have no such point. They
	/// call this before submitting more uploads, and completion polls call it too, so retained staging pages and
	/// native commands stay bounded by the uploads still running instead of growing for the life of the context.
	pub(super) fn retire_completed_internal_uploads(&mut self) {
		for sequence_index in 0..self.internal_upload_queues.len() {
			if self.internal_upload_queues[sequence_index].is_none() {
				continue;
			}
			let synchronizer =
				synchronizer_for_sequence(&self.synchronizers, self.internal_upload_synchronizer, sequence_index as u8);
			if self.synchronizers.resource_mut(synchronizer).poll(&mut self.queues) {
				self.internal_upload_queues[sequence_index] = None;
			}
		}
	}

	pub fn new(
		settings: crate::device::Features,
		device: Retained<ProtocolObject<dyn mtl::MTLDevice>>,
		queues: Vec<queue::StoredQueue>,
	) -> Result<Context, &'static str> {
		let compiler_descriptor = mtl::MTL4CompilerDescriptor::new();
		if cfg!(debug_assertions) && settings.debug_labels {
			compiler_descriptor.setLabel(Some(&NSString::from_str("Byte Engine")));
		}
		// The context's factory and every factory it creates share this compiler.
		let compiler = device
			.newCompilerWithDescriptor_error(&compiler_descriptor)
			.map_err(|error| {
				eprintln!(
					"Metal 4 compiler creation failed: {}. The most likely cause is that Metal could not allocate a compiler for this device.",
					error.localizedDescription(),
				);
				"Metal 4 compiler creation failed. The most likely cause is that Metal could not allocate a compiler for this device."
			})?;
		let frames = MAX_FRAMES_IN_FLIGHT as u8;
		let mut synchronizers = ResourceCollection::with_capacity(32);
		// The context does not exist yet, so the internal upload synchronizer is built like `create_synchronizer` does.
		let internal_upload_synchronizer: graphics_hardware_interface::SynchronizerHandle =
			synchronizers.add_chain((0..frames).map(|_| Synchronizer::default()));
		Ok(Context {
			factory: Factory::new(device.clone(), compiler, settings),
			device,
			frames,
			queues,
			buffers: ResourceCollection::with_capacity(1024),
			images: ResourceCollection::with_capacity(1024),
			samplers: Vec::new(),
			allocations: Vec::new(),
			descriptor_sets: ResourceCollection::default(),
			meshes: Vec::new(),
			acceleration_structures: Vec::new(),
			pipelines: Vec::new(),
			command_buffers: Vec::new(),
			synchronizers,
			internal_upload_synchronizer,
			internal_upload_queues: vec![None; MAX_FRAMES_IN_FLIGHT],
			swapchains: Vec::new(),
			texture_readbacks: crate::context::TextureReadbackRegistry::new(),
			resource_to_descriptor: HashMap::default(),
			settings,
			pending_buffer_syncs: VecDeque::new(),
			pending_image_syncs: VecDeque::new(),
			tasks: Vec::new(),
			upload_arenas: (0..=MAX_FRAMES_IN_FLIGHT)
				.map(|_| command_buffer::UploadArena::new(settings.debug_labels))
				.collect(),
			argument_tables: Default::default(),
			image_groups: crate::image_group::ImageGroups::default(),
			next_group_heap_serial: 0,
		})
	}

	pub(super) fn create_buffer_resource(
		&mut self,
		name: Option<&str>,
		size: usize,
		resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
	) -> Buffer {
		let options = utils::resource_options_from_access(device_accesses);
		let name = crate::debug_name(name);
		let buffer = self
			.device
			.newBufferWithLength_options(size as _, options)
			.expect("Metal buffer creation failed. The most likely cause is that the device is out of memory.");

		let staging = (device_accesses == crate::DeviceAccesses::DeviceOnly).then(|| {
			self.device
				.newBufferWithLength_options(size as _, mtl::MTLResourceOptions::StorageModeShared)
				.expect("Metal staging buffer creation failed. The most likely cause is that the device is out of memory.")
		});

		#[cfg(debug_assertions)]
		if let Some(name) = name.as_deref().filter(|_| self.settings.debug_labels) {
			buffer.setLabel(Some(&NSString::from_str(name)));
			if let Some(staging) = staging.as_ref() {
				staging.setLabel(Some(&NSString::from_str(&format!("{name}_staging"))));
			}
		}

		// A device-only buffer is mapped through its staging copy.
		let pointer = staging.as_ref().unwrap_or(&buffer).contents().as_ptr().cast::<u8>();
		if size != 0 && !pointer.is_null() {
			// Typed buffer APIs expose the mapped bytes as zeroable POD values, so initialize the complete representation.
			unsafe { std::ptr::write_bytes(pointer, 0, size) };
		}
		let gpu_address = buffer.gpuAddress();
		let staging = staging.map(|staging| {
			self.buffers
				.add(Buffer {
					name: name.as_ref().map(|name| format!("{name}_staging")),
					staging: None,
					buffer: staging,
					size,
					gpu_address: 0,
					pointer,
					uses: resource_uses,
					access: crate::DeviceAccesses::HostToDevice,
				})
				.1
		});

		Buffer {
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

	/// Returns the typed CPU mapping of a buffer's copy for frame sequence `sequence_index`.
	///
	/// A buffer with one copy maps it for every sequence. A device-only buffer maps its staging copy.
	pub(crate) fn typed_buffer_pointer<T: ?Sized + crate::buffer::BufferContents>(
		&self,
		buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>,
		sequence_index: u8,
	) -> *mut T {
		let handle = self
			.buffers
			.nth_handle(buffer_handle.into(), sequence_index as usize)
			.expect("Missing Metal buffer. The most likely cause is that the buffer handle came from another context.");
		let buffer = self.buffers.resource(handle);
		<T as crate::buffer::BufferContents>::from_raw_parts(buffer.pointer, buffer.size).expect(
			"Failed to map a typed Metal buffer. The most likely cause is that the buffer has no sufficiently large, aligned CPU-visible storage.",
		)
	}

	/// Queues the staging upload of the copy of `buffer_handle` that frame sequence `sequence_index` uses.
	///
	/// Does nothing for a buffer without staging storage. The next recording on a queue submits the upload.
	pub(crate) fn sync_buffer_copy(
		&mut self,
		buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		sequence_index: u8,
	) {
		let handle = self
			.buffers
			.nth_handle(buffer_handle, sequence_index as usize)
			.expect("Missing Metal buffer. The most likely cause is that the buffer handle came from another context.");
		if self.buffers.resource(handle).staging.is_some() {
			self.pending_buffer_syncs.push_back(handle);
		}
	}

	/// Stores one resolved retained descriptor and advances the set version used by immutable native snapshots.
	pub(crate) fn update_descriptor_slot(
		&mut self,
		set_handle: DescriptorSetHandle,
		slot: crate::shader::ResourceSlot,
		descriptor: Descriptor,
		array_element: u32,
	) {
		let descriptor_set = self.descriptor_sets.resource_mut(set_handle);
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
		let binding = (set_handle, slot, array_element);
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
			crate::descriptors::WriteData::StaticSamplers | crate::descriptors::WriteData::CombinedImageSamplerArray => None,
			crate::descriptors::WriteData::AccelerationStructure { handle } => {
				Some(Descriptor::AccelerationStructure { handle })
			}
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

		for &(set_handle, ..) in bindings {
			let descriptor_set = self.descriptor_sets.resource_mut(set_handle);
			descriptor_set.version = descriptor_set.version.wrapping_add(1);
		}
	}

	/// Runs the tasks scheduled for `sequence_index` and keeps every other task for its own frame.
	pub(crate) fn process_tasks(&mut self, sequence_index: u8) {
		for task in std::mem::take(&mut self.tasks) {
			if task.frame != sequence_index {
				self.tasks.push(task);
				continue;
			}

			let handle = self
				.images
				.nth_handle(task.handle, sequence_index as usize)
				.expect("Missing Metal frame-local image. The most likely cause is an invalid dynamic image handle.");
			self.resize_image_internal(handle, task.extent);
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

		let description = ImageDescription {
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
		let (members, requirements): (Vec<_>, Vec<_>) = requests
			.iter()
			.map(|request| {
				let handle = ImageHandle(request.image.0);
				let description = ImageDescription {
					extent: request.extent,
					..self.images.resource(handle).description
				};
				let descriptor = build_texture_descriptor(description);
				let size_and_align = self.device.heapTextureSizeAndAlignWithDescriptor(&descriptor);
				let memory = crate::image_group::MemoryRequirements {
					size: size_and_align.size as u64,
					alignment: size_and_align.align as u64,
					// Every member lives in private storage, so any member can share a heap with any other.
					category: 0,
				};
				((handle, description, descriptor), (memory, request.lifetime.clone()))
			})
			.unzip();
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
			*image = Image {
				name: image.name.take(),
				texture,
				description,
				staging: None,
				slot: Some(GroupSlot {
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
}

/// Returns the private synchronizer a frame sequence signals for one public synchronizer.
///
/// It takes the synchronizer collection instead of the context, so split borrows such as
/// [`crate::metal::command_buffer::RecordingCommit`] can call it while other context fields are borrowed.
pub(crate) fn synchronizer_for_sequence(
	synchronizers: &ResourceCollection<
		Synchronizer,
		graphics_hardware_interface::SynchronizerHandle,
		crate::synchronizer::SynchronizerHandle,
	>,
	synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
	sequence_index: u8,
) -> crate::synchronizer::SynchronizerHandle {
	synchronizers
		.nth_handle(synchronizer_handle, sequence_index as usize)
		.expect("Missing Metal synchronizer. The most likely cause is that the synchronizer handle came from another context.")
}
