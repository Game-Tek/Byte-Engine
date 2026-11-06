use super::super::*;

impl Device {
	pub fn build_dynamic_image(&mut self, builder: image::Builder) -> crate::DynamicImageHandle {
		let handle = self.build_image(builder.use_case(crate::UseCases::DYNAMIC));
		crate::DynamicImageHandle(handle.0)
	}

	/// Moves a finished image out of this context so another context of the same device can intern it.
	///
	/// Wait for every submission that uses the image before exporting it. The handle is invalid in this context
	/// afterwards. Next, pass the image to [`Self::intern_image`] or [`crate::dx12::Frame::intern_image`] on the
	/// receiving context.
	///
	/// # Panics
	///
	/// Panics when the handle names no image of this context, an image with one representation per frame, or an
	/// image-group member, whose memory belongs to this context's group heap.
	pub fn export_image(&mut self, image_handle: ImageHandle) -> crate::dx12::factory::DetachedImage {
		let handle = image_handle.0;
		assert!(
			self.image_groups.member(handle).is_none(),
			"Image export failed. The most likely cause is that the image is an image-group member, whose memory stays with its group."
		);
		let slot = self
			.images
			.get_mut(handle.index() as usize)
			.expect("Image export failed. The most likely cause is that the handle is stale or belongs to another context.");
		assert!(
			slot.frame_resources.is_none() && slot.resource.is_some(),
			"Image export failed. The most likely cause is that the handle names a dynamic image or one already exported."
		);
		// The slot stays behind without a resource, so the handle reads as exported instead of naming another image.
		let image = std::mem::replace(
			slot,
			Image {
				extent: slot.extent,
				is_3d: slot.is_3d,
				format: slot.format,
				uses: slot.uses,
				access: slot.access,
				array_layers: slot.array_layers,
				mip_levels: slot.mip_levels,
				resource: None,
				data: None,
				frame_data: None,
				frame_resources: None,
				optimized_clear_value: None,
				name: None,
			},
		);
		// A pending upload would read the image after it left the context.
		self.pending_texture_syncs.retain(|&(pending, ..)| pending != handle);
		let key = image.resource.as_ref().map(Self::native_resource_key);
		// Views cached by resource address would otherwise outlive the resource and match a later one at that address.
		self.invalidate_attachment_views_for_resources(key.as_slice());
		self.invalidate_clear_uav_descriptors_for_resources(key.as_slice());
		let state = key.and_then(|key| self.image_states.remove(&key));
		crate::dx12::factory::DetachedImage { image, state }
	}

	/// Deletes an image once frame sequence `sequence_index` starts again, when every frame recorded so far has
	/// completed. See [`crate::dx12::Frame::delete_image`].
	pub(crate) fn delete_image(&mut self, image: ImageHandle, sequence_index: u8) {
		if let Some(resource) = self.export_image(image).image.resource {
			self.defer_task(sequence_index, DeferredTask::RetireResource(resource));
		}
	}

	/// Interns an image another context exported and returns the handle recordings use it with.
	///
	/// The image keeps the barrier state its last submission left it in, so the first barrier here starts from it.
	pub fn intern_image(&mut self, image: crate::dx12::factory::DetachedImage) -> ImageHandle {
		let crate::dx12::factory::DetachedImage { image, state } = image;
		if let Some(resource) = image.resource.as_ref() {
			self.materialize_image_attachment_views(resource, image.format, image.uses, image.array_layers);
			if let Some(state) = state {
				self.image_states.insert(Self::native_resource_key(resource), state);
			}
		}
		self.images.push(image);
		ImageHandle(crate::BaseImageHandle::from_index((self.images.len() - 1) as u64))
	}

	pub fn get_texture_slice_mut(&mut self, texture_handle: ImageHandle) -> &mut [u8] {
		self.texture_slice_mut_static(texture_handle.0)
	}

	pub(crate) fn texture_slice_mut_static(&mut self, texture_handle: crate::BaseImageHandle) -> &mut [u8] {
		self.texture_slice_mut_for_sequence(texture_handle, 0)
	}

	pub(crate) fn texture_slice_mut_for_sequence(
		&mut self,
		texture_handle: crate::BaseImageHandle,
		sequence_index: u8,
	) -> &mut [u8] {
		let image = &mut self.images[texture_handle.index() as usize];
		let Some(data) = image.staging_mut(sequence_index) else {
			return &mut [];
		};
		data.as_mut_slice()
	}

