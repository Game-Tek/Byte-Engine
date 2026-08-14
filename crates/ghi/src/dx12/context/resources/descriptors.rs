use super::super::*;

impl Device {
	/// Returns whether any retained native materialization contains this logical set.
	pub(crate) fn descriptor_set_has_native_heaps(&self, descriptor_set: DescriptorSetHandle) -> Option<(bool, bool)> {
		self.descriptor_sets.get(descriptor_set.0 as usize)?;
		let frame_sets = self.collect_descriptor_set_handles(descriptor_set);
		let mut cbv_srv_uav = false;
		let mut sampler = false;
		for (key, materialization) in &self.descriptor_materializations {
			if !key.descriptor_sets.iter().any(|set| frame_sets.contains(set)) {
				continue;
			}
			cbv_srv_uav |= materialization.cbv_srv_uav_heap.is_some();
			sampler |= materialization.sampler_heap.is_some();
		}
		Some((cbv_srv_uav, sampler))
	}

	/// Returns the number of cached frame-resolved native descriptor snapshots.
	#[cfg(test)]
	pub(crate) fn descriptor_materialization_count(&self) -> usize {
		self.descriptor_materializations.len()
	}

	#[cfg(test)]
	pub(crate) fn pipeline_descriptor_counts(&self, pipeline: PipelineHandle) -> Option<(u32, u32)> {
		let pipeline = self.pipelines.get(pipeline.0 as usize)?;
		let layout = self.pipeline_layouts.get(pipeline.layout.0 as usize)?;
		Some((layout.cbv_srv_uav_descriptor_count, layout.sampler_descriptor_count))
	}

	#[cfg(test)]
	pub(crate) fn pipeline_descriptor_slot(
		&self,
		pipeline: PipelineHandle,
		slot: ResourceSlot,
		array_element: u32,
		sampler_heap: bool,
	) -> Option<u32> {
		let pipeline = self.pipelines.get(pipeline.0 as usize)?;
		let layout = self.pipeline_layouts.get(pipeline.layout.0 as usize)?;
		let resource = layout.resources.iter().find(|resource| resource.descriptor.slot() == slot)?;
		if array_element >= resource.descriptor.count() {
			return None;
		}
		let offset = if sampler_heap {
			resource.sampler_offset
		} else {
			resource.cbv_srv_uav_offset
		}?;
		Some(offset + array_element)
	}

	#[cfg(test)]
	pub(crate) fn pipeline_resource_descriptor(
		&self,
		pipeline: PipelineHandle,
		slot: ResourceSlot,
	) -> Option<ShaderResourceDescriptor> {
		let pipeline = self.pipelines.get(pipeline.0 as usize)?;
		self.pipeline_layouts[pipeline.layout.0 as usize]
			.resources
			.iter()
			.find(|resource| resource.descriptor.slot() == slot)
			.map(|resource| resource.descriptor)
	}

	pub(crate) fn pipeline_layout_has_root_signature(&self, pipeline_layout: PipelineLayoutHandle) -> Option<bool> {
		self.pipeline_root_signatures
			.get(pipeline_layout.0 as usize)
			.map(|root_signature| root_signature.is_some())
	}

	pub(crate) fn root_signature_bind_count(&self) -> usize {
		self.root_signature_bind_count
	}

	pub(crate) fn descriptor_heap_bind_count(&self) -> usize {
		self.descriptor_heap_bind_count
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
			.get(command_buffer.0 as usize)
			.map(|command_buffer| command_buffer.pending_clear_descriptor_copies.len())
			.unwrap_or(0)
	}

	#[cfg(test)]
	pub(crate) fn clear_descriptor_copy_call_count(&self) -> usize {
		self.clear_descriptor_copy_call_count
	}

	pub(crate) fn descriptor_table_bind_count(&self) -> usize {
		self.descriptor_table_bind_count
	}

	#[cfg(test)]
	pub(crate) fn descriptor_table_bind_records(&self) -> &[DescriptorTableBindRecord] {
		&self.descriptor_table_bind_records
	}

	pub(crate) fn push_constant_write_count(&self) -> usize {
		self.push_constant_write_count
	}

	#[cfg(test)]
	pub(crate) fn push_constant_write_records(&self) -> &[PushConstantWriteRecord] {
		&self.push_constant_write_records
	}

	pub(crate) fn descriptor_write_count(&self) -> usize {
		self.descriptor_write_count
	}

	pub(crate) fn image_srv_descriptor_write_count(&self) -> usize {
		self.image_srv_descriptor_write_count
	}

	pub(crate) fn image_uav_descriptor_write_count(&self) -> usize {
		self.image_uav_descriptor_write_count
	}

	pub(crate) fn acceleration_structure_descriptor_write_count(&self) -> usize {
		self.acceleration_structure_descriptor_write_count
	}

	#[cfg(test)]
	pub(crate) fn sampler_descriptor_write_records(&self) -> &[SamplerDescriptorWriteRecord] {
		&self.sampler_descriptor_write_records
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
		SamplerHandle((self.samplers.len() - 1) as u64)
	}

	/// Applies retained flat-slot descriptor writes to every frame-local set.
	pub fn write(&mut self, descriptor_set_writes: &[DescriptorWrite]) {
		for write in descriptor_set_writes {
			let set_handles = self.collect_descriptor_set_handles(DescriptorSetHandle(write.descriptor_set.0));
			for set_handle in set_handles {
				let retained = RetainedDescriptor {
					descriptor: write.descriptor,
					frame_offset: write.frame_offset.unwrap_or(0),
				};
				let descriptor_set = &mut self.descriptor_sets[set_handle.0 as usize];
				let previous = descriptor_set
					.descriptors
					.entry(write.slot)
					.or_default()
					.insert(write.array_element, retained);
				if previous != Some(retained) {
					descriptor_set.version = descriptor_set.version.wrapping_add(1);
				}
				self.materialize_descriptor_base_image_resource(set_handle, write.descriptor);
			}
		}
	}
}
