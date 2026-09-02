use super::super::*;

impl Device {
	pub fn build_dynamic_image(&mut self, builder: image::Builder) -> crate::DynamicImageHandle {
		let handle = self.build_image(builder.use_case(crate::UseCases::DYNAMIC));
		crate::DynamicImageHandle(handle.0)
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
		let image = &mut self.images[texture_handle.0 as usize];
		let data = if let Some(frame_data) = image.frame_data.as_mut() {
			let requested_index = usize::from(sequence_index);
			let index = if requested_index < frame_data.len() {
				requested_index
			} else {
				0
			};
			frame_data.get_mut(index)
		} else {
			image.data.as_mut()
		};
		let Some(data) = data else { return &mut [] };
		data.as_mut_slice()
	}

	pub fn write_texture(&mut self, texture_handle: ImageHandle, f: impl FnOnce(&mut [u8])) {
		// Writes into CPU-side staging storage when available.
		let Some(image) = self.images.get_mut(texture_handle.0.0 as usize) else {
			return;
		};

		let Some(staging) = image.data.as_mut() else {
			return;
		};

		f(staging);
	}

	pub(crate) fn queue_texture_sync_for_sequence(&mut self, image_handle: crate::BaseImageHandle, sequence_index: u8) {
		if !self
			.pending_texture_syncs
			.iter()
			.any(|&(pending_image, pending_sequence)| pending_image == image_handle && pending_sequence == sequence_index)
		{
			self.pending_texture_syncs.push((image_handle, sequence_index));
		}
	}

