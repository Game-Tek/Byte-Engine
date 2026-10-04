use super::super::*;

impl Context {
	/// Prepares `swapchain_handle` for the frame of `sequence_index` and returns the key that presents it.
	///
	/// This does not take a drawable. `nextDrawable` blocks until the display releases one, so the drawable is taken
	/// at submission, after the frame's commands were committed, and only the copy into it waits for the display; see
	/// [`crate::metal::Frame::execute_finished_batch`]. Frames render into the sequence's swapchain image instead.
	pub(crate) fn acquire_swapchain_image_for_sequence(
		&mut self,
		sequence_index: u8,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		// Update the layer extent here so drawables taken at submission already have the frame's size.
		// update_layer_extent only calls setDrawableSize when the size actually changed, avoiding unnecessary drawable
		// pool invalidation.
		let extent = {
			let swapchain = &self.swapchains[swapchain_handle.0 as usize];
			update_layer_extent(&swapchain.layer, &swapchain.view)
		};
		self.swapchains[swapchain_handle.0 as usize].extent = extent;

		// A zero-sized window has nothing to render, and Metal cannot create an empty texture.
		if extent.width() > 0 && extent.height() > 0 {
			self.update_swapchain_images(swapchain_handle, extent);
		}

		let present_key = graphics_hardware_interface::PresentKey {
			image_index: 0,
			sequence_index,
			swapchain: swapchain_handle,
		};

		let present_time = {
			let swapchain = &mut self.swapchains[swapchain_handle.0 as usize];
			let bits = swapchain.last_presented_time.load(std::sync::atomic::Ordering::Acquire);
			// While nothing new reaches the screen, as when the window is minimized, the presented time stays the
			// same. Reconverting it would jitter, and the frame clock would take each jitter for a new presentation.
			if swapchain.presented_instant.is_none_or(|(converted, _)| converted != bits) {
				swapchain.presented_instant = presented_time_to_instant(f64::from_bits(bits)).map(|instant| (bits, instant));
			}
			swapchain.presented_instant.map(|(_, instant)| instant)
		};

		Some(crate::frame::SwapchainAcquisition {
			present_key,
			extent,
			present_time,
		})
	}

	/// Gives every frame sequence of `swapchain_handle` an image at `extent`, creating missing ones and resizing the rest.
	///
	/// Sequences above the context's frame count get no image, so they cost no memory until frames in flight grow.
	fn update_swapchain_images(&mut self, swapchain_handle: graphics_hardware_interface::SwapchainHandle, extent: Extent) {
		let uses = self.swapchains[swapchain_handle.0 as usize].uses;
		let mut changed = false;
		for sequence_index in 0..self.frames as usize {
			match self.swapchains[swapchain_handle.0 as usize].images[sequence_index] {
				Some(image) => changed |= self.resize_image_internal(image, extent),
				None => {
					let description = image::ImageDescription::new(
						&image_builder::Builder::new(SWAPCHAIN_FORMAT, uses | Uses::BlitSource).extent(extent),
					);
					let image = build_image(&self.device, Some("Swapchain Image"), description, self.settings.debug_labels);
					self.swapchains[swapchain_handle.0 as usize].images[sequence_index] = Some(self.images.add(image).1);
					changed = true;
				}
			}
		}

		if changed {
			// Swapchain descriptors resolve to these images when encoded, so only a new backing invalidates them.
			self.rewrite_descriptors_for_handle(PrivateHandles::Swapchain(crate::swapchain::SwapchainHandle(
				swapchain_handle.0,
			)));
		}
	}
}

/// Converts a `CAMetalDrawable::presentedTime` host time into an `Instant`.
///
/// Both clocks derive from `mach_absolute_time`, so the conversion is an offset from now.
/// Returns `None` when nothing was shown yet (Metal reports zero) or the value lies in the future.
fn presented_time_to_instant(presented_time: f64) -> Option<std::time::Instant> {
	if presented_time <= 0.0 {
		return None;
	}
	let now = std::time::Instant::now();
	let age = objc2_quartz_core::CACurrentMediaTime() - presented_time;
	if age < 0.0 {
		return None;
	}
	now.checked_sub(std::time::Duration::from_secs_f64(age))
}
