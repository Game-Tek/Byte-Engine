use objc2_metal::MTLTexture as _;

use super::super::*;

impl Context {
	/// Prepares `swapchain_handle` for the frame of `sequence_index` and returns the key that presents it.
	///
	/// Without a display link this does not take a drawable. `nextDrawable` blocks until the display releases one, so
	/// the drawable is taken at submission, after the frame's commands were committed, and only the copy into it waits
	/// for the display; see [`crate::metal::Frame::execute_finished_batch`].
	/// With a display link this waits here for the display's next opportunity, keeps its drawable for the sequence's
	/// presentation, and reports when the frame will be shown. Returns `None` when the display offered none within a
	/// second, as while the window is occluded.
	/// Either way, frames render into the sequence's swapchain image, which presentation copies into the drawable.
	pub(crate) fn acquire_swapchain_image_for_sequence(
		&mut self,
		sequence_index: u8,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		// Update the layer extent here so drawables taken at submission already have the frame's size.
		// update_layer_extent only calls setDrawableSize when the size actually changed, avoiding unnecessary drawable
		// pool invalidation.
		let swapchain = &mut self.swapchains[swapchain_handle.index() as usize];
		let extent = update_layer_extent(&swapchain.layer, &swapchain.view);
		swapchain.extent = extent;

		// A zero-sized window has nothing to render, and Metal cannot create an empty texture.
		if extent.width() > 0 && extent.height() > 0 {
			self.update_swapchain_images(swapchain_handle, extent);
		}

		// A display-linked swapchain waits here for the display and keeps the drawable for the sequence's presentation.
		// A drawable handed out before the window resized has the old extent, which the presentation copy cannot take,
		// so it is released and the next one taken.
		let swapchain = &mut self.swapchains[swapchain_handle.index() as usize];
		let target_present_time = match &swapchain.display_link {
			Some(display_link) if extent.width() > 0 && extent.height() > 0 => {
				let deadline = std::time::Instant::now() + crate::metal::display_link::UPDATE_TIMEOUT;
				let update = std::iter::from_fn(|| display_link.take(deadline)).find(|update| {
					let texture = update.drawable().texture();
					texture.width() as u32 == extent.width() && texture.height() as u32 == extent.height()
				})?;
				swapchain.drawables[sequence_index as usize] = Some(update.drawable());
				host_time_to_instant(update.targetPresentationTimestamp())
			}
			_ => None,
		};

		let present_key = graphics_hardware_interface::PresentKey {
			image_index: 0,
			sequence_index,
			swapchain: swapchain_handle,
		};

		let present_time = {
			let swapchain = &mut self.swapchains[swapchain_handle.index() as usize];
			let bits = swapchain.last_presented_time.load(std::sync::atomic::Ordering::Acquire);
			// While nothing new reaches the screen, as when the window is minimized, the presented time stays the
			// same. Reconverting it would jitter, and the frame clock would take each jitter for a new presentation.
			if swapchain.presented_instant.is_none_or(|(converted, _)| converted != bits) {
				// Metal reports zero until something was shown, and a presented time never lies ahead.
				let presented_time = f64::from_bits(bits);
				swapchain.presented_instant = host_time_to_instant(presented_time)
					.filter(|instant| presented_time > 0.0 && *instant <= std::time::Instant::now())
					.map(|instant| (bits, instant));
			}
			swapchain.presented_instant.map(|(_, instant)| instant)
		};

		Some(crate::frame::SwapchainAcquisition {
			present_key,
			extent,
			present_time,
			target_present_time,
		})
	}

	/// Gives every frame sequence of `swapchain_handle` an image at `extent`, creating missing ones and resizing the rest.
	///
	/// Sequences above the context's frame count get no image, so they cost no memory until frames in flight grow.
	fn update_swapchain_images(&mut self, swapchain_handle: graphics_hardware_interface::SwapchainHandle, extent: Extent) {
		let uses = self.swapchains[swapchain_handle.index() as usize].uses;
		let mut changed = false;
		for sequence_index in 0..self.frames as usize {
			match self.swapchains[swapchain_handle.index() as usize].images[sequence_index] {
				Some(image) => changed |= self.resize_image_internal(image, extent, None),
				None => {
					let description = ImageDescription::new(
						&image_builder::Builder::new(SWAPCHAIN_FORMAT, uses | Uses::BlitSource).extent(extent),
					);
					let image = self.create_image_resource(Some("Swapchain Image"), description);
					self.swapchains[swapchain_handle.index() as usize].images[sequence_index] = Some(self.images.add(image).1);
					changed = true;
				}
			}
		}

		if changed {
			// Swapchain descriptors resolve to these images when encoded, so only a new backing invalidates them.
			self.rewrite_descriptors_for_handle(PrivateHandles::Swapchain(crate::swapchain::SwapchainHandle::from_index(
				swapchain_handle.index(),
			)));
		}
	}
}

/// Converts a Core Animation host time, such as a drawable's presented time or a display link's target time, into an
/// `Instant`.
///
/// Both clocks derive from `mach_absolute_time`, so the conversion is an offset from now, into the past or the future.
/// Returns `None` when the offset does not fit an `Instant`.
fn host_time_to_instant(host_time: f64) -> Option<std::time::Instant> {
	let now = std::time::Instant::now();
	let offset = host_time - objc2_quartz_core::CACurrentMediaTime();
	if !offset.is_finite() {
		return None;
	}
	let magnitude = std::time::Duration::try_from_secs_f64(offset.abs()).ok()?;
	if offset >= 0.0 {
		now.checked_add(magnitude)
	} else {
		now.checked_sub(magnitude)
	}
}
