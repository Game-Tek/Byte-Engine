use super::super::*;

impl Context {
	/// Creates one retained logical descriptor set per in-flight frame without allocating a native layout.
	pub fn create_descriptor_set(&mut self, _name: Option<&str>) -> graphics_hardware_interface::DescriptorSetHandle {
		let handle = graphics_hardware_interface::DescriptorSetHandle(self.descriptor_sets.len() as u64);
		let mut previous_handle: Option<DescriptorSetHandle> = None;

		for _ in 0..self.frames {
			let descriptor_set_handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);
			self.descriptor_sets.push(descriptor_set::DescriptorSet {
				next: None,
				version: 0,
				descriptors: HashMap::default(),
			});

			if let Some(previous_handle) = previous_handle {
				self.descriptor_sets[previous_handle.0 as usize].next = Some(descriptor_set_handle);
			}

			previous_handle = Some(descriptor_set_handle);
		}

		handle
	}

	/// Interns a factory-built sampler into this device and returns its public sampler handle.
	pub fn intern_sampler(&mut self, sampler: crate::metal::device::Sampler) -> graphics_hardware_interface::SamplerHandle {
		self.samplers.push(sampler.sampler);
		graphics_hardware_interface::SamplerHandle((self.samplers.len() - 1) as u64)
	}

	pub fn build_sampler(&mut self, builder: sampler_builder::Builder) -> graphics_hardware_interface::SamplerHandle {
		let descriptor = build_sampler_descriptor(&builder);
		apply_sampler_reduction_fallback(self.device.as_ref(), &descriptor);

		let sampler_state = self
			.device
			.newSamplerStateWithDescriptor(&descriptor)
			.expect("Metal sampler creation failed. The most likely cause is that the device is out of sampler resources.");
		self.samplers.push(super::super::sampler::Sampler { sampler: sampler_state });
		graphics_hardware_interface::SamplerHandle((self.samplers.len() - 1) as u64)
	}

	/// Applies retained descriptor writes to every frame-local logical set they target.
	pub fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		for write in descriptor_set_writes {
			self.apply_descriptor_write_to_all_frames(
				DescriptorSetHandle(write.descriptor_set.0),
				write.slot,
				write.descriptor,
				write.array_element,
				write.frame_offset.unwrap_or(0),
			);
		}
	}
}
