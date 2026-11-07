use super::*;
use crate::vulkan::swapchain::DisplayFeedback;

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
				format.format == vk::Format::B8G8R8A8_UNORM && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
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

	/// Returns what a swapchain on `surface` with `present_mode` learns from the display, from what the device and
	/// the surface support.
	///
	/// Only FIFO modes wait for presents: they show every frame, so waiting for one bounds how far ahead of the display
	/// the CPU runs. Mailbox and immediate presentation exist to run ahead of it. Display times name presents by their
	/// identifiers, so only swapchains that wait report them.
	pub(crate) fn display_feedback(&self, surface: vk::SurfaceKHR, present_mode: vk::PresentModeKHR) -> DisplayFeedback {
		if self.present_wait.is_none() || !matches!(present_mode, vk::PresentModeKHR::FIFO | vk::PresentModeKHR::FIFO_RELAXED) {
			return DisplayFeedback::default();
		}

		let vk_surface_info = vk::PhysicalDeviceSurfaceInfo2KHR::default().surface(surface);
		let mut present_id = vk::SurfaceCapabilitiesPresentId2KHR::default();
		let mut present_wait = vk::SurfaceCapabilitiesPresentWait2KHR::default();
		let mut present_timing = vk::PresentTimingSurfaceCapabilitiesEXT::default();
		let mut vk_surface_capabilities = vk::SurfaceCapabilities2KHR::default()
			.push(&mut present_id)
			.push(&mut present_wait);
		// The timing structure may only be chained when its extension is enabled.
		if self.present_timing.is_some() {
			vk_surface_capabilities = vk_surface_capabilities.push(&mut present_timing);
		}
		unsafe {
			self.surface_capabilities
				.get_physical_device_surface_capabilities2(self.physical_device, &vk_surface_info, &mut vk_surface_capabilities)
				.expect(
					"Failed to query Vulkan surface capabilities. The most likely cause is that the window surface was lost.",
				)
		};
		let waits_for_presents =
			present_id.present_id2_supported == vk::TRUE && present_wait.present_wait2_supported == vk::TRUE;
		DisplayFeedback {
			waits_for_presents,
			reports_present_times: waits_for_presents
				&& present_timing.present_timing_supported == vk::TRUE
				&& present_timing
					.present_stage_queries
					.contains(vk::PresentStageFlagsEXT::IMAGE_FIRST_PIXEL_OUT),
		}
	}

	/// Prepares a swapchain created with present timing to report display times, and returns the time domain its
	/// reports use: `CLOCK_MONOTONIC`, which [`std::time::Instant`] reads, when the swapchain offers it, otherwise its
	/// own present-stage clock, which [`Self::newest_present_time`] calibrates against it.
	pub(crate) fn configure_present_timing(&self, swapchain: vk::SwapchainKHR) -> Option<(vk::TimeDomainKHR, u64)> {
		let present_timing = self.present_timing.as_ref()?;
		let functions = present_timing.timing.fp();
		let device = self.device.handle();

		// Each present queues one timing until acquisition reads it; the queue holds a few frames more than are in
		// flight, so a late read never fills it.
		let result =
			unsafe { (functions.set_swapchain_present_timing_queue_size_ext)(device, swapchain, PRESENT_TIMING_QUEUE_SIZE) };
		if result != vk::Result::SUCCESS {
			return None;
		}

		let mut domains = [vk::TimeDomainKHR::default(); 8];
		let mut domain_ids = [0u64; 8];
		let mut properties = vk::SwapchainTimeDomainPropertiesEXT {
			time_domain_count: domains.len() as u32,
			p_time_domains: domains.as_mut_ptr(),
			p_time_domain_ids: domain_ids.as_mut_ptr(),
			..Default::default()
		};
		let mut counter = 0;
		let result =
			unsafe { (functions.get_swapchain_time_domain_properties_ext)(device, swapchain, &mut properties, &mut counter) };
		if !matches!(result, vk::Result::SUCCESS | vk::Result::INCOMPLETE) {
			return None;
		}
		let count = (properties.time_domain_count as usize).min(domains.len());
		let find = |wanted| domains[..count].iter().position(|&domain| domain == wanted);
		find(vk::TimeDomainKHR::CLOCK_MONOTONIC)
			.or_else(|| find(vk::TimeDomainKHR::PRESENT_STAGE_LOCAL_EXT))
			.map(|index| (domains[index], domain_ids[index]))
	}

	/// Returns the identifier of the newest of `swapchain`'s completed presents and the `CLOCK_MONOTONIC` time at
	/// which it put its first pixel out, reading every completed timing so the queue empties. Returns `None` when none
	/// completed since the last call, or when a time in the swapchain's own clock couldn't be calibrated.
	pub(crate) fn newest_present_time(
		&self,
		swapchain: vk::SwapchainKHR,
		(domain, time_domain): (vk::TimeDomainKHR, u64),
	) -> Option<(u64, u64)> {
		let present_timing = self.present_timing.as_ref()?;
		// One stage per present, held in fixed arrays so acquisition doesn't allocate.
		let mut stages = [vk::PresentStageTimeEXT::default(); PRESENT_TIMING_QUEUE_SIZE as usize];
		let mut timings = [vk::PastPresentationTimingEXT::default(); PRESENT_TIMING_QUEUE_SIZE as usize];
		for (timing, stage) in timings.iter_mut().zip(&mut stages) {
			timing.present_stage_count = 1;
			timing.p_present_stages = stage;
		}
		let info = vk::PastPresentationTimingInfoEXT::default().swapchain(swapchain);
		let mut properties = vk::PastPresentationTimingPropertiesEXT {
			presentation_timing_count: timings.len() as u32,
			p_presentation_timings: timings.as_mut_ptr(),
			..Default::default()
		};
		let result = unsafe {
			(present_timing.timing.fp().get_past_presentation_timing_ext)(self.device.handle(), &info, &mut properties)
		};
		if !matches!(result, vk::Result::SUCCESS | vk::Result::INCOMPLETE) {
			return None;
		}
		let count = (properties.presentation_timing_count as usize).min(timings.len());
		let newest = timings[..count]
			.iter()
			.zip(&stages)
			.filter(|(timing, stage)| {
				timing.report_complete == vk::TRUE
					&& timing.time_domain_id == time_domain
					&& stage.stage == vk::PresentStageFlagsEXT::IMAGE_FIRST_PIXEL_OUT
					&& stage.time != 0
			})
			.map(|(timing, stage)| (timing.present_id, stage.time))
			.max_by_key(|&(present_id, _)| present_id)?;
		let (present_id, newest) = newest;
		if domain == vk::TimeDomainKHR::CLOCK_MONOTONIC {
			return Some((present_id, newest));
		}

		// Sample the swapchain's clock and `CLOCK_MONOTONIC` together, then move the time by their offset.
		let mut swapchain_clock = vk::SwapchainCalibratedTimestampInfoEXT::default()
			.swapchain(swapchain)
			.present_stage(vk::PresentStageFlagsEXT::IMAGE_FIRST_PIXEL_OUT)
			.time_domain_id(time_domain);
		let infos = [
			vk::CalibratedTimestampInfoKHR::default()
				.time_domain(domain)
				.push(&mut swapchain_clock),
			vk::CalibratedTimestampInfoKHR::default().time_domain(vk::TimeDomainKHR::CLOCK_MONOTONIC),
		];
		let mut timestamps = [0u64; 2];
		let mut max_deviation = 0;
		let result = unsafe {
			(present_timing.calibration.fp().get_calibrated_timestamps_khr)(
				self.device.handle(),
				infos.len() as u32,
				infos.as_ptr(),
				timestamps.as_mut_ptr(),
				&mut max_deviation,
			)
		};
		if result != vk::Result::SUCCESS {
			return None;
		}
		let [swapchain_now, monotonic_now] = timestamps;
		Some((present_id, monotonic_now.wrapping_add(newest).wrapping_sub(swapchain_now)))
	}

	/// Returns how long one refresh of the display `swapchain` presents to lasts, or `None` while the presentation
	/// engine doesn't know or the refresh rate varies.
	pub(crate) fn refresh_duration(&self, swapchain: vk::SwapchainKHR) -> Option<std::time::Duration> {
		let present_timing = self.present_timing.as_ref()?;
		let mut properties = vk::SwapchainTimingPropertiesEXT::default();
		let mut counter = 0;
		let result = unsafe {
			(present_timing.timing.fp().get_swapchain_timing_properties_ext)(
				self.device.handle(),
				swapchain,
				&mut properties,
				&mut counter,
			)
		};
		(result == vk::Result::SUCCESS && properties.refresh_duration != 0)
			.then(|| std::time::Duration::from_nanos(properties.refresh_duration))
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
	///
	/// `feedback` decides whether it accepts present identifiers and timing requests; see [`Self::display_feedback`].
	pub(crate) fn create_vulkan_swapchain(
		&self,
		surface: vk::SurfaceKHR,
		present_mode: vk::PresentModeKHR,
		feedback: DisplayFeedback,
		capabilities: &vk::SurfaceCapabilitiesKHR,
		extent: vk::Extent2D,
		image_usage: vk::ImageUsageFlags,
		desired_image_count: Option<u8>,
		old_swapchain: vk::SwapchainKHR,
	) -> vk::SwapchainKHR {
		let presentation_modes = [present_mode];
		let mut present_modes_create_info =
			vk::SwapchainPresentModesCreateInfoEXT::default().present_modes(&presentation_modes);

		// Three images let the GPU render one while the display shows one and one waits to be shown. More only hold
		// frames in a queue, since acquisition already waits for the display; see `Context::wait_for_display`.
		const DEFAULT_IMAGE_COUNT: u32 = 3;
		let requested_image_count = match (desired_image_count, capabilities.max_image_count) {
			(Some(count), 0) => u32::from(count),
			(Some(count), max_image_count) => u32::from(count).min(max_image_count),
			(None, 0) => DEFAULT_IMAGE_COUNT,
			(None, max_image_count) => DEFAULT_IMAGE_COUNT.min(max_image_count),
		};
		// Per-image state lives in fixed arrays, so never ask for more images than they hold.
		let requested_image_count = requested_image_count
			.min(MAX_SWAPCHAIN_IMAGES as u32)
			.max(capabilities.min_image_count);

		let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
			.push(&mut present_modes_create_info)
			.flags(
				vk::SwapchainCreateFlagsKHR::DEFERRED_MEMORY_ALLOCATION_EXT
					| flag_if(
						feedback.waits_for_presents,
						vk::SwapchainCreateFlagsKHR::PRESENT_ID_2 | vk::SwapchainCreateFlagsKHR::PRESENT_WAIT_2,
					) | flag_if(
					feedback.reports_present_times,
					vk::SwapchainCreateFlagsKHR::PRESENT_TIMING_EXT,
				),
			)
			.surface(surface)
			.min_image_count(requested_image_count)
			.image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
			.image_format(vk::Format::B8G8R8A8_UNORM)
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

/// How many presents' timings a swapchain queues until acquisition reads them.
const PRESENT_TIMING_QUEUE_SIZE: u32 = 8;

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
