use super::super::*;

impl Context {
	pub fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: graphics_hardware_interface::PresentationModes,
		_fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		let layer = CAMetalLayer::new();

		layer.setDevice(Some(&self.device));
		layer.setPixelFormat(mtl::MTLPixelFormat::BGRA8Unorm);

		let display_sync_enabled = match presentation_mode {
			graphics_hardware_interface::PresentationModes::Inmediate => false,
			graphics_hardware_interface::PresentationModes::FIFO | graphics_hardware_interface::PresentationModes::Mailbox => {
				true
			}
		};

		layer.setDisplaySyncEnabled(display_sync_enabled);

		let desired_drawable_count = match presentation_mode {
			graphics_hardware_interface::PresentationModes::Inmediate
			| graphics_hardware_interface::PresentationModes::FIFO => 2,
			graphics_hardware_interface::PresentationModes::Mailbox => 3,
		};

		// A value other than 2 or 3 causes an exception
		layer.setMaximumDrawableCount(desired_drawable_count);

		let uses_proxy = !drawable_supports_uses(uses);

		// framebufferOnly permits Metal's optimized display path when raster output is the drawable's only use.
		let framebuffer_only_uses = Uses::RenderTarget | Uses::Clear;
		layer.setFramebufferOnly(!uses_proxy && framebuffer_only_uses.contains(uses));

		window_os_handles.view.setWantsLayer(true);
		window_os_handles.view.setLayer(Some(layer.as_super()));

		let extent = get_layer_extent(&layer, &window_os_handles.view);

		let format = mtl::MTLPixelFormat::BGRA8Unorm;

		let format = match format {
			mtl::MTLPixelFormat::BGRA8Unorm => crate::Formats::BGRAu8,
			mtl::MTLPixelFormat::BGRA8Unorm_sRGB => crate::Formats::BGRAsRGB,
			_ => panic!(
				"Unsupported Metal swapchain pixel format. The most likely cause is that the layer pixel format does not have a matching GHI format."
			),
		};

		let mut images = [None; super::super::MAX_SWAPCHAIN_IMAGES];

		if uses_proxy {
			// Create proxies for every swapchain image

			for image_index in 0..super::super::MAX_SWAPCHAIN_IMAGES {
				let proxy = self.create_image_resource(
					Some("Swapchain Proxy Image"),
					extent,
					format,
					uses | Uses::BlitSource,
					DeviceAccesses::DeviceOnly,
					1,
					false,
					false,
					1,
				);

				let image_handle = self.images.add(proxy);

				images[image_index] = Some(image_handle.1);
			}
		}

		let handle = graphics_hardware_interface::SwapchainHandle(self.swapchains.len() as u64);

		self.swapchains.push(Swapchain {
			layer,
			view: window_os_handles.view.clone(),
			images,
			uses_proxy,
			uses,
			extent,
			pending_drawable: None,
			last_presented_time: std::sync::Arc::new(AtomicU64::new(0)),
		});

		handle
	}

	/// Acquires the drawable that `frame` will present before the frame is started.
	///
	/// Waiting on the frame sequence's synchronizer first gives the same reuse guarantee that
	/// `start_frame` provides for in-frame acquisition. The synchronizer stays signaled, so the
	/// later `start_frame` wait returns immediately.
	pub fn acquire_swapchain_image(
		&mut self,
		frame: crate::queue::FrameRequest<'_>,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> crate::frame::SwapchainAcquisition {
		let sequence_index = (frame.index % u64::from(self.frames)) as u8;
		let synchronizer_handle = self.synchronizer_for_sequence(frame.synchronizer, sequence_index);
		self.wait_for_private_synchronizer(synchronizer_handle);
		self.acquire_swapchain_image_for_sequence(sequence_index, swapchain_handle)
	}

	/// Acquires the next drawable for `swapchain_handle` and records it as the pending presentation of `sequence_index`.
	pub(crate) fn acquire_swapchain_image_for_sequence(
		&mut self,
		sequence_index: u8,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> crate::frame::SwapchainAcquisition {
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
			self.swapchains[swapchain_handle.0 as usize]
				.layer
				.nextDrawable()
				.expect("Failed to acquire Metal drawable. The most likely cause is that the layer has no available drawables.")
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

		crate::frame::SwapchainAcquisition {
			present_key,
			extent,
			present_time,
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
