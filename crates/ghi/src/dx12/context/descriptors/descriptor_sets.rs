use super::super::*;

impl Device {
	/// Adds one frame copy of a set holding `descriptors`, leases its heap ranges, and fills them with null
	/// descriptors, so shaders can read elements the application never writes. Every written element is stale.
	pub(crate) fn push_descriptor_set_copy(
		&mut self,
		layout: &crate::shader::DescriptorSetLayout,
		sequence_index: u8,
		descriptors: Box<[Option<RetainedDescriptor>]>,
	) -> DescriptorSetHandle {
		let (resource_base, sampler_base) = self.lease_set_ranges(layout);
		let stale = descriptors
			.iter()
			.enumerate()
			.filter(|(_, descriptor)| descriptor.is_some())
			.map(|(element, _)| element as u32)
			.collect();
		let handle = DescriptorSetHandle::from_index(self.descriptor_sets.len() as u64);
		self.descriptor_sets.push(DescriptorSet {
			next: None,
			sequence_index,
			reader: None,
			layout: layout.clone(),
			resource_base,
			sampler_base,
			descriptors,
			stale,
		});
		handle
	}

	/// Leases the heap ranges of one set copy of `layout` and fills them with null descriptors and default samplers,
	/// so shaders can read elements the application never writes. Returns the resource and sampler bases.
	fn lease_set_ranges(&mut self, layout: &crate::shader::DescriptorSetLayout) -> (u32, u32) {
		let resource_base = self.lease_shader_visible_range(false, layout.element_count());
		let sampler_base = self.lease_shader_visible_range(true, Self::layout_sampler_count(layout));

		let mut sampler_offset = 0;
		for (resource_index, &resource) in layout.resources().iter().enumerate() {
			let element_base = layout.element_base(resource_index);
			for array_element in 0..resource.count() {
				let cpu_handle = self.shader_visible_cpu_handle(false, resource_base + element_base + array_element);
				self.write_null_cbv_srv_uav_descriptor(resource, cpu_handle);
			}
			if Self::holds_sampler(resource) {
				for array_element in 0..resource.count() {
					let cpu_handle = self.shader_visible_cpu_handle(true, sampler_base + sampler_offset + array_element);
					self.write_default_sampler_descriptor(cpu_handle);
				}
				sampler_offset += resource.count();
			}
		}
		(resource_base, sampler_base)
	}

	/// Readies a set copy for a bind by `reader`, then writes its stale descriptors.
	///
	/// When the copy changed since a recording that may still run read it, the copy moves to new heap ranges and writes
	/// every element there, so that recording keeps reading the descriptors it was recorded with. The old ranges go
	/// back to the heaps when that recording's command buffer frame starts its next recording, which waits for the
	/// submission to complete.
	pub(crate) fn prepare_set_for_bind(&mut self, set_index: usize, reader: SetReader) {
		let set = &self.descriptor_sets[set_index];
		let in_use_by = set.reader.filter(|previous| {
			self.command_buffers
				.get(previous.command_buffer.index() as usize)
				.and_then(|command_buffer| command_buffer.frames.get(previous.frame as usize))
				.is_some_and(|frame| frame.generation == previous.generation)
		});
		if let (false, Some(previous)) = (set.stale.is_empty(), in_use_by) {
			let layout = set.layout.clone();
			let (old_resources, old_samplers) = (set.resource_base, set.sampler_base);
			let (resource_count, sampler_count) = (layout.element_count(), Self::layout_sampler_count(&layout));
			let (resource_base, sampler_base) = self.lease_set_ranges(&layout);

			let frame = &mut self.command_buffers[previous.command_buffer.index() as usize].frames[previous.frame as usize];
			for (sampler_heap, start, count) in [(false, old_resources, resource_count), (true, old_samplers, sampler_count)] {
				if count != 0 {
					frame
						.retired_set_descriptors
						.push((sampler_heap, start as usize..(start + count) as usize));
				}
			}

			let set = &mut self.descriptor_sets[set_index];
			set.resource_base = resource_base;
			set.sampler_base = sampler_base;
			set.stale.clear();
			let written = set
				.descriptors
				.iter()
				.enumerate()
				.filter(|(_, descriptor)| descriptor.is_some())
				.map(|(element, _)| element as u32);
			set.stale.extend(written);
		}
		self.descriptor_sets[set_index].reader = Some(reader);
		self.write_stale_descriptors(set_index);
	}

