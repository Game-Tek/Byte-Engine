use super::*;

impl InnerDevice {
	fn create_vulkan_surface(&self, window_os_handles: &window::Handles) -> vk::SurfaceKHR {
		let surface = {
			#[cfg(target_os = "linux")]
			{
				let wayland_surface_create_info = vk::WaylandSurfaceCreateInfoKHR::default()
					.display(window_os_handles.display)
					.surface(window_os_handles.surface);

				unsafe {
					self.wayland_surface
						.create_wayland_surface(&wayland_surface_create_info, None)
						.expect("No surface")
				}
			}
			#[cfg(target_os = "windows")]
			{
				let win32_surface_create_info = vk::Win32SurfaceCreateInfoKHR::default()
					.hinstance(window_os_handles.hinstance.0 as isize)
					.hwnd(window_os_handles.hwnd.0 as isize);

				unsafe {
					self.win32_surface
						.create_win32_surface(&win32_surface_create_info, None)
						.expect("No surface")
				}
			}
			#[cfg(target_os = "macos")]
			{
				let metal_layer = objc2_quartz_core::CAMetalLayer::new();

				let view = &window_os_handles.view;
				let logical_size = view.frame().size;
				let drawable_size = view.convertSizeToBacking(logical_size);
				let scale_factor = if logical_size.width > 0.0 {
					(drawable_size.width / logical_size.width).max(1.0)
				} else if logical_size.height > 0.0 {
					(drawable_size.height / logical_size.height).max(1.0)
				} else {
					1.0
				};

				view.setWantsLayer(true);
				view.setLayer(Some(&metal_layer));
				metal_layer.setContentsScale(scale_factor);
				metal_layer.setDrawableSize(drawable_size);

				let macos_surface_create_info =
					vk::MetalSurfaceCreateInfoEXT::default().layer(objc2::rc::Retained::as_ptr(&metal_layer) as _);

				unsafe {
					self.macos_surface
						.create_metal_surface(&macos_surface_create_info, None)
						.expect("No surface")
				}
			}
		};

		let surface_capabilities = unsafe {
			self.surface
				.get_physical_device_surface_capabilities(self.physical_device, surface)
				.expect("No surface capabilities")
		};

		let surface_format = unsafe {
			self.surface
				.get_physical_device_surface_formats(self.physical_device, surface)
				.expect("No surface formats")
		};

		let _: vk::SurfaceFormatKHR = surface_format
			.iter()
			.find(|format| {
				format.format == vk::Format::B8G8R8A8_SRGB && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
			})
			.expect("No surface format")
			.to_owned();

		let surface_present_modes = unsafe {
			self.surface
				.get_physical_device_surface_present_modes(self.physical_device, surface)
				.expect("No surface present modes")
		};

		let _: vk::PresentModeKHR = surface_present_modes
			.iter()
			.find(|present_mode| **present_mode == vk::PresentModeKHR::FIFO)
			.expect("No surface present mode")
			.to_owned();

		let _surface_resolution = surface_capabilities.current_extent;

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
		crate::Formats,
		vk::ImageUsageFlags,
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

		let mut vk_surface_present_mode = vk::SurfacePresentModeEXT::default().present_mode(vk_present_mode);

		let vk_surface_info = vk::PhysicalDeviceSurfaceInfo2KHR::default()
			.push(&mut vk_surface_present_mode)
			.surface(vk_surface);

		let mut vk_presentation_modes = [vk::PresentModeKHR::default(); 8];

		let mut vk_surface_present_mode_compatibility =
			vk::SurfacePresentModeCompatibilityEXT::default().present_modes(&mut vk_presentation_modes);

		let mut vk_surface_capabilities =
			vk::SurfaceCapabilities2KHR::default().push(&mut vk_surface_present_mode_compatibility);

		unsafe {
			self.surface_capabilities
				.get_physical_device_surface_capabilities2(self.physical_device, &vk_surface_info, &mut vk_surface_capabilities)
				.expect("No surface capabilities")
		};

		let vk_surface_capabilities = vk_surface_capabilities.surface_capabilities;

		let min_image_count = vk_surface_capabilities.min_image_count;
		let max_image_count = vk_surface_capabilities.max_image_count;

		let extent = if vk_surface_capabilities.current_extent.width != u32::MAX
			&& vk_surface_capabilities.current_extent.height != u32::MAX
		{
			vk_surface_capabilities.current_extent
		} else {
			vk::Extent2D::default()
				.width(fallback_extent.width())
				.height(fallback_extent.height())
		};

		let presentation_modes = [vk_present_mode];

		let mut present_modes_create_info =
			vk::SwapchainPresentModesCreateInfoEXT::default().present_modes(&presentation_modes);

		let requested_image_count = if max_image_count != 0 {
			max_image_count.max(min_image_count)
		} else {
			(min_image_count * 2).min(MAX_SWAPCHAIN_IMAGES as u32)
		};

		let format = crate::Formats::BGRAsRGB;
		let proxy_format = crate::Formats::BGRAu8;

		let requested_image_usage = into_vk_image_usage_flags(uses, format);
		let supported_image_usage = vk_surface_capabilities.supported_usage_flags;
		let uses_proxy_images = self.swapchain_needs_proxy(supported_image_usage, requested_image_usage, uses);

		let native_image_usage = if uses_proxy_images {
			self.validate_swapchain_proxy_format(uses);

			let fallback_usage = vk::ImageUsageFlags::TRANSFER_DST;

			if !supported_image_usage.contains(fallback_usage) {
				panic!(
					"Failed to create swapchain fallback copy path. The most likely cause is that the surface does not support transfer destination usage for swapchain images."
				);
			}

			fallback_usage
		} else {
			requested_image_usage
		};

		let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
			.push(&mut present_modes_create_info)
			.flags(vk::SwapchainCreateFlagsKHR::DEFERRED_MEMORY_ALLOCATION_EXT)
			.surface(vk_surface)
			.min_image_count(requested_image_count)
			.image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
			.image_format(vk::Format::B8G8R8A8_SRGB)
			.image_extent(extent)
			.image_usage(native_image_usage)
			.image_sharing_mode(vk::SharingMode::EXCLUSIVE)
			.pre_transform(vk_surface_capabilities.current_transform)
			.composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
			.present_mode(vk_present_mode)
			.image_array_layers(1)
			.clipped(true);

		let vk_swapchain = unsafe {
			self.swapchain
				.create_swapchain(&swapchain_create_info, None)
				.expect("No swapchain")
		};
		(
			vk_surface,
			vk_present_mode,
			min_image_count,
			extent,
			format,
			proxy_format,
			supported_image_usage,
			uses_proxy_images,
			native_image_usage,
			vk_swapchain,
		)
	}

	fn swapchain_needs_proxy(
		&self,
		supported_usage_flags: vk::ImageUsageFlags,
		requested_usage_flags: vk::ImageUsageFlags,
		uses: crate::Uses,
	) -> bool {
		!supported_usage_flags.contains(requested_usage_flags)
			|| uses.contains(crate::Uses::Storage) && !self.swapchain_native_supports_formatless_storage_write
	}

	fn validate_swapchain_proxy_format(&self, uses: crate::Uses) {
		if uses.contains(crate::Uses::Storage) && !self.swapchain_proxy_supports_formatless_storage_write {
			panic!(
				"Failed to create swapchain storage proxy image. The most likely cause is that the selected Vulkan device does not support storage writes without format for the swapchain proxy format."
			);
		}
	}

	#[cfg(any(debug_assertions, test))]
	fn get_log_count(&self) -> u64 {
		use std::sync::atomic::Ordering;
		unsafe { &(*self.debug_data) }.error_count.load(Ordering::SeqCst)
	}

	#[cfg(any(debug_assertions, test))]
	pub(crate) fn has_errors(&self) -> bool {
		self.get_log_count() > 0
	}
}
