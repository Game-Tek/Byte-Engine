use super::*;

impl InnerDevice {
	fn create_vulkan_surface(&self, window_os_handles: &window::Handles) -> vk::SurfaceKHR {
		let wayland_surface_create_info = vk::WaylandSurfaceCreateInfoKHR::default()
			.display(window_os_handles.display)
			.surface(window_os_handles.surface);
		let surface = unsafe {
			self.wayland_surface
				.create_wayland_surface(&wayland_surface_create_info, None)
		}
		.expect("No surface");

		let surface_formats = unsafe {
			self.surface
				.get_physical_device_surface_formats(self.physical_device, surface)
		}
		.expect("No surface formats");
		assert!(
			surface_formats.iter().any(|format| {
				format.format == vk::Format::B8G8R8A8_SRGB && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
			}),
			"No surface format"
		);

		surface
	}

	pub fn build_swapchain(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: crate::PresentationModes,
		fallback_extent: Extent,
		uses: crate::Uses,
	) -> (
		vk::SurfaceKHR,
		vk::PresentModeKHR,
		u32,
		vk::Extent2D,
		crate::Formats,
		bool,
		vk::ImageUsageFlags,
		vk::SwapchainKHR,
	) {
		let vk_surface = self.create_vulkan_surface(window_os_handles);

		let vk_present_mode = match presentation_mode {
			graphics_hardware_interface::PresentationModes::FIFO => vk::PresentModeKHR::FIFO,
			graphics_hardware_interface::PresentationModes::Inmediate => vk::PresentModeKHR::IMMEDIATE,
			graphics_hardware_interface::PresentationModes::Mailbox => vk::PresentModeKHR::MAILBOX,
		};

		let vk_surface_capabilities = self.query_swapchain_surface_capabilities(vk_surface, vk_present_mode);
		let extent = Self::swapchain_extent(
			&vk_surface_capabilities,
			vk::Extent2D::default()
				.width(fallback_extent.width())
				.height(fallback_extent.height()),
		);

		let format = crate::Formats::BGRAsRGB;
		let requested_image_usage = into_vk_image_usage_flags(uses, format);
		let supported_image_usage = vk_surface_capabilities.supported_usage_flags;
		let uses_storage = uses.contains(crate::Uses::Storage);
		let uses_proxy_images = !supported_image_usage.contains(requested_image_usage)
			|| uses_storage && !self.swapchain_native_supports_formatless_storage_write;

		let native_image_usage = if uses_proxy_images {
			assert!(
				!uses_storage || self.swapchain_proxy_supports_formatless_storage_write,
				"Failed to create swapchain storage proxy image. The most likely cause is that the selected Vulkan device does not support storage writes without format for the swapchain proxy format."
			);
			assert!(
				supported_image_usage.contains(vk::ImageUsageFlags::TRANSFER_DST),
				"Failed to create swapchain fallback copy path. The most likely cause is that the surface does not support transfer destination usage for swapchain images."
			);
			vk::ImageUsageFlags::TRANSFER_DST
		} else {
			requested_image_usage
		};

		let vk_swapchain = self.create_vulkan_swapchain(
			vk_surface,
			vk_present_mode,
			&vk_surface_capabilities,
			extent,
			native_image_usage,
			vk::SwapchainKHR::null(),
		);
		(
			vk_surface,
			vk_present_mode,
			vk_surface_capabilities.min_image_count,
			extent,
			crate::Formats::BGRAu8,
			uses_proxy_images,
			native_image_usage,
			vk_swapchain,
		)
	}

	pub(crate) fn query_swapchain_surface_capabilities(
		&self,
		surface: vk::SurfaceKHR,
		present_mode: vk::PresentModeKHR,
	) -> vk::SurfaceCapabilitiesKHR {
		// Chaining the present mode makes the reported capabilities, such as the image counts, specific to that mode.
		let mut vk_surface_present_mode = vk::SurfacePresentModeEXT::default().present_mode(present_mode);
		let vk_surface_info = vk::PhysicalDeviceSurfaceInfo2KHR::default()
			.push(&mut vk_surface_present_mode)
			.surface(surface);
		let mut vk_surface_capabilities = vk::SurfaceCapabilities2KHR::default();

		unsafe {
			self.surface_capabilities
				.get_physical_device_surface_capabilities2(self.physical_device, &vk_surface_info, &mut vk_surface_capabilities)
				.expect(
					"Failed to query Vulkan surface capabilities. The most likely cause is that the window surface was lost.",
				)
		};

		vk_surface_capabilities.surface_capabilities
	}

	/// Uses the surface's current extent, or the clamped fallback when the platform lets the swapchain choose.
	pub(crate) fn swapchain_extent(capabilities: &vk::SurfaceCapabilitiesKHR, fallback: vk::Extent2D) -> vk::Extent2D {
		if capabilities.current_extent.width != u32::MAX && capabilities.current_extent.height != u32::MAX {
			return capabilities.current_extent;
		}

		let (min, max) = (capabilities.min_image_extent, capabilities.max_image_extent);
		vk::Extent2D::default()
			.width(fallback.width.clamp(min.width, max.width.max(min.width)))
			.height(fallback.height.clamp(min.height, max.height.max(min.height)))
	}

	/// Creates a swapchain, retiring `old_swapchain` so in-flight presentation can finish during recreation.
	pub(crate) fn create_vulkan_swapchain(
		&self,
		surface: vk::SurfaceKHR,
		present_mode: vk::PresentModeKHR,
		capabilities: &vk::SurfaceCapabilitiesKHR,
		extent: vk::Extent2D,
		image_usage: vk::ImageUsageFlags,
		old_swapchain: vk::SwapchainKHR,
	) -> vk::SwapchainKHR {
		let presentation_modes = [present_mode];
		let mut present_modes_create_info =
			vk::SwapchainPresentModesCreateInfoEXT::default().present_modes(&presentation_modes);

		let requested_image_count = match capabilities.max_image_count {
			0 => capabilities.min_image_count * 2,
			max_image_count => max_image_count,
		};
		// Per-image state lives in fixed arrays, so never ask for more images than they hold.
		let requested_image_count = requested_image_count
			.min(MAX_SWAPCHAIN_IMAGES as u32)
			.max(capabilities.min_image_count);

		let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
			.push(&mut present_modes_create_info)
			.flags(vk::SwapchainCreateFlagsKHR::DEFERRED_MEMORY_ALLOCATION_EXT)
			.surface(surface)
			.min_image_count(requested_image_count)
			.image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
			.image_format(vk::Format::B8G8R8A8_SRGB)
			.image_extent(extent)
			.image_usage(image_usage)
			.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
			.pre_transform(capabilities.current_transform)
			.composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
			.present_mode(present_mode)
			.image_array_layers(1)
			.clipped(true)
			.old_swapchain(old_swapchain);

		unsafe {
			self.swapchain.create_swapchain(&swapchain_create_info, None).expect(
				"Failed to create a Vulkan swapchain. The most likely cause is that the surface was lost or its extent is zero.",
			)
		}
	}

	#[cfg(any(debug_assertions, test))]
	pub(crate) fn has_errors(&self) -> bool {
		self.debug_data.error_count.load(std::sync::atomic::Ordering::SeqCst) > 0
	}
}
