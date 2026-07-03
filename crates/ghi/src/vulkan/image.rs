use std::num::NonZeroU32;

use ash::vk;
use utils::Extent;

use crate::{image::ImageHandle, DeviceAccesses, Formats, HandleLike, Next, Uses};

/// The `Image` struct stores Vulkan image resources and views for the GHI backend, including swapchain-backed images.
/// Swapchain-backed images keep the swapchain Vulkan handles and image views, chain across frames via `next`, and do not store extents or own the images.
#[derive(Clone)]
pub(crate) struct Image {
	pub(crate) next: Option<ImageHandle>,
	pub(crate) staging_buffer: Option<vk::Buffer>,
	pub(crate) pointer: Option<*mut u8>,
	pub(crate) image: vk::Image,
	pub(crate) full_image_view: vk::ImageView,
	pub(crate) image_views: [vk::ImageView; 8],
	pub(crate) extent: Extent,
	pub(crate) format: vk::Format,
	pub(crate) format_: Formats,
	pub(crate) access: DeviceAccesses,
	pub(crate) size: usize,
	pub(crate) uses: Uses,
	pub(crate) layers: Option<NonZeroU32>,
	pub(crate) owns_image: bool,
}

impl Next for Image {
	type Handle = ImageHandle;

	fn next(&self) -> Option<Self::Handle> {
		self.next
	}
}

impl HandleLike for ImageHandle {
	type Item = Image;

	fn build(value: u64) -> Self {
		Self(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Self::Item {
		&collection[self.0 as usize]
	}
}
