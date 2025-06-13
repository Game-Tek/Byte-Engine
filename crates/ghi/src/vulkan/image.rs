use ash::vk;

use crate::{vulkan::{BufferHandle, ImageHandle}, Formats, Uses};

#[derive(Clone)]
pub(crate) struct Image {
	#[cfg(debug_assertions)]
	pub(crate) name: Option<String>,
	pub(crate) next: Option<ImageHandle>,
	pub(crate) staging_buffer: Option<BufferHandle>,
	pub(crate) image: vk::Image,
	pub(crate) image_view: vk::ImageView,
	pub(crate) image_views: [vk::ImageView; 8],
	pub(crate) pointer: *const u8,
	pub(crate) extent: vk::Extent3D,
	pub(crate) format: vk::Format,
	pub(crate) format_: Formats,
	pub(crate) size: usize,
	pub(crate) uses: Uses,
	pub(crate) layers: u32,
}

unsafe impl Send for Image {}
unsafe impl Sync for Image {}