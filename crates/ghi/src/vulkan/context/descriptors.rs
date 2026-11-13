use super::*;
use crate::descriptors::WriteData;
use crate::vulkan::descriptor_set::{CopyKey, Reader, RetainedDescriptor, SetConsumption, SetCopy};
use crate::vulkan::{Completion, FIXED_SAMPLER_COUNT, RESOURCE_DESCRIPTOR_SIZE, holds_samplers};

impl Context {
	/// Creates one mapped descriptor heap of `size` bytes that counts in descriptors of `unit` bytes.
	///
	/// The implementation's reserved range goes at the end of the heap, so heap index 0 belongs to the application,
	/// where the sampler heap keeps its fixed samplers.
	pub(crate) fn create_descriptor_heap_arena(
		&mut self,
		name: &str,
		size: u64,
		heap_alignment: u64,
		unit: u64,
		reserved_size: u64,
	) -> DescriptorHeapArena {
		assert!(
			size <= u32::MAX as u64,
			"Vulkan descriptor heap exceeds the 32-bit index space. The most likely cause is an implementation reservation larger than base indices can address.",
		);
		let reserved_offset = size.checked_sub(reserved_size).expect(
			"Vulkan descriptor heap is smaller than its reserved range. The most likely cause is invalid descriptor-heap properties.",
		) / heap_alignment.max(unit)
			* heap_alignment.max(unit);
		let units = reserved_offset / unit;
		assert!(
			units > 0,
			"Vulkan descriptor heap has no application-owned range. The most likely cause is that the implementation reservation consumes the device's maximum heap size.",
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
		// Prefers device-local host-visible memory and falls back to plain host memory when the device has none.
		let device_accesses = crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead;
		let (allocation, _) = self.create_allocation_internal(
			creation.size,
			creation.memory_flags.into(),
			device_accesses,
			creation.dedicated_resource(),
		);
		let (device_address, pointer) = self.bind_vulkan_buffer_memory(&creation, allocation, 0);
		let aligned_address = device_address.next_multiple_of(heap_alignment);
		let base_offset = aligned_address - device_address;

		assert!(base_offset + size <= buffer_size);

		DescriptorHeapArena {
			buffer: creation.resource,
			pointer: crate::vulkan::MappedMemoryPointer(unsafe { pointer.add(base_offset as usize) }),
			device_address: aligned_address,
			size,
			unit,
			reserved_offset,
			reserved_size,
			free: ::utils::RangeAllocator::new(units as usize, 64),
		}
	}

	/// Allocates the long-lived resource and sampler heaps of the context, and writes the fixed samplers at the first
	/// [`FIXED_SAMPLER_COUNT`] sampler heap indices. The first descriptor set creates them.
	pub(crate) fn create_descriptor_heaps(&mut self) -> DescriptorHeaps {
		const RESOURCE_HEAP_TARGET_SIZE: u64 = 64 * 1024 * 1024;
		const SAMPLER_HEAP_TARGET_SIZE: u64 = 1024 * 1024;
		let properties = self.device.descriptor_heap_properties;
		let sampler_size = properties.sampler_descriptor_size;
		assert!(
			sampler_size > 0 && sampler_size.is_multiple_of(properties.sampler_descriptor_alignment.max(1)),
			"Vulkan sampler descriptors can't be indexed by their size. The most likely cause is a device whose sampler descriptor size isn't a multiple of its alignment, which this backend doesn't support.",
		);
		let resource_minimum = properties.min_resource_heap_reserved_range + RESOURCE_DESCRIPTOR_SIZE;
		let sampler_minimum = properties.min_sampler_heap_reserved_range + sampler_size * u64::from(FIXED_SAMPLER_COUNT + 1);
		assert!(
			resource_minimum <= properties.max_resource_heap_size && sampler_minimum <= properties.max_sampler_heap_size,
			"Vulkan descriptor heap limits are inconsistent. The most likely cause is that the implementation reservation leaves no room for the application's descriptors.",
		);
		let resource_size = RESOURCE_HEAP_TARGET_SIZE
			.max(resource_minimum)
			.min(properties.max_resource_heap_size);
		let sampler_size_bytes = SAMPLER_HEAP_TARGET_SIZE
			.max(sampler_minimum)
			.min(properties.max_sampler_heap_size);

		let resource = self.create_descriptor_heap_arena(
			"GHI Resource Descriptor Heap",
			resource_size,
			properties.resource_heap_alignment.max(1),
			RESOURCE_DESCRIPTOR_SIZE,
			properties.min_resource_heap_reserved_range,
		);
		let mut sampler = self.create_descriptor_heap_arena(
			"GHI Sampler Descriptor Heap",
			sampler_size_bytes,
			properties.sampler_heap_alignment.max(1),
			sampler_size,
			properties.min_sampler_heap_reserved_range,
		);

		// Shaders index fixed samplers by their state, so they take the first indices, and sets' samplers come after.
		let fixed = sampler.allocate(FIXED_SAMPLER_COUNT);
		assert_eq!(fixed, 0, "The fixed samplers must start the sampler heap.");
		let samplers = std::array::from_fn::<_, { FIXED_SAMPLER_COUNT as usize }, _>(|index| {
			Sampler::fixed(crate::vulkan::fixed_sampler(index as u32))
		});
		let reductions = samplers
			.each_ref()
			.map(|sampler| vk::SamplerReductionModeCreateInfo::default().reduction_mode(sampler.reduction_mode));
		let infos = std::array::from_fn::<_, { FIXED_SAMPLER_COUNT as usize }, _>(|index| {
			let mut info = samplers[index].create_info();
			info.p_next = (&raw const reductions[index]).cast();
			info
		});
		let destinations = std::array::from_fn::<_, { FIXED_SAMPLER_COUNT as usize }, _>(|index| {
			sampler.host_range(index as u32, sampler_size)
		});
		// SAFETY: Every create info and the reduction it points at live until the call returns, and every destination
		// is mapped heap memory that no command reads yet.
		unsafe {
			self.device
				.descriptor_heap
				.write_sampler_descriptors(&infos, &destinations)
				.expect(
					"Vulkan fixed sampler write failed. The most likely cause is a device without support for min and max sampler reductions.",
				);
		}

		DescriptorHeaps { resource, sampler }
	}

	/// Creates a set for `layout` with a heap copy for recordings outside frames and one per frame in flight, whose
	/// ranges it holds until the context drops.
	pub(crate) fn create_descriptor_set_internal(
		&mut self,
		layout: &crate::shader::DescriptorSetLayout,
	) -> graphics_hardware_interface::DescriptorSetHandle {
		let mut sampler_count = 0u32;
		let sampler_bases = layout
			.resources()
			.iter()
			.map(|resource| {
				let base = sampler_count;
				if holds_samplers(resource.kind()) {
					sampler_count += resource.count();
				}
				base
			})
			.collect::<Box<[_]>>();
		if self.descriptor_heaps.is_none() {
			self.descriptor_heaps = Some(self.create_descriptor_heaps());
		}
		let element_count = layout.element_count();
		let copies = (0..=self.frames)
			.map(|_| self.allocate_set_copy(element_count, sampler_count))
			.collect();

		let handle = graphics_hardware_interface::DescriptorSetHandle::from_index(self.descriptor_sets.len() as u64);
		self.descriptor_sets.push(DescriptorSet {
			next: None,
			layout: layout.clone(),
			sampler_bases,
			sampler_count,
			descriptors: vec![None; element_count as usize].into_boxed_slice(),
			copies,
		});
		handle
	}

	/// Takes the heap ranges of one copy of a set with `element_count` descriptors and `sampler_count` samplers.
	pub(crate) fn allocate_set_copy(&mut self, element_count: u32, sampler_count: u32) -> SetCopy {
		let heaps = self.descriptor_heaps.as_mut().expect(
			"Missing Vulkan descriptor heaps. The most likely cause is that a set copy was allocated before the first set created the heaps.",
		);
		SetCopy {
			resource_base: heaps.resource.allocate(element_count),
			sampler_base: heaps.sampler.allocate(sampler_count),
			dirty: Vec::new(),
			consumptions: Vec::new(),
			reader: None,
		}
	}

	/// Gives every set a copy for each frame sequence from `first_sequence` to the frames in flight, with every
	/// written element still to write.
	pub(crate) fn add_descriptor_set_copies(&mut self, first_sequence: u8) {
		for set_index in 0..self.descriptor_sets.len() {
			let set = &self.descriptor_sets[set_index];
			let (element_count, sampler_count) = (set.descriptors.len() as u32, set.sampler_count);
			for _ in first_sequence..self.frames {
				let mut copy = self.allocate_set_copy(element_count, sampler_count);
				let descriptors = &self.descriptor_sets[set_index].descriptors;
				copy.dirty
					.extend((0..element_count).filter(|&element| descriptors[element as usize].is_some()));
				self.descriptor_sets[set_index].copies.push(copy);
			}
		}
	}

	/// Returns the resource whose storage replacements rewrite a write of `descriptor`: its first frame-local copy, or
	/// a swapchain's first image. Samplers and acceleration structures keep their storage.
	fn reference_key(&self, descriptor: WriteData) -> Option<super::super::Handles> {
		use super::super::Handles;
		match descriptor {
			WriteData::Buffer { handle } => Some(Handles::Buffer(BufferHandle::from_index(handle.index()))),
			WriteData::Image { handle, .. }
			| WriteData::CombinedImageSampler {
				image_handle: handle, ..
			} => Some(Handles::Image(ImageHandle::from_index(handle.index()))),
			WriteData::Swapchain(handle) => Some(Handles::Image(self.swapchains[handle.index() as usize].images[0])),
			WriteData::Sampler(_) | WriteData::AccelerationStructure { .. } => None,
		}
	}

	/// Retains each write in its set, and marks its element to write into every copy of the set before commands bind
	/// the copy again. A write that repeats the element's current one changes nothing.
	///
	/// # Panics
	///
	/// Panics when a write names a slot the set's layout doesn't declare, an array element past the resource's count,
	/// or a resource of another kind than the layout declares.
	pub(crate) fn write_descriptors(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		for &descriptor_write in descriptor_set_writes {
			let retained = RetainedDescriptor {
				descriptor: descriptor_write.descriptor,
				frame_offset: descriptor_write.frame_offset.unwrap_or(0),
			};
			let set_index = descriptor_write.descriptor_set.index();
			let set = self.descriptor_sets.get_mut(set_index as usize).expect(
				"Invalid Vulkan descriptor set. The most likely cause is that the write used a handle from another context. Write sets the context created.",
			);
			let layout = &set.layout;
			let slot = descriptor_write.slot;
			let resource_index = layout.resource_index(slot).unwrap_or_else(|| {
				panic!(
					"Descriptor set layout `{}` declares no resource at slot {}. The most likely cause is that the write names a resource of another descriptor set, or an array element by its flat slot instead of its array element. Write the slot the set's layout declares, and pick array elements with `with_array_element`.",
					layout.name(),
					slot.index(),
				)
			});
			let resource = layout.resources()[resource_index];
			assert!(
				descriptor_write.array_element < resource.count(),
				"Descriptor array element {} is out of range at slot {} of layout `{}`. The most likely cause is that a write exceeded the resource count the layout declares. Write elements below {}.",
				descriptor_write.array_element,
				slot.index(),
				layout.name(),
				resource.count(),
			);
			assert!(
				retained.descriptor.matches_kind(resource.kind()),
				"Descriptor kind mismatch at slot {} of layout `{}`. The most likely cause is that an image, sampler, or buffer was written where the layout declares a {:?}. Write the kind of resource the layout declares.",
				slot.index(),
				layout.name(),
				resource.kind(),
			);
			let element = layout.element_base(resource_index) + descriptor_write.array_element;
			let previous = set.descriptors[element as usize].replace(retained);
			if previous == Some(retained) {
				continue;
			}
			for copy in &mut set.copies {
				copy.dirty.push(element);
			}

			// Keep the reverse index in step, so replacing a resource's storage finds every element that names it.
			let previous_key = previous.and_then(|previous| self.reference_key(previous.descriptor));
			let key = self.reference_key(retained.descriptor);
			if previous_key == key {
				continue;
			}
			if let Some(previous_key) = previous_key
				&& let Some(references) = self.descriptor_references.get_mut(&previous_key)
			{
				if let Some(position) = references
					.iter()
					.position(|reference| *reference == (set_index as u32, element))
				{
					references.swap_remove(position);
				}
				if references.is_empty() {
					self.descriptor_references.remove(&previous_key);
				}
			}
			if let Some(key) = key {
				self.descriptor_references
					.entry(key)
					.or_default()
					.push((set_index as u32, element));
			}
		}
	}

	/// Marks every set element whose write names `resource` to write again into every copy before commands bind it,
	/// because the storage behind it, or the swapchain image it resolves to, changed. `resource` is the key
	/// [`Self::reference_key`] gives.
	pub(crate) fn invalidate_descriptors(&mut self, resource: super::super::Handles) {
		let Some(references) = self.descriptor_references.get(&resource) else {
			return;
		};
		for &(set_index, element) in references {
			for copy in &mut self.descriptor_sets[set_index as usize].copies {
				copy.dirty.push(element);
			}
		}
	}

	/// Readies the copy of the set `handle` that `key` names, which a recording binds as set `position` of
	/// `pipeline`, and returns its resource and sampler heap base indices for the push data.
	///
	/// The copy writes the elements that changed since commands last bound it. When work that may still run read it,
	/// the copy moves to new heap ranges, and its old ones return once that work completes. A recording outside frames
	/// reads the copy until it executes; then [`Self::settle_recording_reads`] gives its completion.
	///
	/// # Panics
	///
	/// Panics when a resource of the set that the pipeline uses, other than an array, was never written.
	pub(crate) fn bind_descriptor_set(
		&mut self,
		pipeline: graphics_hardware_interface::PipelineHandle,
		position: usize,
		handle: graphics_hardware_interface::DescriptorSetHandle,
		key: CopyKey,
	) -> [u32; 2] {
		let set_index = handle.index() as usize;
		let index = key.index();
		{
			let pipeline_set = &self.pipelines[pipeline.index() as usize].layout.sets[position];
			let set = &self.descriptor_sets[set_index];
			for (index, stages) in pipeline_set.stages.iter().enumerate() {
				let resource = set.layout.resources()[index];
				assert!(
					stages.is_empty()
						|| resource.count() > 1
						|| set.descriptors[set.layout.element_base(index) as usize].is_some(),
					"Missing Vulkan descriptor at slot {} of layout `{}`. The most likely cause is that a resource the pipeline reads was not written before the set was bound. Write every resource of the set that the pipeline's shaders read.",
					resource.slot().index(),
					set.layout.name(),
				);
			}
		}

		let previous = self.descriptor_sets[set_index].copies[index].reader;
		let reader = match key {
			CopyKey::Frame { frame, .. } => Reader::Frame(frame),
			CopyKey::Recording => Reader::OpenRecording,
		};
		if !self.descriptor_sets[set_index].copies[index].dirty.is_empty() {
			// A frame's earlier frames on its sequence completed before it started, so only the frame itself may read the
			// copy. A recording outside frames may still run until its synchronizer signals.
			let in_use = match previous {
				Some(Reader::Frame(frame)) => Some(Reader::Frame(frame)) == Some(reader),
				Some(Reader::OpenRecording) => true,
				Some(Reader::Recording(completion)) => !self.completion_reached(completion),
				None => false,
			};
			if in_use {
				self.move_set_copy(set_index, index, previous);
			}
			let mut dirty = std::mem::take(&mut self.descriptor_sets[set_index].copies[index].dirty);
			dirty.sort_unstable();
			dirty.dedup();
			self.write_set_elements(set_index, key, &dirty);
			dirty.clear();
			self.descriptor_sets[set_index].copies[index].dirty = dirty;
			self.rebuild_set_consumptions(set_index, key);
		}
		self.descriptor_sets[set_index].copies[index].reader = Some(reader);
		if key == CopyKey::Recording && previous != Some(Reader::OpenRecording) {
			self.open_recording_sets.push(set_index);
		}
		let copy = &self.descriptor_sets[set_index].copies[index];
		[copy.resource_base, copy.sampler_base]
	}

	/// Gives the reads of the recording outside frames that executes now their `completion`: the set copies it bound,
	/// and the heap ranges it read that moved, which return once it completes.
	pub(crate) fn settle_recording_reads(&mut self, completion: Completion) {
		let recording = CopyKey::Recording.index();
		for set_index in std::mem::take(&mut self.open_recording_sets) {
			let copy = &mut self.descriptor_sets[set_index].copies[recording];
			if copy.reader == Some(Reader::OpenRecording) {
				copy.reader = Some(Reader::Recording(completion));
			}
		}
		for retired in std::mem::take(&mut self.open_recording_retirements) {
			self.tasks.push(Task::after(retired, completion));
		}
	}

	/// Moves copy `index` of a set to new heap ranges, marking every written element to write there, and returns its
	/// old ranges once `reader`, the work that last read them, completes.
	fn move_set_copy(&mut self, set_index: usize, index: usize, reader: Option<Reader>) {
		let set = &mut self.descriptor_sets[set_index];
		let (element_count, sampler_count) = (set.descriptors.len() as u32, set.sampler_count);
		let heaps = self.descriptor_heaps.as_mut().expect(
			"Missing Vulkan descriptor heaps. The most likely cause is that a set was bound before the first set created the heaps.",
		);
		let copy = &mut set.copies[index];
		let retired = Tasks::FreeDescriptorRanges {
			resource_index: copy.resource_base,
			resource_count: element_count,
			sampler_index: copy.sampler_base,
			sampler_count,
		};
		copy.resource_base = heaps.resource.allocate(element_count);
		copy.sampler_base = heaps.sampler.allocate(sampler_count);
		copy.dirty.clear();
		let descriptors = &set.descriptors;
		copy.dirty
			.extend((0..element_count).filter(|&element| descriptors[element as usize].is_some()));
		match reader {
			// The recording that read the ranges hasn't executed, so it learns its completion when it does.
			Some(Reader::OpenRecording) => self.open_recording_retirements.push(retired),
			Some(Reader::Recording(completion)) => self.tasks.push(Task::after(retired, completion)),
			Some(Reader::Frame(_)) | None => self.defer_destruction(retired),
		}
	}

	/// Returns the heap ranges a moved set copy held. [`Self::run_destruction_task`] calls it once the frames that read
	/// them complete.
	pub(crate) fn free_descriptor_ranges(
		&mut self,
		resource_index: u32,
		resource_count: u32,
		sampler_index: u32,
		sampler_count: u32,
	) {
		if let Some(heaps) = &mut self.descriptor_heaps {
			heaps.resource.release(resource_index, resource_count);
			heaps.sampler.release(sampler_index, sampler_count);
		}
	}

	/// Writes `elements` of the copy `key` names into the heaps, each resolved to the resources of the copy's frame
	/// sequence. `elements` holds each element once.
	fn write_set_elements(&self, set_index: usize, key: CopyKey, elements: &[u32]) {
		let properties = self.device.descriptor_heap_properties;
		let set = &self.descriptor_sets[set_index];
		let copy = &set.copies[key.index()];
		let sequence_index = key.sequence();

		// Each list stays on the stack for the few elements a change usually rewrites.
		let mut address_writes = SmallVec::<[(vk::DescriptorType, vk::DeviceAddressRangeEXT, u32, u64); 8]>::new();
		let mut image_writes = SmallVec::<
			[(
				vk::DescriptorType,
				// `None` writes a null descriptor, for an image that has no memory yet.
				Option<vk::ImageViewCreateInfo<'static>>,
				vk::ImageLayout,
				u32,
				u64,
			); 8],
		>::new();
		let mut sampler_writes = SmallVec::<[(graphics_hardware_interface::SamplerHandle, u32); 8]>::new();
		for &element in elements {
			let Some(retained) = set.descriptors[element as usize] else {
				continue;
			};
			let (resource_index, array_element) = set.resource_of(element);
			let resource = set.layout.resources()[resource_index];
			let slot = copy.resource_base + element;
			let sampler_index = copy.sampler_base + set.sampler_bases[resource_index] + array_element;
			match self.resolve_retained_descriptor(retained, sequence_index) {
				Descriptor::Buffer { buffer } => {
					let buffer = self.buffers.resource(buffer);
					let size = buffer.size as u64;
					assert!(
						size > 0,
						"Invalid Vulkan buffer descriptor. The most likely cause is that an empty buffer was written to a shader resource. Create the buffer with a non-zero size."
					);
					address_writes.push((
						crate::vulkan::descriptor_type(resource.kind()).unwrap(),
						vk::DeviceAddressRangeEXT::default().address(buffer.device_address).size(size),
						slot,
						properties.buffer_descriptor_size,
					));
				}
				Descriptor::Image {
					image,
					layout: image_layout,
					mip_level,
				} => {
					let descriptor_type = crate::vulkan::descriptor_type(resource.kind()).unwrap();
					if descriptor_type == vk::DescriptorType::SAMPLED_IMAGE {
						self.assert_sampleable(image);
					}
					let image = &self.images[image.index() as usize];
					// Storage views address exactly one mip; a sampled image without a chosen mip exposes the whole chain.
					let (base_mip_level, level_count) = match mip_level {
						Some(mip_level) => (mip_level, 1),
						None if descriptor_type == vk::DescriptorType::SAMPLED_IMAGE => (0, image.mip_levels),
						None => (0, 1),
					};
					image_writes.push((
						descriptor_type,
						self.descriptor_image_view(image, resource.texture_view(), base_mip_level, level_count),
						texture_format_and_resource_use_to_image_layout(image.format_, image_layout, None),
						slot,
						properties.image_descriptor_size,
					));
				}
				Descriptor::CombinedImageSampler {
					image,
					sampler,
					layout: image_layout,
				} => {
					self.assert_sampleable(image);
					let image = &self.images[image.index() as usize];
					image_writes.push((
						vk::DescriptorType::SAMPLED_IMAGE,
						self.descriptor_image_view(image, resource.texture_view(), 0, image.mip_levels),
						texture_format_and_resource_use_to_image_layout(image.format_, image_layout, None),
						slot,
						properties.image_descriptor_size,
					));
					sampler_writes.push((sampler, sampler_index));
				}
				Descriptor::Sampler { sampler } => sampler_writes.push((sampler, sampler_index)),
				Descriptor::AccelerationStructure { handle, copy } => {
					let top_level = self.acceleration_structures.top_level(handle);
					let copy = top_level.copies.get(copy as usize).expect(
						"The top-level acceleration structure has no copy for this frame. The most likely cause is that the frames in flight grew after the structure was created. Create top-level structures after you set the frames in flight.",
					);
					// Descriptors name a structure by its address; the range's size only has to be valid.
					address_writes.push((
						vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
						vk::DeviceAddressRangeEXT::default()
							.address(copy.address)
							.size(copy.storage.size),
						slot,
						properties.buffer_descriptor_size,
					));
				}
			}
		}

		let heaps = self.descriptor_heaps.as_ref().unwrap();
		if !address_writes.is_empty() {
			let infos = address_writes
				.iter()
				.map(|(ty, range, ..)| {
					vk::ResourceDescriptorInfoEXT::default()
						.ty(*ty)
						.data(vk::ResourceDescriptorDataEXT { p_address_range: range })
				})
				.collect::<SmallVec<[_; 8]>>();
			let destinations = address_writes
				.iter()
				.map(|(_, _, slot, size)| heaps.resource.host_range(*slot, *size))
				.collect::<SmallVec<[_; 8]>>();
			// SAFETY: The infos point at ranges that live until the call returns, and no command that may still execute
			// reads the destinations: the copy was last bound by a completed frame, or moved to new ranges.
			unsafe {
				self.device
					.descriptor_heap
					.write_resource_descriptors(&infos, &destinations)
					.expect("Vulkan buffer descriptor write failed. The most likely cause is an invalid device address range.");
			}
		}
		if !image_writes.is_empty() {
			let images = image_writes
				.iter()
				.map(|(_, view, layout, ..)| {
					view.as_ref()
						.map(|view| vk::ImageDescriptorInfoEXT::default().view(view).layout(*layout))
				})
				.collect::<SmallVec<[_; 8]>>();
			let infos = image_writes
				.iter()
				.zip(&images)
				.map(|((ty, ..), image)| {
					// A null data pointer writes a null descriptor, which the `nullDescriptor` feature allows.
					let p_image = image.as_ref().map_or(std::ptr::null(), |image| image as *const _);
					vk::ResourceDescriptorInfoEXT::default()
						.ty(*ty)
						.data(vk::ResourceDescriptorDataEXT { p_image })
				})
				.collect::<SmallVec<[_; 8]>>();
			let destinations = image_writes
				.iter()
				.map(|(_, _, _, slot, size)| heaps.resource.host_range(*slot, *size))
				.collect::<SmallVec<[_; 8]>>();
			// SAFETY: As above, and the view descriptions live until the call returns.
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
				.map(|(handle, _)| {
					vk::SamplerReductionModeCreateInfo::default()
						.reduction_mode(self.samplers[handle.index() as usize].reduction_mode)
				})
				.collect::<SmallVec<[_; 8]>>();
			let samplers = sampler_writes
				.iter()
				.zip(&reductions)
				.map(|((handle, _), reduction)| {
					let mut sampler = self.samplers[handle.index() as usize].create_info();
					sampler.p_next = (reduction as *const vk::SamplerReductionModeCreateInfo).cast();
					sampler
				})
				.collect::<SmallVec<[_; 8]>>();
			let destinations = sampler_writes
				.iter()
				.map(|(_, index)| heaps.sampler.host_range(*index, properties.sampler_descriptor_size))
				.collect::<SmallVec<[_; 8]>>();
			// SAFETY: As above, and each reduction a create info points at lives until the call returns.
			unsafe {
				self.device
					.descriptor_heap
					.write_sampler_descriptors(&samplers, &destinations)
					.expect(
						"Vulkan sampler descriptor write failed. The most likely cause is an unsupported sampler description.",
					);
			}
		}
	}

	/// Lists the resources one copy of a set makes commands access, merged by resource and layout, each with the
	/// layout resources that name it.
	///
	/// A table of bindless textures can name thousands of distinct resources, so once the list holds more than a few,
	/// earlier entries are found through an index instead of a scan. The index keeps its memory on the context.
	fn rebuild_set_consumptions(&mut self, set_index: usize, key: CopyKey) {
		const SCANNED: usize = 16;
		let sequence_index = key.sequence();
		let mut consumptions = std::mem::take(&mut self.descriptor_sets[set_index].copies[key.index()].consumptions);
		consumptions.clear();
		let mut merged = std::mem::take(&mut self.descriptor_merge_index);
		merged.clear();

		let set = &self.descriptor_sets[set_index];
		for (element, retained) in (0u32..).zip(&set.descriptors) {
			let Some(retained) = retained else {
				continue;
			};
			let Some((handle, layout)) = Self::consumed_resource(self.resolve_retained_descriptor(*retained, sequence_index))
			else {
				continue;
			};
			let (resource_index, _) = set.resource_of(element);
			let resource = 1u64 << resource_index;
			if merged.is_empty() && consumptions.len() > SCANNED {
				merged.extend(
					consumptions
						.iter()
						.enumerate()
						.map(|(index, existing)| ((existing.handle, existing.layout), index)),
				);
			}
			let existing = if merged.is_empty() {
				consumptions
					.iter()
					.position(|existing| (existing.handle, existing.layout) == (handle, layout))
			} else {
				match merged.entry((handle, layout)) {
					std::collections::hash_map::Entry::Occupied(entry) => Some(*entry.get()),
					std::collections::hash_map::Entry::Vacant(entry) => {
						entry.insert(consumptions.len());
						None
					}
				}
			};
			match existing {
				Some(index) => consumptions[index].resources |= resource,
				None => consumptions.push(SetConsumption {
					handle,
					layout,
					resources: resource,
				}),
			}
		}

		self.descriptor_sets[set_index].copies[key.index()].consumptions = consumptions;
		self.descriptor_merge_index = merged;
	}

	/// Returns the resource a set's descriptor makes commands access, and the layout an image is accessed in.
	///
	/// Descriptors that return the same value merge into one consumption. A sampler returns `None`, because it touches
	/// no memory the resource tracker follows.
	fn consumed_resource(descriptor: Descriptor) -> Option<(super::super::Handles, crate::Layouts)> {
		use super::super::Handles;
		match descriptor {
			Descriptor::Buffer { buffer, .. } => Some((Handles::Buffer(buffer), crate::Layouts::General)),
			Descriptor::Image { image, layout, .. } | Descriptor::CombinedImageSampler { image, layout, .. } => {
				Some((Handles::Image(image), layout))
			}
			Descriptor::AccelerationStructure { handle, copy } => Some((
				Handles::AccelerationStructure(super::super::AccelerationStructureMemory::top_level(handle, copy)),
				crate::Layouts::General,
			)),
			Descriptor::Sampler { .. } => None,
		}
	}

	/// Resolves a retained write to the resources the copy of its set for frame sequence `sequence_index` names: that
	/// sequence's copy of a per-frame resource, moved by the write's frame offset, and a swapchain's acquired image.
	pub(crate) fn resolve_retained_descriptor(
		&self,
		retained: crate::vulkan::descriptor_set::RetainedDescriptor,
		sequence_index: u8,
	) -> Descriptor {
		let resource_sequence = frame_index_with_offset(sequence_index as usize, retained.frame_offset, self.frames as usize);
		let image = |handle| {
			self.resolve_descriptor_image_handle(
				graphics_hardware_interface::ImageHandle(handle),
				sequence_index as usize,
				retained.frame_offset,
			)
		};
		match retained.descriptor {
			WriteData::Buffer { handle } => Descriptor::Buffer {
				buffer: self.buffers.nth_handle(handle, resource_sequence).expect(
					"Missing Vulkan buffer copy. The most likely cause is that the buffer has fewer frame-local copies than the frames in flight. Create dynamic buffers after you set the frames in flight.",
				),
			},
			WriteData::Image {
				handle,
				layout,
				mip_level,
			} => Descriptor::Image {
				image: image(handle),
				layout,
				mip_level,
			},
			WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
			} => Descriptor::CombinedImageSampler {
				image: image(image_handle),
				sampler: sampler_handle,
				layout,
			},
			WriteData::Sampler(sampler) => Descriptor::Sampler { sampler },
			// Each frame traces the copy it builds.
			WriteData::AccelerationStructure { handle } => Descriptor::AccelerationStructure {
				handle,
				copy: resource_sequence as u8,
			},
			WriteData::Swapchain(handle) => {
				let swapchain = &self.swapchains[handle.index() as usize];
				Descriptor::Image {
					image: swapchain.images[swapchain.acquired_image_indices[resource_sequence] as usize],
					layout: crate::Layouts::General,
					mip_level: None,
				}
			}
		}
	}