	/// Returns whether a resource takes application-bound samplers from its set's sampler range, one per element.
	/// A sampled image whose shader fixes its sampler reads a fixed sampler instead.
	pub(crate) fn holds_sampler(resource: ShaderResourceDescriptor) -> bool {
		matches!(resource.kind(), ResourceKind::CombinedImageSampler | ResourceKind::Sampler)
	}

	/// Returns how many application-bound samplers a set of `layout` holds.
	pub(crate) fn layout_sampler_count(layout: &crate::shader::DescriptorSetLayout) -> u32 {
		layout
			.resources()
			.iter()
			.filter(|resource| Self::holds_sampler(**resource))
			.map(|resource| resource.count())
			.sum()
	}

	/// Returns how many application-bound samplers the resources before `resource_index` hold, the offset of its
	/// first sampler in the set's sampler range.
	fn sampler_offset(layout: &crate::shader::DescriptorSetLayout, resource_index: usize) -> u32 {
		layout.resources()[..resource_index]
			.iter()
			.filter(|resource| Self::holds_sampler(**resource))
			.map(|resource| resource.count())
			.sum()
	}

	/// Returns the position in `layout.resources()` of the resource that holds descriptor `element`.
	fn element_resource(layout: &crate::shader::DescriptorSetLayout, element: u32) -> usize {
		// The resource of an element is the last one whose first element is at or before it.
		let (mut low, mut high) = (0, layout.resources().len());
		while high - low > 1 {
			let middle = (low + high) / 2;
			if layout.element_base(middle) <= element {
				low = middle;
			} else {
				high = middle;
			}
		}
		low
	}

	/// Extends every descriptor set chain to cover a larger frame topology.
	///
	/// Call this only after all queues are idle. New frame copies inherit the latest retained writes so the first bind
	/// after a topology change observes the same logical set on every frame.
	pub(crate) fn resize_descriptor_set_chains(&mut self, frames: u8) {
		let original_set_count = self.descriptor_sets.len();
		for root_index in 0..original_set_count {
			if self.descriptor_sets[root_index].sequence_index != 0 {
				continue;
			}

			let mut tail = DescriptorSetHandle::from_index(root_index as u64);
			let mut chain_length = 1u8;
			while let Some(next) = self.descriptor_sets[tail.index() as usize].next {
				tail = DescriptorSetHandle::from_index(next.index());
				chain_length += 1;
			}

			while chain_length < frames {
				let tail_set = &self.descriptor_sets[tail.index() as usize];
				let (layout, descriptors) = (tail_set.layout.clone(), tail_set.descriptors.clone());
				let next = self.push_descriptor_set_copy(&layout, chain_length, descriptors);
				self.descriptor_sets[tail.index() as usize].next =
					Some(crate::descriptors::DescriptorSetHandle::from_index(next.index()));
				tail = next;
				chain_length += 1;
			}
		}
	}

	/// Queues for rewriting every retained descriptor for which `references` holds, in every frame copy of every set.
	/// `references` gets the descriptor and the frame sequence whose resource it reads in that copy.
	///
	/// Call it when what descriptors name changes under them, such as a resized image or a recreated swapchain proxy.
	/// Each copy rewrites them at its next bind, which records for the copy's own frame sequence.
	pub(crate) fn mark_stale_descriptors(&mut self, references: impl Fn(WriteData, u8) -> bool) {
		let frames = self.frames as usize;
		for set in &mut self.descriptor_sets {
			for (element, descriptor) in set.descriptors.iter().enumerate() {
				let stale = descriptor.is_some_and(|retained| {
					let sequence = crate::frame_resources::frame_index_with_offset(
						set.sequence_index as usize,
						retained.frame_offset,
						frames,
					);
					references(retained.descriptor, sequence as u8)
				});
				if stale {
					set.stale.push(element as u32);
				}
			}
			Self::bound_stale_descriptors(set);
		}
	}

