use super::super::*;

impl Context {
	/// Interns a factory-built image into this device and returns its public image handle.
	pub fn intern_image(&mut self, image: crate::metal::device::Image) -> graphics_hardware_interface::ImageHandle {
		let name = image.image.name.clone();
		let (root_image_handle, _) = self.images.add(image.image);
		let handle = graphics_hardware_interface::ImageHandle(root_image_handle);

		#[cfg(debug_assertions)]
		{
			if let Some(name) = name {
				self.names.insert(graphics_hardware_interface::Handles::Image(handle), name);
			}
		}

		handle
	}

	pub fn build_dynamic_image(&mut self, builder: image_builder::Builder) -> graphics_hardware_interface::DynamicImageHandle {
		let layers = builder.array_layers.map(|l| l.get()).unwrap_or(1);
		let root = self.create_image_internal(
			None,
			builder.get_name(),
			builder.extent,
			builder.format,
			builder.resource_uses,
			builder.device_accesses,
			layers,
			builder.cube_compatible,
			builder.cube_array_compatible,
			builder.mip_levels,
		);
		let master = graphics_hardware_interface::BaseImageHandle::new(root.0);

		if self.frames > 1 {
			// Defer frame-local resources until the frame is first processed so startup only pays for frame 0.
			self.tasks.push(Task::new(
				Tasks::BuildImage(BuildImage {
					previous: root,
					master: graphics_hardware_interface::ImageHandle(master),
				}),
				Some(1),
			));
		}

		graphics_hardware_interface::DynamicImageHandle(master)
	}

	pub fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::ImageHandle) -> &mut [u8] {
		let handle = self.images.nth_handle(texture_handle.0, 0).unwrap();
		let image = self.images.resource_mut(handle);

		let Some(staging) = image.staging.as_mut() else {
			return &mut [];
		};

		staging.as_mut_slice()
	}

	pub fn write_texture(&mut self, texture_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let image_handle = self.images.nth_handle(texture_handle.0, 0).unwrap();
		let image = self.images.resource_mut(image_handle);

		let Some(staging) = image.staging.as_mut() else {
			return;
		};

		f(staging);
		self.pending_image_syncs.push_back(image_handle);
	}

	pub fn sync_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle) {
		let handle = self.images.nth_handle(image_handle.0, 0).unwrap();
		self.pending_image_syncs.push_back(handle);
	}

	pub fn build_image(&mut self, builder: image_builder::Builder) -> graphics_hardware_interface::ImageHandle {
		let layers = builder.array_layers.map(|l| l.get()).unwrap_or(1);
		let image_handle = self.create_image_internal(
			None,
			builder.get_name(),
			builder.extent,
			builder.format,
			builder.resource_uses,
			builder.device_accesses,
			layers,
			builder.cube_compatible,
			builder.cube_array_compatible,
			builder.mip_levels,
		);

		graphics_hardware_interface::ImageHandle(graphics_hardware_interface::BaseImageHandle::new(image_handle.0))
	}

	/// Waits for Metal work, copies one transfer result, and releases its native staging buffer.
	pub fn get_image_data(
		&mut self,
		texture_copy_handle: graphics_hardware_interface::TextureCopyHandle,
	) -> Result<crate::TextureReadback, crate::TextureTransferError> {
		self.texture_readbacks.submitted(texture_copy_handle)?;
		self.wait();
		let mut readback = self.texture_readbacks.take_submitted(texture_copy_handle)?;
		let pointer = readback.buffer.contents().as_ptr().cast::<u8>();
		// Metal requires aligned native rows. Repack once mapping is synchronized so callers receive the compact authoritative layout.
		for image in 0..readback.image_count {
			for row in 0..readback.row_count {
				let source_offset = image * readback.native_bytes_per_image + row * readback.native_bytes_per_row;
				let destination_offset = image * readback.bytes_per_image + row * readback.bytes_per_row;
				// SAFETY: Native readback layout calculations bound this row within the mapped Metal buffer.
				let source = unsafe { pointer.add(source_offset) };
				// SAFETY: Compact layout calculations bound this row within the owned destination vector.
				let destination = unsafe { readback.bytes.as_mut_ptr().add(destination_offset) };
				// SAFETY: The mapped native buffer and owned destination vector are distinct and cover one compact row.
				unsafe { std::ptr::copy_nonoverlapping(source, destination, readback.bytes_per_row) };
			}
		}

		Ok(crate::TextureReadback {
			bytes: readback.bytes,
			extent: readback.extent,
			format: readback.format,
			bytes_per_row: readback.bytes_per_row,
			bytes_per_image: readback.bytes_per_image,
		})
	}
}
