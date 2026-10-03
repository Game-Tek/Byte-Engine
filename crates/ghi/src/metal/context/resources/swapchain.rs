use super::super::*;

/// The pixel format of every swapchain drawable and of the images frames render into before presentation.
pub(crate) const SWAPCHAIN_FORMAT: crate::Formats = crate::Formats::BGRAu8;

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
			match swapchain.presented_instant {
				// While nothing new reaches the screen, as when the window is minimized, the presented time stays the
				// same. Reconverting it would jitter, and the frame clock would take each jitter for a new presentation.
				Some((converted, instant)) if converted == bits => Some(instant),
				_ => {
					let instant = presented_time_to_instant(f64::from_bits(bits));
					swapchain.presented_instant = instant.map(|instant| (bits, instant));
					instant
				}
			}
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
					let description = image::ImageDescription {
						extent,
						format: SWAPCHAIN_FORMAT,
						uses: uses | Uses::BlitSource,
						access: DeviceAccesses::DeviceOnly,
						array_layers: 1,
						cube_compatible: false,
						cube_array_compatible: false,
						mip_levels: 1,
					};
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

	/// Takes the next drawable of `swapchain_handle`, blocking until the display releases one.
	///
	/// With display sync this usually returns at the refresh that shows the previously presented frame. Returns `None`
	/// when no drawable became available within Core Animation's one-second timeout, as happens while the window is
	/// occluded.
	pub(crate) fn next_drawable(
		&self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Option<Retained<ProtocolObject<dyn CAMetalDrawable>>> {
		// SAFETY: The pool is created and drained on this thread, so the drawable's autoreleased reference does not
		// outlive the call and only the returned reference keeps it from the layer's pool.
		let _pool = unsafe { NSAutoreleasePool::new() };
		self.swapchains[swapchain_handle.0 as usize].layer.nextDrawable()
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