	pub fn write_texture(&mut self, texture_handle: ImageHandle, f: impl FnOnce(&mut [u8])) {
		// Writes into CPU-side staging storage when the format has a CPU layout.
		let Some(image) = self.images.get_mut(texture_handle.0.index() as usize) else {
			return;
		};

		let Some(staging) = image.staging_mut(0) else {
			return;
		};

		f(staging);
	}

	pub(crate) fn queue_texture_sync_for_sequence(&mut self, image_handle: crate::BaseImageHandle, sequence_index: u8) {
		if !self
			.pending_texture_syncs
			.iter()
			.any(|&(pending_image, pending_sequence, region)| {
				pending_image == image_handle && pending_sequence == sequence_index && region.is_none()
			}) {
			self.pending_texture_syncs.push((image_handle, sequence_index, None));
		}
	}

	/// Queues one validated patch while keeping image storage owned by the device.
	pub(crate) fn queue_texture_region_for_sequence(
		&mut self,
		image_handle: crate::BaseImageHandle,
		sequence_index: u8,
		region: crate::image::Region,
	) {
		let image = &self.images[image_handle.index() as usize];
		region.validate(image.extent, image.format, image.array_layers);
		self.pending_texture_syncs.push((image_handle, sequence_index, Some(region)));
	}

	pub fn build_image(&mut self, builder: image::Builder) -> ImageHandle {
		if builder.group.is_some() {
			crate::image_group::ImageGroups::validate_member(&builder);
		}
		// Reject unsupported view contracts before a logical handle can escape to callers.
		let array_layers = builder.array_layers.map(|layers| layers.get()).unwrap_or(1);
		let is_3d = builder.extent.depth() > 1;
		Self::validate_image_dimension(
			builder.extent,
			is_3d,
			array_layers,
			builder.cube_compatible || builder.cube_array_compatible,
		);
		self.validate_image_format_support(builder.format, builder.resource_uses, is_3d);
		// The CPU staging copies wait for the first CPU write; see `Image::staging_mut`.
		let frame_data = (builder.use_case == UseCases::DYNAMIC).then(|| vec![Vec::new(); self.frames as usize]);
		let initializes_frame_resources = frame_data.is_some();
		let flags = Self::image_resource_flags(builder.format, builder.resource_uses);
		let optimized_clear_value = builder
			.optimized_clear_value
			.and_then(|clear| Self::optimized_image_clear_value(builder.format, flags, clear));
		let resource = if builder.use_case == UseCases::DYNAMIC {
			None
		} else {
			self.create_image_resource(
				builder.extent,
				is_3d,
				builder.format,
				builder.resource_uses,
				array_layers,
				builder.mip_levels,
				optimized_clear_value,
				builder.name,
			)
		};
		if let Some(resource) = resource.as_ref() {
			self.materialize_image_attachment_views(resource, builder.format, builder.resource_uses, array_layers);
		}
		let frame_resources = if builder.use_case == UseCases::DYNAMIC {
			let mut resources = vec![None; self.frames as usize];
			if let Some(first_resource) = resource.clone() {
				if let Some(slot) = resources.first_mut() {
					*slot = Some(first_resource);
				}
			}
			Some(resources)
		} else {
			None
		};

		self.images.push(Image {
			extent: builder.extent,
			is_3d,
			format: builder.format,
			uses: builder.resource_uses,
			access: builder.device_accesses,
			array_layers,
			mip_levels: builder.mip_levels,
			resource,
			data: None,
			frame_data,
			frame_resources,
			optimized_clear_value,
			name: self.debug_name(builder.name),
		});

		let handle = crate::BaseImageHandle::from_index((self.images.len() - 1) as u64);
		// A member built without an extent has no resource until its group is placed.
		if let Some(group) = builder.group {
			self.image_groups.add_member(group, handle);
		}
		if initializes_frame_resources {
			// Committed textures have undefined contents. Each frame's first use uploads zeros in their place.
			for sequence_index in 0..self.frames {
				self.pending_texture_syncs.push((handle, sequence_index, None));
			}
		}

		ImageHandle(handle)
	}

	#[cfg(test)]
	pub(crate) fn image_native_dimension(&self, image: ImageHandle) -> Option<(i32, u16)> {
		let resource = self.images.get(image.0.index() as usize)?.resource.as_ref()?;
		// SAFETY: The image registry retains this COM resource for the duration of the query.
		let descriptor = unsafe { resource.GetDesc() };
		Some((descriptor.Dimension.0, descriptor.DepthOrArraySize))
	}

	pub(crate) fn tracked_image_resource_state(&self, image: ImageHandle) -> Option<TextureBarrierState> {
		self.tracked_image_resource_state_for_sequence(image, 0)
	}

