use ash::vk;

use crate::vulkan::{ImageHandle, SynchronizerHandle, MAX_FRAMES_IN_FLIGHT, MAX_SWAPCHAIN_IMAGES};
use crate::{Formats, Uses};

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
	pub format: Formats,
	pub supported_usage_flags: vk::ImageUsageFlags,
	pub acquired_image_indices: [u8; MAX_FRAMES_IN_FLIGHT],
	pub extent: vk::Extent2D,
	pub vk_present_mode: vk::PresentModeKHR,
	pub min_image_count: u32,
	pub max_image_count: u32,
}
