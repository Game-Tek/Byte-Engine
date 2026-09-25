use ash::vk;

use crate::Uses;
use crate::synchronizer::SynchronizerHandle;
use crate::vulkan::{ImageHandle, MAX_FRAMES_IN_FLIGHT, MAX_SWAPCHAIN_IMAGES};

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
	pub proxy_uses: Uses,
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
	/// The minimum time between presented frames; `None` presents on the next refresh.
	pub present_interval: Option<std::time::Duration>,
	/// The earliest time the next acquisition may start when `present_interval` is set.
	pub next_present_slot: Option<std::time::Instant>,
}
