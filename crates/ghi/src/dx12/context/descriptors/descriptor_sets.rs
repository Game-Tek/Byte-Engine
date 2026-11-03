use super::super::*;

impl Device {
	/// Creates a descriptor set from `layout`, with one copy for each frame in flight.
	///
	/// Each copy leases its ranges of the shader-visible heaps now and holds them for its lifetime, so binding it
	/// only writes two root constants. Next, fill it with [`Self::write`] and bind it with a pipeline whose shaders
	/// declare `layout`.
	pub fn create_descriptor_set(
		&mut self,
		_name: Option<&str>,
		layout: &crate::shader::DescriptorSetLayout,
	) -> DescriptorSetHandle {
		let handle = DescriptorSetHandle::from_index(self.descriptor_sets.len() as u64);
		let mut previous: Option<DescriptorSetHandle> = None;

		for sequence_index in 0..self.frames {
			let descriptors = vec![None; layout.element_count() as usize].into_boxed_slice();
			let frame_handle = self.push_descriptor_set_copy(layout, sequence_index, descriptors);
			if let Some(previous) = previous {
				self.descriptor_sets[previous.index() as usize].next =
					Some(crate::descriptors::DescriptorSetHandle::from_index(frame_handle.index()));
			}
			previous = Some(frame_handle);
		}

		handle
	}

	/// Adds one frame copy of a set holding `descriptors`, leases its heap ranges, and fills them with null
	/// descriptors, so shaders can read elements the application never writes. Every written element is stale.
	fn push_descriptor_set_copy(
		&mut self,
		layout: &crate::shader::DescriptorSetLayout,
		sequence_index: u8,
		descriptors: Box<[Option<RetainedDescriptor>]>,
	) -> DescriptorSetHandle {
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
			version: 0,
			layout: layout.clone(),
			resource_base,
			sampler_base,
			descriptors,
			stale,
		});
		handle
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
				let next = DescriptorSetHandle::from_index(next.index());
				assert!(
					next.index() < original_set_count as u64,
					"Invalid DX12 descriptor-set chain. The most likely cause is that a frame-local next handle points outside retained descriptor storage. Create descriptor sets only through the device."
				);
				tail = next;
				chain_length = chain_length.checked_add(1).expect(
					"Invalid DX12 descriptor-set chain. The most likely cause is that the frame-local chain contains a cycle. Create descriptor sets only through the device.",
				);
				assert!(
					chain_length <= crate::MAX_FRAMES_IN_FLIGHT as u8,
					"Invalid DX12 descriptor-set chain. The most likely cause is that the frame-local chain contains a cycle or exceeds the engine frame limit. Create descriptor sets only through the device."
				);
			}

			while chain_length < frames {
				let tail_set = &self.descriptor_sets[tail.index() as usize];
				let (layout, version, descriptors) = (tail_set.layout.clone(), tail_set.version, tail_set.descriptors.clone());
				let next = self.push_descriptor_set_copy(&layout, chain_length, descriptors);
				self.descriptor_sets[next.index() as usize].version = version;
				self.descriptor_sets[tail.index() as usize].next =
					Some(crate::descriptors::DescriptorSetHandle::from_index(next.index()));
				tail = next;
				chain_length += 1;
			}
		}
	}

	/// Queues for rewriting every retained descriptor for which `references` holds, in every frame copy of every set.
	///
	/// Call it when what descriptors name changes under them, such as a resized image or a recreated swapchain proxy.
	/// Each copy rewrites them at its next bind, which records for the copy's own frame sequence.
	pub(crate) fn mark_stale_descriptors(&mut self, references: impl Fn(WriteData) -> bool) {
		for set in &mut self.descriptor_sets {
			for (element, descriptor) in set.descriptors.iter().enumerate() {
				if descriptor.is_some_and(|retained| references(retained.descriptor)) {
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
			WriteData::Buffer { handle } => self.write_native_buffer_descriptor(resource, handle, resource_sequence, slot),
			WriteData::Image { handle, mip_level, .. } => {
				self.write_native_image_descriptor(resource, handle, resource_sequence, None, mip_level, slot)
			}
			WriteData::CombinedImageSampler { image_handle, .. } => {
				self.write_native_image_descriptor(resource, image_handle, resource_sequence, None, None, slot)
			}
			WriteData::Swapchain(handle) => {
				let uses = Self::descriptor_image_use(resource);
				let image = self.get_swapchain_image_for_sequence(handle, uses, resource_sequence).0;
				self.write_native_image_descriptor(resource, image.into(), resource_sequence, None, None, slot);
			}
			WriteData::AccelerationStructure { .. } => crate::rt::unsupported("DX12"),
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

	/// Drops attachment views whose native resources were replaced.
	pub(crate) fn invalidate_attachment_views_for_resources(&mut self, resources: &[usize]) {
		if resources.is_empty() {
			return;
		}
		self.render_target_views.retain(|key, _| !resources.contains(&key.resource));
		self.depth_stencil_views.retain(|key, _| !resources.contains(&key.resource));
	}
}