	pub(crate) fn tracked_image_resource_state_for_sequence(
		&self,
		image: ImageHandle,
		sequence_index: u8,
	) -> Option<TextureBarrierState> {
		let image = self.images.get(image.0.index() as usize)?;
		let resource = if let Some(resources) = image.frame_resources.as_ref() {
			resources.get(sequence_index as usize)?.as_ref()?
		} else {
			image.resource.as_ref()?
		};
		self.image_states.get(&Self::native_resource_key(resource)).copied()
	}

	#[cfg(test)]
	pub(crate) fn pending_texture_sync_count(&self) -> usize {
		self.pending_texture_syncs.len()
	}

	/// Returns the native texture for a frame, creating deferred dynamic image resources on first use.
	pub(crate) fn ensure_image_resource_for_sequence(
		&mut self,
		image_handle: crate::BaseImageHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		let (extent, is_3d, format, uses, array_layers, mip_levels, optimized_clear_value, dynamic, name) = {
			let image = self.images.get(image_handle.index() as usize)?;
			(
				image.extent,
				image.is_3d,
				image.format,
				image.uses,
				image.array_layers,
				image.mip_levels,
				image.optimized_clear_value,
				image.frame_resources.is_some(),
				image.name.clone(),
			)
		};
		if !dynamic {
			let resource = self
				.images
				.get(image_handle.index() as usize)
				.and_then(|image| image.resource.clone());
			if let (Some(command_buffer), Some(resource)) = (self.active_command_buffer, resource.as_ref()) {
				self.retain_command_buffer_resource(command_buffer, resource);
			}
			return resource;
		}

		let frame_index = sequence_index as usize;
		let needs_resource = self
			.images
			.get(image_handle.index() as usize)
			.and_then(|image| image.frame_resources.as_ref())
			.and_then(|resources| resources.get(frame_index))
			.and_then(Clone::clone)
			.is_none();

		if needs_resource {
			let resource = self.create_image_resource(
				extent,
				is_3d,
				format,
				uses,
				array_layers,
				mip_levels,
				optimized_clear_value,
				name.as_deref(),
			);
			if let Some(resource) = resource.as_ref() {
				self.materialize_image_attachment_views(resource, format, uses, array_layers);
			}
			let image = self.images.get_mut(image_handle.index() as usize)?;
			if let Some(resources) = image.frame_resources.as_mut() {
				if resources.len() <= frame_index {
					resources.resize(frame_index + 1, None);
				}
				resources[frame_index] = resource.clone();
			}
		}

		let resource = self
			.images
			.get(image_handle.index() as usize)
			.and_then(|image| image.frame_resources.as_ref())
			.and_then(|resources| resources.get(frame_index))
			.and_then(Clone::clone);
		if let (Some(command_buffer), Some(resource)) = (self.active_command_buffer, resource.as_ref()) {
			self.retain_command_buffer_resource(command_buffer, resource);
		}
		resource
	}

	#[cfg(test)]
	pub(crate) fn render_target_view_count(&self) -> usize {
		self.render_target_views.len()
	}

	#[cfg(test)]
	pub(crate) fn depth_stencil_view_count(&self) -> usize {
		self.depth_stencil_views.len()
	}

	#[cfg(test)]
	pub(crate) fn render_target_view_allocation_count(&self) -> usize {
		self.native_counters.render_target_view_allocation_count
	}

	#[cfg(test)]
	pub(crate) fn depth_stencil_view_allocation_count(&self) -> usize {
		self.native_counters.depth_stencil_view_allocation_count
	}

	#[cfg(test)]
	pub(crate) fn depth_stencil_descriptor_count(&self) -> u32 {
		self.depth_stencil_views
			.values()
			.map(|view| unsafe { view.native.GetDesc() }.NumDescriptors)
			.sum()
	}

	#[cfg(test)]
	pub(crate) fn depth_stencil_view_array_range(array_layers: u32, first_layer: u32, layer_count: u32) -> Option<(u32, u32)> {
		let descriptor = Self::depth_stencil_view_desc(Formats::Depth32, array_layers, first_layer, layer_count);
		if descriptor.ViewDimension != D3D12_DSV_DIMENSION_TEXTURE2DARRAY {
			return None;
		}
		let array = unsafe { descriptor.Anonymous.Texture2DArray };
		Some((array.FirstArraySlice, array.ArraySize))
	}

	pub(crate) fn texture_readback_resolve_count(&self) -> usize {
		self.native_counters.texture_readback_resolve_count
	}
}
