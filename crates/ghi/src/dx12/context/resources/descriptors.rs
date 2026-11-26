use super::super::*;

impl Device {
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

	/// Returns how many copies the resource a descriptor names has, which bounds the frame offsets a write may use.
	fn descriptor_resource_copies(&self, descriptor: WriteData) -> usize {
		match descriptor {
			WriteData::Buffer { handle } => self.buffer(handle).copies.count(self.frames) as usize,
			WriteData::Image { handle, .. } => self.image_copy_count(handle),
			WriteData::CombinedImageSampler { image_handle, .. } => self.image_copy_count(image_handle),
			_ => 1,
		}
	}

	/// Applies retained descriptor writes to every frame copy of their sets.
	///
	/// Each copy keeps the write and queues its element; the native descriptor follows at the copy's next bind. Work
	/// recorded before that bind keeps reading the old descriptors: when a recording that may still run read the copy,
	/// the bind moves the copy to new heap ranges instead of overwriting them.
	///
	/// # Panics
	///
	/// Panics when a write doesn't fit its set's layout (see [`crate::descriptors::DescriptorWrite::target`]), or when
	/// a frame offset's magnitude reaches the resource's copy count.
	pub fn write(&mut self, descriptor_set_writes: &[DescriptorWrite]) {
		for write in descriptor_set_writes {
			if let Some(frame_offset) = write.frame_offset {
				crate::descriptors::assert_frame_offset(self.descriptor_resource_copies(write.descriptor), frame_offset);
			}
			let (_, element) = write.target(&self.descriptor_sets[write.descriptor_set.index() as usize].layout);
			let mut current = Some(DescriptorSetHandle::from_index(write.descriptor_set.index()));
			while let Some(set_handle) = current {
				let descriptor_set = &mut self.descriptor_sets[set_handle.index() as usize];
				let retained = RetainedDescriptor {
					descriptor: write.descriptor,
					frame_offset: write.frame_offset.unwrap_or(0),
				};
				let next = descriptor_set
					.next
					.map(|handle| DescriptorSetHandle::from_index(handle.index()));
				if descriptor_set.descriptors[element as usize] != Some(retained) {
					descriptor_set.descriptors[element as usize] = Some(retained);
					descriptor_set.stale.push(element);
					Self::bound_stale_descriptors(descriptor_set);
				}
				self.materialize_descriptor_base_image_resource(set_handle, write.descriptor);
				current = next;
			}
		}
	}
}
