use std::sync::Arc;

use crate::ui::{Transform, Visual, layout::Sizing, style::ConcreteStyle};

/// The `Image` struct is the retained state of an RGBA bitmap drawn in a box.
///
/// The engine owns every image. Declare one with [`crate::ui::ElementContext::image`] and edit it with
/// [`crate::ui::EvaluationContext::update_image`].
pub struct Image {
	id: u64,
	version: u64,
	width_pixels: u32,
	height_pixels: u32,
	pixels: Arc<[u8]>,
	pub width: Sizing,
	pub height: Sizing,
	pub(crate) style: ConcreteStyle,
	pub(crate) transform: Transform,
	pub(crate) visual: Visual,
}

impl Image {
	/// Identifies the pixel contents without comparing them.
	pub(crate) fn content_key(&self) -> (u64, u64, u32, u32) {
		(self.id, self.version, self.width_pixels, self.height_pixels)
	}

	/// Creates an image shown at its pixel size. `id` must differ from every other image's, so render caches can
	/// tell bitmaps apart without comparing pixels. The caller checks that `pixels` matches the dimensions.
	pub(crate) fn new(id: u64, width: u32, height: u32, pixels: &[u8]) -> Self {
		Self {
			id,
			version: 0,
			width_pixels: width,
			height_pixels: height,
			pixels: Arc::from(pixels),
			width: Sizing::Absolute(width as f32),
			height: Sizing::Absolute(height as f32),
			style: ConcreteStyle::default(),
			transform: Transform::default(),
			visual: Visual::default(),
		}
	}

	/// Replaces the pixels and advances the version. The caller checks that `pixels` matches the dimensions.
	pub(crate) fn set_rgba(&mut self, width: u32, height: u32, pixels: &[u8]) {
		self.width_pixels = width;
		self.height_pixels = height;
		self.pixels = Arc::from(pixels);
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

	/// Returns shared RGBA pixels. Clone the owner to retain this version across [`crate::ui::Properties::pixels`]
	/// edits.
	pub fn pixels(&self) -> &Arc<[u8]> {
		&self.pixels
	}

	pub fn style_ref(&self) -> &ConcreteStyle {
		&self.style
	}

	pub fn transform_ref(&self) -> &Transform {
		&self.transform
	}

	pub fn visual_ref(&self) -> &Visual {
		&self.visual
	}
}

#[cfg(test)]
mod tests {
	use super::Image;

	#[test]
	fn retained_pixels_preserve_their_version_after_replacement() {
		let mut image = Image::new(1, 1, 1, &[255, 0, 0, 255]);
		let previous = image.pixels().clone();
		let version = image.version();

		image.set_rgba(1, 1, &[0, 255, 0, 255]);

		assert_eq!(&*previous, &[255, 0, 0, 255]);
		assert_eq!(&**image.pixels(), &[0, 255, 0, 255]);
		assert_ne!(image.version(), version);
	}
}
