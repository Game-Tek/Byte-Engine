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

	pub fn get_texture_slice_mut(&self, texture_handle: graphics_hardware_interface::ImageHandle) -> &'static mut [u8] {
		let image = self.images.get_single(texture_handle.0).unwrap();

		let Some(staging) = image.staging.as_ref() else {
			return &mut [];
		};

		unsafe { std::slice::from_raw_parts_mut(staging.as_ptr() as *mut u8, staging.len()) }
	}

	pub fn write_texture(&mut self, texture_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let image = self.images.resource_mut(self.images.nth_handle(texture_handle.0, 0).unwrap());

		let Some(staging) = image.staging.as_mut() else {
			return;
		};

		f(staging);

		let texture = image.texture.clone();
		let format = image.format;
		let extent = image.extent;
		let array_layers = image.array_layers;
		let staging = staging.to_vec();

		self.upload_texture_from_staging(texture.as_ref(), format, extent, array_layers, &staging, None, 0);
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

	pub fn get_image_data(&mut self, texture_copy_handle: graphics_hardware_interface::TextureCopyHandle) -> &[u8] {
		let image = self.images.resource_mut(ImageHandle(texture_copy_handle.0));
		let Some(staging) = image.staging.as_mut() else {
			return &[];
		};
		let Some((bytes_per_row, ..)) = utils::texture_upload_layout(image.format, image.extent) else {
			return &[];
		};

		let data_ptr = NonNull::new(staging.as_mut_ptr() as *mut std::ffi::c_void)
			.expect("Texture readback buffer was null. The most likely cause is an empty image staging allocation.");
		let mut region_size = utils::texture_copy_size(image.format, image.extent);
		region_size.depth = 1;
		let region = mtl::MTLRegion {
			origin: mtl::MTLOrigin { x: 0, y: 0, z: 0 },
			size: region_size,
		};

		// `transfer_textures` synchronized the managed texture; now refresh its existing compact CPU staging allocation.
		unsafe {
			image
				.texture
				.getBytes_bytesPerRow_fromRegion_mipmapLevel(data_ptr, bytes_per_row as _, region, 0);
		}

		staging
	}
}
