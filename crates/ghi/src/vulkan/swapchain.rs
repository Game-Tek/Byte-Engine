use ash::vk;

use crate::{vulkan::{ImageHandle, MAX_FRAMES_IN_FLIGHT, MAX_SWAPCHAIN_IMAGES, SynchronizerHandle}};

#[derive(Clone)]
pub(crate) struct Swapchain {
	pub surface: vk::SurfaceKHR,
	pub swapchain: vk::SwapchainKHR,
	pub acquire_synchronizers: [SynchronizerHandle; MAX_FRAMES_IN_FLIGHT],
	pub submit_synchronizers: [SynchronizerHandle; MAX_SWAPCHAIN_IMAGES],
	pub images: [ImageHandle; MAX_SWAPCHAIN_IMAGES],
	pub extent: vk::Extent2D,
	pub sync_stage: vk::PipelineStageFlags2,
	pub vk_present_mode: vk::PresentModeKHR,
	pub min_image_count: u32,
	pub max_image_count: u32,
}