	/// Drops repeated stale elements once a copy that doesn't bind queues more of them than it holds, so the list
	/// stays within twice the copy's size.
	pub(crate) fn bound_stale_descriptors(set: &mut DescriptorSet) {
		if set.stale.len() > set.descriptors.len() {
			set.stale.sort_unstable();
			set.stale.dedup();
		}
	}

	/// Returns whether a retained descriptor names `image`.
	pub(crate) fn references_image(descriptor: WriteData, image: crate::BaseImageHandle) -> bool {
		matches!(
			descriptor,
			WriteData::Image { handle, .. } | WriteData::CombinedImageSampler { image_handle: handle, .. } if handle == image
		)
	}

	/// Writes the native descriptors of a set copy's stale elements into its heap ranges.
	pub(crate) fn write_stale_descriptors(&mut self, set_index: usize) {
		if self.descriptor_sets[set_index].stale.is_empty() {
			return;
		}
		let mut stale = std::mem::take(&mut self.descriptor_sets[set_index].stale);
		for &element in &stale {
			self.write_set_element(set_index, element);
		}
		stale.clear();
		// Resolving a resource can create a frame copy of it, which queues its descriptors again; those wait for the
		// next bind, and the list keeps its capacity.
		let set = &mut self.descriptor_sets[set_index];
		stale.append(&mut set.stale);
		set.stale = stale;
	}

	/// Writes the native descriptor, and sampler, of one element of a set copy from its retained write.
	fn write_set_element(&mut self, set_index: usize, element: u32) {
		let set = &self.descriptor_sets[set_index];
		let Some(retained) = set.descriptors[element as usize] else {
			return;
		};
		let resource_index = Self::element_resource(&set.layout, element);
		let resource = set.layout.resources()[resource_index];
		let array_element = element - set.layout.element_base(resource_index);
		let slot = set.resource_base + element;
		let sampler_slot = Self::holds_sampler(resource)
			.then(|| set.sampler_base + Self::sampler_offset(&set.layout, resource_index) + array_element);
		let resource_sequence = self.frame_index_with_offset(set.sequence_index, retained.frame_offset);

		match retained.descriptor {
			WriteData::Buffer { handle } => {
				self.descriptor_sets[set_index].stale.retain(|&stale| stale != element);
				self.write_native_buffer_descriptor(resource, handle, resource_sequence, slot)
			}
			WriteData::Image {
				handle,
				mip_level,
				array_layer,
				..
			} => self.write_native_image_descriptor(resource, handle, resource_sequence, array_layer, mip_level, slot),
			WriteData::CombinedImageSampler { image_handle, .. } => {
				self.write_native_image_descriptor(resource, image_handle, resource_sequence, None, None, slot)
			}
			WriteData::Swapchain(handle) => {
				let uses = Self::descriptor_image_use(resource);
				let image = self.get_swapchain_image(handle, uses).0;
				self.write_native_image_descriptor(resource, image.into(), resource_sequence, None, None, slot);
			}
			WriteData::AccelerationStructure { handle } => {
				self.write_native_acceleration_structure_descriptor(handle, resource_sequence, slot)
			}
			WriteData::Sampler(_) => {}
		}
		if let (
			Some(sampler_slot),
			WriteData::CombinedImageSampler { sampler_handle, .. } | WriteData::Sampler(sampler_handle),
		) = (sampler_slot, retained.descriptor)
		{
			self.write_native_sampler_descriptor(sampler_handle, sampler_slot);
		}
	}
}
