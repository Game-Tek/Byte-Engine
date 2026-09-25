use super::super::*;

impl Context {
	/// Acquires the next drawable for `swapchain_handle` and records it as the pending presentation of `sequence_index`.
	///
	/// Returns `None` when the layer has no drawable to give, which happens once the pool is exhausted while the
	/// window is occluded or hidden. No presentation is pending for the swapchain in that case.
	pub(crate) fn acquire_swapchain_image_for_sequence(
		&mut self,
		sequence_index: u8,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		// Update layer extent before acquiring the drawable so that if a resize occurred,
		// the drawable is allocated at the correct size. update_layer_extent only calls
		// setDrawableSize when the size actually changed, avoiding unnecessary drawable
		// pool invalidation.
		let extent = {
			let swapchain = &self.swapchains[swapchain_handle.0 as usize];
			update_layer_extent(&swapchain.layer, &swapchain.view)
		};
		self.swapchains[swapchain_handle.0 as usize].extent = extent;

		// Proxy swapchains must keep their intermediate texture aligned with the drawable.
		if self.swapchains[swapchain_handle.0 as usize].uses_proxy {
			self.resize_swapchain_images(swapchain_handle, extent);
		}

		let drawable = {
			// SAFETY: The pool is created and drained on this thread around a call that may run outside any frame pool.
			let _pool = unsafe { NSAutoreleasePool::new() };
			self.swapchains[swapchain_handle.0 as usize].layer.nextDrawable()
		};

		let Some(drawable) = drawable else {
			self.swapchains[swapchain_handle.0 as usize].pending_drawable = None;
			return None;
		};

		let present_key = graphics_hardware_interface::PresentKey {
			image_index: 0,
			sequence_index,
			swapchain: swapchain_handle,
		};

		let present_time = {
			let bits = self.swapchains[swapchain_handle.0 as usize]
				.last_presented_time
				.load(std::sync::atomic::Ordering::Acquire);
			presented_time_to_instant(f64::from_bits(bits))
		};

		self.swapchains[swapchain_handle.0 as usize].pending_drawable = Some(drawable);
		if !self.swapchains[swapchain_handle.0 as usize].uses_proxy {
			// A CAMetalLayer supplies a different drawable texture on each acquisition.
			self.rewrite_descriptors_for_handle(PrivateHandles::Swapchain(crate::swapchain::SwapchainHandle(
				swapchain_handle.0,
			)));
		}

		Some(crate::frame::SwapchainAcquisition {
			present_key,
			extent,
			present_time,
		})
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
