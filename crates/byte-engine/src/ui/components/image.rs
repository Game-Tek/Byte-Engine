#![expect(clippy::disallowed_types, reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md")]
use std::sync::Arc;

use crate::ui::layout::Sizing;

/// The `ImageSource` enum names where an image's texels come from, so the UI can show both bitmaps the application
/// holds and pictures the GPU draws, such as a [`crate::rendering::RenderImage`].
#[derive(Clone, Debug)]
pub enum ImageSource {
	/// RGBA8 rows the UI copies to the GPU each time they change.
	// Shared, not owned: megabytes of texels that the widget, each render, the render pass's draw list, and the
	// application all hold while the image shows. Every rebuild would otherwise copy them.
	Pixels(Arc<[u8]>),
	/// An image the UI samples where it is, every frame, without copying it. The UI shows what the image holds when
	/// the UI draws, so a render image shows the picture of the same frame.
	Gpu(ghi::ImageHandle),
}

impl ImageSource {
	/// The RGBA8 rows of a bitmap, or `None` for a GPU image.
	pub fn pixels(&self) -> Option<&[u8]> {
		match self {
			ImageSource::Pixels(pixels) => Some(pixels),
			ImageSource::Gpu(_) => None,
		}
	}
}

impl From<Vec<u8>> for ImageSource {
	fn from(pixels: Vec<u8>) -> Self {
		ImageSource::Pixels(pixels.into())
	}
}

impl From<&[u8]> for ImageSource {
	fn from(pixels: &[u8]) -> Self {
		ImageSource::Pixels(pixels.into())
	}
}

impl From<Arc<[u8]>> for ImageSource {
	fn from(pixels: Arc<[u8]>) -> Self {
		ImageSource::Pixels(pixels)
	}
}

impl From<ghi::ImageHandle> for ImageSource {
	fn from(image: ghi::ImageHandle) -> Self {
		ImageSource::Gpu(image)
	}
}

/// The `Image` struct is the retained state of a picture drawn in a box: an RGBA bitmap or a GPU image.
///
/// The engine owns every image. Declare one with [`crate::ui::ElementContext::image`] and edit it with
/// [`crate::ui::EvaluationContext::update_image`].
pub struct Image {
	id: u64,
	version: u64,
	width_pixels: u32,
	height_pixels: u32,
	source: ImageSource,
	pub width: Sizing,
	pub height: Sizing,
}

impl Image {
	/// Identifies the pixel contents without comparing them.
	pub(crate) fn content_key(&self) -> (u64, u64, u32, u32) {
		(self.id, self.version, self.width_pixels, self.height_pixels)
	}

	/// Creates an image shown at its pixel size. `id` must differ from every other image's, so render caches can
	/// tell bitmaps apart without comparing pixels. The caller checks that `pixels` matches the dimensions.
	pub(crate) fn new(id: u64, width: u32, height: u32, source: ImageSource) -> Self {
		Self {
			id,
			version: 0,
			width_pixels: width,
			height_pixels: height,
			source,
			width: Sizing::Absolute(width as f32),
			height: Sizing::Absolute(height as f32),
		}
	}

	/// Replaces the source and its pixel size, and advances the version. The caller checks that pixels match the
	/// dimensions.
	pub(crate) fn set_source(&mut self, width: u32, height: u32, source: ImageSource) {
		self.width_pixels = width;
		self.height_pixels = height;
		self.source = source;
		self.version = self.version.wrapping_add(1);
	}

	pub fn id(&self) -> u64 {
		self.id
	}

	pub fn version(&self) -> u64 {
		self.version
	}

	pub fn width_pixels(&self) -> u32 {
		self.width_pixels
	}

	pub fn height_pixels(&self) -> u32 {
		self.height_pixels
	}

	/// Returns where the texels come from. Clone shared RGBA pixels to retain this version across
	/// [`crate::ui::Properties::pixels`] edits.
	pub fn source(&self) -> &ImageSource {
		&self.source
	}
}

#[cfg(test)]
mod tests {
	use super::Image;

	#[test]
	fn retained_pixels_preserve_their_version_after_replacement() {
		let mut image = Image::new(1, 1, 1, vec![255, 0, 0, 255].into());
		let previous = image.source().clone();
		let version = image.version();

		image.set_source(1, 1, vec![0, 255, 0, 255].into());

		assert_eq!(previous.pixels(), Some(&[255, 0, 0, 255][..]));
		assert_eq!(image.source().pixels(), Some(&[0, 255, 0, 255][..]));
		assert_ne!(image.version(), version);
	}
}
