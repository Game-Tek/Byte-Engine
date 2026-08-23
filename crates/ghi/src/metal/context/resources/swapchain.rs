use super::super::*;

impl Context {
	pub fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		_presentation_mode: graphics_hardware_interface::PresentationModes,
		_fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		let layer = CAMetalLayer::new();
		layer.setDevice(Some(&self.device));
		layer.setPixelFormat(mtl::MTLPixelFormat::BGRA8Unorm);
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
		});

		handle
	}
}