	pub fn build_image(&mut self, builder: image::Builder) -> ImageHandle {
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
		let size = utils::texture_copy_size(builder.format, builder.extent);
		let data = size.map(|bytes| vec![0u8; bytes]);
		let frame_data = if builder.use_case == UseCases::DYNAMIC {
			data.as_ref().map(|data| vec![data.clone(); self.frames as usize])
		} else {
			None
		};
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
			data,
			frame_data,
			frame_resources,
			optimized_clear_value,
		});

		let handle = crate::BaseImageHandle((self.images.len() - 1) as u64);
		if initializes_frame_resources {
			// Committed textures have undefined contents. Upload each frame's zeroed staging image on first use.
			for sequence_index in 0..self.frames {
				self.pending_texture_syncs.push((handle, sequence_index));
			}
		}

		ImageHandle(handle)
	}

	pub(crate) fn image_resource_state(&self, image: ImageHandle) -> Option<(Extent, bool)> {
		self.images
			.get(image.0.0 as usize)
			.map(|image| (image.extent, image.resource.is_some()))
	}

	#[cfg(test)]
	pub(crate) fn image_native_dimension(&self, image: ImageHandle) -> Option<(i32, u16)> {
		let resource = self.images.get(image.0.0 as usize)?.resource.as_ref()?;
		// SAFETY: The image registry retains this COM resource for the duration of the query.
		let descriptor = unsafe { resource.GetDesc() };
		Some((descriptor.Dimension.0, descriptor.DepthOrArraySize))
	}

	pub(crate) fn image_frame_resource_state(&self, image: ImageHandle, sequence_index: u8) -> Option<bool> {
		self.images.get(image.0.0 as usize).map(|image| {
			image
				.frame_resources
				.as_ref()
				.and_then(|resources| resources.get(sequence_index as usize))
				.and_then(|resource| resource.as_ref())
				.is_some()
		})
	}

	pub(crate) fn tracked_image_resource_state(&self, image: ImageHandle) -> Option<TextureBarrierState> {
		self.tracked_image_resource_state_for_sequence(image, 0)
	}

	pub(crate) fn tracked_image_resource_state_for_sequence(
		&self,
		image: ImageHandle,
		sequence_index: u8,
	) -> Option<TextureBarrierState> {
		let image = self.images.get(image.0.0 as usize)?;
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
		let (extent, is_3d, format, uses, array_layers, mip_levels, optimized_clear_value, dynamic) = {
			let image = self.images.get(image_handle.0 as usize)?;
			(
				image.extent,
				image.is_3d,
				image.format,
				image.uses,
				image.array_layers,
				image.mip_levels,
				image.optimized_clear_value,
				image.frame_resources.is_some(),
			)
		};
		if !dynamic {
			let resource = self
				.images
				.get(image_handle.0 as usize)
				.and_then(|image| image.resource.clone());
			if let (Some(command_buffer), Some(resource)) = (self.active_command_buffer, resource.as_ref()) {
				self.retain_command_buffer_resource(command_buffer, resource);
			}
			return resource;
		}

		let frame_index = sequence_index as usize;
		let needs_resource = self
			.images
			.get(image_handle.0 as usize)
			.and_then(|image| image.frame_resources.as_ref())
			.and_then(|resources| resources.get(frame_index))
			.and_then(Clone::clone)
			.is_none();

		if needs_resource {
			let resource =
				self.create_image_resource(extent, is_3d, format, uses, array_layers, mip_levels, optimized_clear_value);
			if let Some(resource) = resource.as_ref() {
				self.materialize_image_attachment_views(resource, format, uses, array_layers);
			}
			let image = self.images.get_mut(image_handle.0 as usize)?;
			if let Some(resources) = image.frame_resources.as_mut() {
				if resources.len() <= frame_index {
					resources.resize(frame_index + 1, None);
				}
				resources[frame_index] = resource.clone();
			}
		}

		let resource = self
			.images
			.get(image_handle.0 as usize)
			.and_then(|image| image.frame_resources.as_ref())
			.and_then(|resources| resources.get(frame_index))
			.and_then(Clone::clone);
		if let (Some(command_buffer), Some(resource)) = (self.active_command_buffer, resource.as_ref()) {
			self.retain_command_buffer_resource(command_buffer, resource);
		}
		resource
	}

	pub(crate) fn image_resource_for_sequence(
		&self,
		image_handle: crate::BaseImageHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		let image = self.images.get(image_handle.0 as usize)?;
		if let Some(resources) = image.frame_resources.as_ref() {
			return resources
				.get(sequence_index as usize)
				.and_then(Clone::clone)
				.or_else(|| resources.first().and_then(Clone::clone));
		}
		image.resource.clone()
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
		self.render_target_view_allocation_count
	}

	#[cfg(test)]
	pub(crate) fn depth_stencil_view_allocation_count(&self) -> usize {
		self.depth_stencil_view_allocation_count
	}

	#[cfg(test)]
	pub(crate) fn depth_stencil_descriptor_count(&self) -> u32 {
		self.depth_stencil_views
			.values()
			.map(|view| unsafe { view.heap.native.GetDesc() }.NumDescriptors)
			.sum()
	}

	#[cfg(test)]
	pub(crate) fn depth_stencil_view_array_range(
		array_layers: u32,
		layer: Option<u32>,
		layer_count: u32,
	) -> Option<(u32, u32)> {
		let descriptor = Self::depth_stencil_view_desc(Formats::Depth32, array_layers, layer, layer_count);
		if descriptor.ViewDimension != D3D12_DSV_DIMENSION_TEXTURE2DARRAY {
			return None;
		}
		let array = unsafe { descriptor.Anonymous.Texture2DArray };
		Some((array.FirstArraySlice, array.ArraySize))
	}

	pub(crate) fn texture_readback_resolve_count(&self) -> usize {
		self.texture_readback_resolve_count
	}

	pub(crate) fn image_is_in_common_state(&self, image: ImageHandle) -> Option<bool> {
		self.images
			.get(image.0.0 as usize)
			.and_then(|image_data| image_data.resource.as_ref())
			.map(|resource| {
				self.image_states
					.get(&Self::native_resource_key(resource))
					.copied()
					.unwrap_or(TextureBarrierState::COMMON)
					== TextureBarrierState::COMMON
			})
	}
}
