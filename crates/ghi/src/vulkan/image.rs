use std::num::NonZeroU32;

use ash::vk;
use utils::Extent;

use crate::{DeviceAccesses, Formats, HandleLike, Next, Uses, image::ImageHandle};

/// The `Image` struct provides Vulkan resources and views for GHI images.
///
/// Swapchain-backed images keep native handles and image views. They chain across
/// frames through `next`, do not own the images, and do not keep their extents.
#[derive(Clone)]
pub(crate) struct Image {
	pub(crate) next: Option<ImageHandle>,
	pub(crate) staging_buffer: Option<vk::Buffer>,
	pub(crate) staging_allocation: Option<crate::AllocationHandle>,
	/// Dedicated memory backing `image`, released together with it when the image is replaced.
	pub(crate) allocation: Option<crate::AllocationHandle>,
	pub(crate) pointer: Option<crate::vulkan::MappedMemoryPointer>,
	pub(crate) image: vk::Image,
	pub(crate) full_image_view: vk::ImageView,
	pub(crate) image_views: Vec<vk::ImageView>,
	pub(crate) extent: Extent,
	pub(crate) format: vk::Format,
	pub(crate) format_: Formats,
	pub(crate) access: DeviceAccesses,
	pub(crate) size: usize,
	pub(crate) uses: Uses,
	pub(crate) layers: Option<NonZeroU32>,
	pub(crate) cube_compatible: bool,
	pub(crate) cube_array_compatible: bool,
	pub(crate) mip_levels: u32,
	pub(crate) owns_image: bool,
}

impl Image {
	/// Describes this image, so a resize or a new frame copy rebuilds it with the same parameters.
	pub(crate) fn builder(&self) -> crate::image::Builder<'static> {
		crate::image::Builder {
			extent: self.extent,
			device_accesses: self.access,
			mip_levels: self.mip_levels,
			array_layers: self.layers,
			cube_compatible: self.cube_compatible,
			cube_array_compatible: self.cube_array_compatible,
			..crate::image::Builder::new(self.format_, self.uses)
		}
	}
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
