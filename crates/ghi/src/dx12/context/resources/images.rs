use super::super::*;

impl Device {
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
			"Image export failed. The most likely cause is that the handle names an image with a copy per frame, or one already exported. Export images built with Copies::One, once."
		);
		// The slot stays behind without a resource, so the handle reads as exported instead of naming another image.
		let image = std::mem::replace(
			slot,
			Image {
				extent: slot.extent,
				is_3d: slot.is_3d,
				format: slot.format,
				uses: slot.uses,
				array_layers: slot.array_layers,
				mip_levels: slot.mip_levels,
				resource: None,
				frame_resources: None,
				optimized_clear_value: None,
				name: None,
			},
		);
		let key = image.resource.as_ref().map(Self::native_resource_key);
		// Views cached by resource address would otherwise outlive the resource and match a later one at that address.
		let state = key.and_then(|key| self.image_states.remove(&key));
		self.forget_native_resources(key.as_slice());
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

	/// Returns how many copies of an image exist: one per frame in flight for an image with a copy per frame.
	pub(crate) fn image_copy_count(&self, image: crate::BaseImageHandle) -> usize {
		match self.images[image.index() as usize].frame_resources {
			Some(_) => self.frames as usize,
			None => 1,
		}
	}

	/// Returns the native texture for a frame, creating a per-frame image's copy for that frame on first use, and keeps
	/// it alive with the recording command buffer. Returns `None` for an image without memory yet.
	pub(crate) fn ensure_image_resource_for_sequence(
		&mut self,
		image_handle: crate::BaseImageHandle,
		sequence_index: u8,
	) -> Option<ID3D12Resource> {
		let index = image_handle.index() as usize;
		let frame = sequence_index as usize;
		let image = &self.images[index];
		let existing = match &image.frame_resources {
			None => Some(image.resource.clone()),
			Some(resources) => resources.get(frame).cloned().flatten().map(Some),
		};
		let resource = match existing {
			Some(resource) => resource,
			None => {
				let (extent, is_3d, format, uses, array_layers, mip_levels, optimized_clear_value, name) = (
					image.extent,
					image.is_3d,
					image.format,
					image.uses,
					image.array_layers,
					image.mip_levels,
					image.optimized_clear_value,
					image.name.clone(),
				);
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
				let resources = self.images[index]
					.frame_resources
					.as_mut()
					.expect("Only an image with a copy per frame lacks a sequence's texture.");
				if resources.len() <= frame {
					resources.resize(frame + 1, None);
				}
				resources[frame] = resource.clone();
				resource
			}
		};
		if let (Some(command_buffer), Some(resource)) = (self.active_command_buffer, resource.as_ref()) {
			self.retain_command_buffer_resource(command_buffer, resource);
		}
		resource
	}
}
