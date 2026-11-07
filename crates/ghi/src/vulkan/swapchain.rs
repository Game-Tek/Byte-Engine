use ash::vk;

use crate::Uses;
use crate::synchronizer::SynchronizerHandle;
use crate::vulkan::{ImageHandle, MAX_FRAMES_IN_FLIGHT, MAX_SWAPCHAIN_IMAGES};

/// The format of every swapchain image.
///
/// The engine encodes display color itself, in its `srgb-display` and tone-mapping passes, so the swapchain stores
/// those bytes as they are. An sRGB format would encode them a second time on write and wash the picture out. Metal and
/// DX12 present the same UNORM format.
pub(crate) const SWAPCHAIN_FORMAT: crate::Formats = crate::Formats::BGRAu8;

/// The `DisplayFeedback` struct records what a swapchain learns from the display, so creation, presentation, and
/// acquisition agree on it; [`crate::vulkan::device::InnerDevice::display_feedback`] decides it.
#[derive(Clone, Copy, Default)]
pub(crate) struct DisplayFeedback {
	/// Presents carry identifiers, and acquisition waits until the display shows the frame presented the frames in
	/// flight before it.
	pub waits_for_presents: bool,
	/// Presents ask when their first pixel goes out, and acquisition reads the answers.
	pub reports_present_times: bool,
}

#[derive(Clone)]
pub(crate) struct Swapchain {
	pub surface: vk::SurfaceKHR,
	pub swapchain: vk::SwapchainKHR,
	pub acquire_synchronizers: [SynchronizerHandle; MAX_FRAMES_IN_FLIGHT],
	pub submit_synchronizers: [SynchronizerHandle; MAX_SWAPCHAIN_IMAGES],
	/// User-facing swapchain images.
	/// These are native swapchain images when compatible with the requested usages,
	/// otherwise these are proxy images that are copied into `native_images` before present.
	pub images: [ImageHandle; MAX_SWAPCHAIN_IMAGES],
	/// Native presentable swapchain images from Vulkan.
	pub native_images: [ImageHandle; MAX_SWAPCHAIN_IMAGES],
	/// Indicates whether `images` are proxy images.
	pub uses_proxy_images: bool,
	/// Uses requested when binding the window, preserved so recreation builds equivalent images.
	pub uses: Uses,
	pub native_image_usage: vk::ImageUsageFlags,
	/// Set when presentation or acquisition reports that the swapchain no longer matches its surface.
	pub needs_recreation: bool,
	pub acquired_image_indices: [u8; MAX_FRAMES_IN_FLIGHT],
	/// Stages of the first access to each sequence's acquired image, where the submission waits on the acquire semaphore.
	pub acquire_wait_stages: [vk::PipelineStageFlags2; MAX_FRAMES_IN_FLIGHT],
	pub extent: vk::Extent2D,
	pub vk_present_mode: vk::PresentModeKHR,
	pub min_image_count: u32,
	pub max_image_count: u32,
	/// The image count requested when binding the window, preserved so recreation asks for the same count.
	pub desired_image_count: Option<u8>,
	/// The minimum time between presented frames; `None` presents on the next refresh.
	pub present_interval: Option<std::time::Duration>,
	/// The earliest time the next acquisition may start when `present_interval` is set.
	pub next_present_slot: Option<std::time::Instant>,
	/// What the swapchain learns from the display, decided once for its surface.
	pub feedback: DisplayFeedback,
	/// The identifier of the most recent present of the current `swapchain`, or 0 before its first present.
	pub present_id: u64,
	/// The time domain, and its identifier, that the current `swapchain` reports display times in, when it reports
	/// them.
	pub present_time_domain: Option<(vk::TimeDomainKHR, u64)>,
	/// The identifier of the newest present the display showed, and when, as its newest completed timing reported.
	pub shown_present: Option<(u64, std::time::Instant)>,
	/// The window's size in pixels, which the swapchain takes when the surface reports none, as Wayland surfaces don't.
	pub window_extent: crate::window::os::wayland::WindowExtent,
}

impl Swapchain {
	/// Returns the extent the swapchain should have for `capabilities`: the surface's own size when it reports one,
	/// otherwise the window's, or the current extent before the window knows its size.
	pub fn target_extent(&self, capabilities: &vk::SurfaceCapabilitiesKHR) -> vk::Extent2D {
		let fallback = self.window_extent.get().map_or(self.extent, |extent| {
			vk::Extent2D::default().width(extent.width()).height(extent.height())
		});
		crate::vulkan::device::InnerDevice::swapchain_extent(capabilities, fallback)
	}
}
