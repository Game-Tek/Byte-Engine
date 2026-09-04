use std::sync::{
	Arc,
	atomic::{AtomicU64, Ordering},
};

use crate::ui::{Transform, Visual, layout::Sizing, style::ConcreteStyle};

static NEXT_IMAGE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
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
	pub fn from_rgba(width: u32, height: u32, pixels: impl Into<Vec<u8>>) -> Self {
		let pixels = pixels.into();
		let expected_len = width as usize * height as usize * 4;

		assert_eq!(
			pixels.len(),
			expected_len,
			"RGBA image data must contain exactly width * height * 4 bytes"
		);

		Self {
			id: NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed),
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

	pub fn size(self, sizing: Sizing) -> Self {
		Self {
			width: sizing,
			height: sizing,
			..self
		}
	}

	pub fn width(self, width: Sizing) -> Self {
		Self { width, ..self }
	}

	pub fn height(self, height: Sizing) -> Self {
		Self { height, ..self }
	}

	pub fn style(mut self, style: impl Into<ConcreteStyle>) -> Self {
		self.style = style.into();
		self
	}

	pub fn transform(mut self, transform: impl Into<Transform>) -> Self {
		self.transform = transform.into();
		self
	}

	pub fn opacity(mut self, opacity: f32) -> Self {
		self.visual.opacity = opacity;
		self
	}

	pub fn set_rgba(&mut self, width: u32, height: u32, pixels: impl Into<Vec<u8>>) {
		let pixels = pixels.into();
		let expected_len = width as usize * height as usize * 4;

		assert_eq!(
			pixels.len(),
			expected_len,
			"RGBA image data must contain exactly width * height * 4 bytes"
		);

		self.width_pixels = width;
		self.height_pixels = height;
		self.pixels = Arc::from(pixels);
		self.version = self.version.wrapping_add(1);
	}

	pub fn set_style(&mut self, style: impl Into<ConcreteStyle>) {
		self.style = style.into();
	}

	pub fn set_transform(&mut self, transform: impl Into<Transform>) {
		self.transform = transform.into();
	}

	pub fn set_opacity(&mut self, opacity: f32) {
		self.visual.opacity = opacity;
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

	/// Returns shared RGBA pixels. Clone the owner to retain this version across [`Self::set_rgba`] calls.
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
		let mut image = Image::from_rgba(1, 1, vec![255, 0, 0, 255]);
		let previous = image.pixels().clone();
		let version = image.version();

		image.set_rgba(1, 1, vec![0, 255, 0, 255]);

		assert_eq!(&*previous, &[255, 0, 0, 255]);
		assert_eq!(&**image.pixels(), &[0, 255, 0, 255]);
		assert_ne!(image.version(), version);
	}
}
