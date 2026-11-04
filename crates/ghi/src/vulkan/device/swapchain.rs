use super::*;

impl InnerDevice {
	pub(crate) fn create_vulkan_surface(&self, window_os_handles: &window::Handles) -> vk::SurfaceKHR {
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

	/// Returns the present mode a swapchain on `surface` uses for `requested`: the requested mode when the surface
	/// offers it, otherwise the closest one it offers. See [`resolve_present_mode`].
	pub(crate) fn surface_present_mode(
		&self,
		surface: vk::SurfaceKHR,
		requested: graphics_hardware_interface::PresentationModes,
	) -> vk::PresentModeKHR {
		let supported = unsafe {
			self.surface
				.get_physical_device_surface_present_modes(self.physical_device, surface)
		}
		.expect("Failed to query Vulkan surface present modes. The most likely cause is that the window surface was lost.");
		resolve_present_mode(requested, &supported)
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
		desired_image_count: Option<u8>,
		old_swapchain: vk::SwapchainKHR,
	) -> vk::SwapchainKHR {
		let presentation_modes = [present_mode];
		let mut present_modes_create_info =
			vk::SwapchainPresentModesCreateInfoEXT::default().present_modes(&presentation_modes);

		let requested_image_count = match (desired_image_count, capabilities.max_image_count) {
			(Some(count), 0) => u32::from(count),
			(Some(count), max_image_count) => u32::from(count).min(max_image_count),
			(None, 0) => capabilities.min_image_count * 2,
			(None, max_image_count) => max_image_count,
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

	pub(crate) fn has_errors(&self) -> bool {
		self.debug_data.get().error_count.load(std::sync::atomic::Ordering::SeqCst) > 0
	}
}

/// Picks the present mode for `requested` among the modes a surface offers in `supported`.
///
/// Immediate presentation never waits for the display; when the surface lacks it, as Wayland compositors usually do,
/// mailbox also never blocks the caller, and FIFO, which every surface offers, is the last resort. Mailbox falls back to
/// FIFO rather than to immediate, so it never tears.
fn resolve_present_mode(
	requested: graphics_hardware_interface::PresentationModes,
	supported: &[vk::PresentModeKHR],
) -> vk::PresentModeKHR {
	use graphics_hardware_interface::PresentationModes;

	let preferences: &[vk::PresentModeKHR] = match requested {
		PresentationModes::Inmediate => &[vk::PresentModeKHR::IMMEDIATE, vk::PresentModeKHR::MAILBOX],
		PresentationModes::Mailbox => &[vk::PresentModeKHR::MAILBOX],
		PresentationModes::FIFO => &[],
	};
	preferences
		.iter()
		.copied()
		.find(|mode| supported.contains(mode))
		.unwrap_or(vk::PresentModeKHR::FIFO)
}

#[cfg(test)]
mod tests {
	use super::*;
	use graphics_hardware_interface::PresentationModes;

	/// Verifies a surface without immediate presentation still never blocks the caller when it offers mailbox.
	#[test]
	fn immediate_falls_back_to_mailbox_then_fifo() {
		let wayland = [vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::FIFO];
		assert_eq!(resolve_present_mode(PresentationModes::Inmediate, &wayland), vk::PresentModeKHR::MAILBOX);
		assert_eq!(
			resolve_present_mode(PresentationModes::Inmediate, &[vk::PresentModeKHR::FIFO]),
			vk::PresentModeKHR::FIFO
		);
		let all = [vk::PresentModeKHR::IMMEDIATE, vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::FIFO];
		assert_eq!(resolve_present_mode(PresentationModes::Inmediate, &all), vk::PresentModeKHR::IMMEDIATE);
	}

	/// Verifies mailbox falls back to FIFO instead of a mode that tears.
	#[test]
	fn mailbox_never_falls_back_to_immediate() {
		let tearing = [vk::PresentModeKHR::IMMEDIATE, vk::PresentModeKHR::FIFO];
		assert_eq!(resolve_present_mode(PresentationModes::Mailbox, &tearing), vk::PresentModeKHR::FIFO);
	}
}