	pub(crate) fn descriptor_image_view_create_info(
		&self,
		image: &Image,
		view_type: crate::TextureViewTypes,
		base_mip_level: u32,
		level_count: u32,
	) -> vk::ImageViewCreateInfo<'static> {
		assert!(
			base_mip_level < image.mip_levels,
			"Vulkan image descriptor mip level is out of range. The most likely cause is that the selected mip exceeds the image mip count. mip_level={base_mip_level}, mip_levels={}",
			image.mip_levels,
		);
		assert!(
			level_count > 0 && level_count <= image.mip_levels - base_mip_level,
			"Vulkan image descriptor mip range is invalid. The most likely cause is that the descriptor view exceeds the image mip count. base_mip_level={base_mip_level}, level_count={level_count}, mip_levels={}",
			image.mip_levels,
		);
		let array_layer_count = image.layers.map_or(1, NonZeroU32::get);
		let (vk_view_type, base_array_layer, layer_count) = match view_type {
			crate::TextureViewTypes::Texture2D => {
				assert!(
					image.layers.is_none() && image.extent.depth().max(1) == 1,
					"Vulkan 2D descriptor view mismatch. The most likely cause is that a layered or 3D image was written to a Texture2D shader resource."
				);
				(vk::ImageViewType::TYPE_2D, 0, 1)
			}
			crate::TextureViewTypes::Texture2DArray => {
				let layers = image.layers.map(NonZeroU32::get).expect(
					"Vulkan array descriptor view mismatch. The most likely cause is that a non-array image was written to a Texture2DArray shader resource.",
				);
				(vk::ImageViewType::TYPE_2D_ARRAY, 0, layers)
			}
			crate::TextureViewTypes::TextureCube => {
				assert!(
					image.cube_compatible && array_layer_count == 6,
					"Vulkan cubemap descriptor view mismatch. The most likely cause is that the image is not a six-layer cube-compatible image."
				);
				(vk::ImageViewType::CUBE, 0, 6)
			}
			crate::TextureViewTypes::TextureCubeArray => {
				assert!(
					image.cube_array_compatible && array_layer_count.is_multiple_of(6),
					"Vulkan cube-array descriptor view mismatch. The most likely cause is that the image is not a cube-array-compatible image."
				);
				(vk::ImageViewType::CUBE_ARRAY, 0, array_layer_count)
			}
			crate::TextureViewTypes::Texture3D => {
				assert!(
					image.layers.is_none() && image.extent.depth() > 1,
					"Vulkan 3D descriptor view mismatch. The most likely cause is that a 2D image was written to a Texture3D shader resource."
				);
				(vk::ImageViewType::TYPE_3D, 0, 1)
			}
		};
		vk::ImageViewCreateInfo::default()
			.image(image.image)
			.view_type(vk_view_type)
			.format(image.format)
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: if image.format_.is_depth() {
					vk::ImageAspectFlags::DEPTH
				} else {
					vk::ImageAspectFlags::COLOR
				},
				base_mip_level,
				level_count,
				base_array_layer,
				layer_count,
			})
	}

	#[inline]
	#[cfg_attr(not(debug_assertions), allow(unused_variables))]
	pub(crate) fn set_object_debug_name(&mut self, name: Option<&str>, handle: graphics_hardware_interface::Handles) {
		#[cfg(debug_assertions)]
		if let Some(name) = name {
			self.names.insert(handle, name.to_string());
		}
	}

	/// Asserts that a sampled descriptor names an image created for sampling, which Vulkan requires of the view.
	fn assert_sampleable(&self, handle: crate::vulkan::ImageHandle) {
		if self.images[handle.index() as usize].uses.contains(crate::Uses::Image) {
			return;
		}
		let base = graphics_hardware_interface::BaseImageHandle::from_index(handle.index());
		let name = self.get_object_debug_name(graphics_hardware_interface::ImageHandle(base).into());
		panic!(
			"Vulkan image `{}` is sampled but was not created for sampling. The most likely cause is that a shader samples an image whose builder lacks `Uses::Image`. Add `Uses::Image` to the image's uses.",
			name.unwrap_or_default(),
		);
	}

	/// Describes the view a descriptor of `image` reads, or `None` while the image has no native image: it gets one
	/// once it has an extent and, in an image group, a placement, and those rewrite the descriptors that name it.
	fn descriptor_image_view(
		&self,
		image: &Image,
		view_type: crate::TextureViewTypes,
		base_mip_level: u32,
		level_count: u32,
	) -> Option<vk::ImageViewCreateInfo<'static>> {
		(!image.image.is_null()).then(|| self.descriptor_image_view_create_info(image, view_type, base_mip_level, level_count))
	}

	#[inline]
	#[cfg_attr(not(debug_assertions), allow(unused_variables))]
	pub(crate) fn get_object_debug_name(&self, handle: graphics_hardware_interface::Handles) -> Option<String> {
		#[cfg(debug_assertions)]
		let name = self.names.get(&handle).cloned();
		#[cfg(not(debug_assertions))]
		let name = None;
		name
	}
}
