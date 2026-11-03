use super::super::*;

impl Device {
	/// Returns the heap indices of a set copy's first descriptor and first application-bound sampler.
	#[cfg(test)]
	pub(crate) fn descriptor_set_bases(&self, descriptor_set: DescriptorSetHandle) -> Option<(u32, u32)> {
		let set = self.descriptor_sets.get(descriptor_set.index() as usize)?;
		Some((set.resource_base, set.sampler_base))
	}

	/// Returns how many elements of a set copy wait for their native descriptor.
	#[cfg(test)]
	pub(crate) fn stale_descriptor_count(&self, descriptor_set: DescriptorSetHandle) -> Option<usize> {
		Some(self.descriptor_sets.get(descriptor_set.index() as usize)?.stale.len())
	}

	/// Returns how many set positions each stage of a pipeline takes bases for, in root parameter order.
	#[cfg(test)]
	pub(crate) fn pipeline_stage_set_counts(&self, pipeline: PipelineHandle) -> Option<SmallVec<[usize; 3]>> {
		let pipeline = self.pipelines.get(pipeline.index() as usize)?;
		let layout = self.pipeline_layouts.get(pipeline.layout.index() as usize)?;
		Some(layout.stages.iter().map(|stage| stage.sets.len()).collect())
	}

	pub(crate) fn root_signature_bind_count(&self) -> usize {
		self.native_counters.root_signature_bind_count
	}

	pub(crate) fn descriptor_heap_bind_count(&self) -> usize {
		self.native_counters.descriptor_heap_bind_count
	}

	/// Returns retained descriptor, page, used-slot, and free-slot counts for clear UAV descriptors.
	#[cfg(test)]
	pub(crate) fn retained_clear_uav_descriptor_pool_state(&self) -> (usize, usize, u32, usize) {
		(
			self.retained_clear_uav_descriptors.len(),
			self.clear_uav_descriptor_pages.len(),
			self.clear_uav_descriptor_pages.iter().map(|page| page.used).sum(),
			self.free_clear_uav_descriptor_slots.len(),
		)
	}

	#[cfg(test)]
	pub(crate) fn pending_clear_descriptor_copy_count(&self, command_buffer: CommandBufferHandle) -> usize {
		self.command_buffers
			.get(command_buffer.index() as usize)
			.map(|command_buffer| command_buffer.pending_clear_descriptor_copies.len())
			.unwrap_or(0)
	}

	#[cfg(test)]
	pub(crate) fn clear_descriptor_copy_call_count(&self) -> usize {
		self.native_counters.clear_descriptor_copy_call_count
	}

	pub(crate) fn set_base_write_count(&self) -> usize {
		self.native_counters.set_base_write_count
	}

	#[cfg(test)]
	pub(crate) fn set_base_write_records(&self) -> &[SetBaseWriteRecord] {
		&self.native_counters.set_base_write_records
	}

	pub(crate) fn push_constant_write_count(&self) -> usize {
		self.native_counters.push_constant_write_count
	}

	#[cfg(test)]
	pub(crate) fn push_constant_write_records(&self) -> &[PushConstantWriteRecord] {
		&self.native_counters.push_constant_write_records
	}

	pub(crate) fn descriptor_write_count(&self) -> usize {
		self.native_counters.descriptor_write_count
	}

	pub(crate) fn image_srv_descriptor_write_count(&self) -> usize {
		self.native_counters.image_srv_descriptor_write_count
	}

	pub(crate) fn image_uav_descriptor_write_count(&self) -> usize {
		self.native_counters.image_uav_descriptor_write_count
	}

	#[cfg(test)]
	pub(crate) fn sampler_descriptor_write_records(&self) -> &[SamplerDescriptorWriteRecord] {
		&self.native_counters.sampler_descriptor_write_records
	}

	pub fn build_sampler(&mut self, builder: sampler::Builder) -> SamplerHandle {
		// Stores sampler parameters without creating a DX12 descriptor.
		self.samplers.push(Sampler {
			filtering_mode: builder.filtering_mode,
			reduction_mode: builder.reduction_mode,
			mip_map_mode: builder.mip_map_mode,
			addressing_mode: builder.addressing_mode,
			anisotropy: builder.anisotropy,
			min_lod: builder.min_lod,
			max_lod: builder.max_lod,
		});
		SamplerHandle::from_index((self.samplers.len() - 1) as u64)
	}

	/// Applies retained descriptor writes to every frame copy of their sets.
	///
	/// Each copy keeps the write and queues its element; the native descriptor follows at the copy's next bind, which
	/// records for its own frame sequence, so a write never changes descriptors that queued work still reads.
	///
	/// # Panics
	///
	/// Panics when a write names a slot its set's layout doesn't declare, an array element past the resource's count,
	/// or a resource of another kind.
	pub fn write(&mut self, descriptor_set_writes: &[DescriptorWrite]) {
		for write in descriptor_set_writes {
			let mut current = Some(DescriptorSetHandle::from_index(write.descriptor_set.index()));
			while let Some(set_handle) = current {
				let descriptor_set = &mut self.descriptor_sets[set_handle.index() as usize];
				let layout = &descriptor_set.layout;
				let resource_index = layout.resource_index(write.slot).unwrap_or_else(|| {
					panic!(
						"Invalid descriptor write at slot {} of layout `{}`. The most likely cause is that the write names a binding of another set. Write the slot the set's layout declares.",
						write.slot.index(),
						layout.name(),
					)
				});
				let resource = layout.resources()[resource_index];
				assert!(
					write.array_element < resource.count(),
					"Descriptor array element {} is out of range at slot {} of layout `{}`. The most likely cause is that a write exceeded the resource's count of {}. Write an element below the count.",
					write.array_element,
					write.slot.index(),
					layout.name(),
					resource.count(),
				);
				assert!(
					write.descriptor.matches_kind(resource.kind()),
					"Descriptor kind mismatch at slot {} of layout `{}`. The most likely cause is that the write doesn't match the resource kind {:?} the layout declares. Write a resource of that kind.",
					write.slot.index(),
					layout.name(),
					resource.kind(),
				);
				let element = layout.element_base(resource_index) + write.array_element;
				let retained = RetainedDescriptor {
					descriptor: write.descriptor,
					frame_offset: write.frame_offset.unwrap_or(0),
				};
				let next = descriptor_set
					.next
					.map(|handle| DescriptorSetHandle::from_index(handle.index()));
				if descriptor_set.descriptors[element as usize] != Some(retained) {
					descriptor_set.descriptors[element as usize] = Some(retained);
					descriptor_set.version = descriptor_set.version.wrapping_add(1);
					descriptor_set.stale.push(element);
					Self::bound_stale_descriptors(descriptor_set);
				}
				self.materialize_descriptor_base_image_resource(set_handle, write.descriptor);
				current = next;
			}
		}
	}
}
