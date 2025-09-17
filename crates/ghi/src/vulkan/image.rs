use std::num::NonZeroU32;

use ash::vk;
use utils::Extent;

use crate::{vulkan::{ImageHandle, Next}, DeviceAccesses, Formats, Uses};

#[derive(Clone)]
pub(crate) struct Image {
	pub(crate) next: Option<ImageHandle>,
	pub(crate) staging_buffer: Option<vk::Buffer>,
	pub(crate) pointer: Option<*mut u8>,
	pub(crate) image: vk::Image,
	pub(crate) image_view: vk::ImageView,
	pub(crate) image_views: [vk::ImageView; 8],
	pub(crate) extent: Extent,
	pub(crate) format: vk::Format,
	pub(crate) format_: Formats,
	pub(crate) access: DeviceAccesses,
	pub(crate) size: usize,
	pub(crate) uses: Uses,
	pub(crate) layers: Option<NonZeroU32>,
}

unsafe impl Send for Image {}
unsafe impl Sync for Image {}

impl Next for Image {
	type Handle = ImageHandle;

	fn next(&self) -> Option<Self::Handle> {
		self.next
	}
}
