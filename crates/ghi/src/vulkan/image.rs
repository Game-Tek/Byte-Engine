use std::num::NonZeroU32;

use ash::vk;
use utils::Extent;

use crate::{
	vulkan::{Handle, HandleLike, Next},
	DeviceAccesses, Formats, Uses,
};

/// The `Image` struct stores Vulkan image resources and views for the GHI backend, including swapchain-backed images.
/// Swapchain-backed images keep the swapchain Vulkan handles and image views, chain across frames via `next`, and do not store extents or own the images.
#[derive(Clone)]
pub(crate) struct Image {
	pub(crate) next: Option<ImageHandle>,
	pub(crate) staging_buffer: Option<vk::Buffer>,
	pub(crate) pointer: Option<*mut u8>,
	pub(crate) image: vk::Image,
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

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct ImageHandle(pub(crate) u64);

impl Into<Handle> for ImageHandle {
	fn into(self) -> Handle {
		Handle::Image(self)
	}
}

impl HandleLike for ImageHandle {
	type Item = Image;

	fn build(value: u64) -> Self {
		ImageHandle(value)
	}

	fn access<'a>(&self, collection: &'a [Self::Item]) -> &'a Image {
		&collection[self.0 as usize]
	}
}

impl Next for Image {
	type Handle = ImageHandle;

	fn next(&self) -> Option<Self::Handle> {
		self.next
	}
}
