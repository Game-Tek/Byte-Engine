use super::*;

impl Device {
	pub(crate) fn dynamic_buffer_slice_mut<T: Copy>(
		&mut self,
		buffer_handle: DynamicBufferHandle<T>,
		sequence_index: u8,
	) -> &mut T {
		let handle = buffer_handle.into();
		let Some((data, _)) = self.buffer_storage_parts_mut_for_sequence(handle, sequence_index) else {
			panic!("Missing DX12 dynamic buffer. The most likely cause is that the buffer handle came from another device.");
		};
		unsafe { &mut *(data as *mut T) }
	}

	pub(crate) fn resize_image_internal(&mut self, image_handle: ImageHandle, extent: Extent) {
		// Resizes CPU-side image storage without emitting GPU commands.
		let Some(current) = self.images.get(image_handle.0.0 as usize) else {
			return;
		};
		if current.extent == extent {
			return;
		}
		let format = current.format;
		let uses = current.uses;
		let array_layers = current.array_layers;
		let mip_levels = current.mip_levels;
		let optimized_clear_value = current.optimized_clear_value;
		let mut retired_state_keys = SmallVec::<[usize; 4]>::new();
		retired_state_keys.extend(current.resource.as_ref().map(Self::native_resource_key));
		if let Some(frame_resources) = current.frame_resources.as_ref() {
			retired_state_keys.extend(frame_resources.iter().flatten().map(Self::native_resource_key));
		}
		let resource = self.create_image_resource(extent, format, uses, array_layers, mip_levels, optimized_clear_value);
		self.invalidate_attachment_views_for_resources(&retired_state_keys);
		self.invalidate_clear_uav_descriptors_for_resources(&retired_state_keys);
		if let Some(resource) = resource.as_ref() {
			self.materialize_image_attachment_views(resource, format, uses, array_layers);
		}
		for &key in &retired_state_keys {
			self.image_states.remove(&key);
		}

		let image = &mut self.images[image_handle.0.0 as usize];
		image.extent = extent;
		image.resource = resource.clone();
		image.data = utils::texture_copy_size(image.format, extent).map(|size| vec![0u8; size]);
		if let Some(frame_data) = image.frame_data.as_mut() {
			let data = image.data.clone().unwrap_or_default();
			*frame_data = vec![data; self.frames as usize];
		}
		if let Some(frame_resources) = image.frame_resources.as_mut() {
			*frame_resources = vec![None; self.frames as usize];
			if let Some(first_resource) = resource {
				frame_resources[0] = Some(first_resource);
			}
		}
		self.invalidate_descriptor_materializations();
	}
}
