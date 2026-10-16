//! Reusable post-scene rendering passes.
//!
//! Tone mapping, bloom, lens flare, motion blur, LUT processing, sky rendering, height fog, and utility passes in
//! this module implement [`crate::rendering::RenderPass`]. Install them through
//! [`crate::rendering::renderer::Renderer`] or the corresponding helpers in
//! [`crate::application::graphics`].

pub mod blit;
pub(crate) mod debug_view;

pub mod aces;
pub mod agx;
pub mod bloom;
pub mod height_fog;
pub mod lens_flare;
pub mod lut;
pub mod motion_blur;
pub mod sky;
pub mod smaa;
pub mod srgb_display;
mod sun;

pub mod image_transform;

/// Returns the matrix that takes a pixel's integer coordinate to the normalized device coordinates of its center,
/// whose y points up, for an image of `extent`. The z and w components pass through.
pub(crate) fn pixel_to_ndc(extent: utils::Extent) -> math::Matrix {
	use maths_rs::Vec4f;

	let (width, height) = (extent.width() as f32, extent.height() as f32);
	math::Matrix::from((
		Vec4f::new(2.0 / width, 0.0, 0.0, 1.0 / width - 1.0),
		Vec4f::new(0.0, -2.0 / height, 0.0, 1.0 - 1.0 / height),
		Vec4f::new(0.0, 0.0, 1.0, 0.0),
		Vec4f::new(0.0, 0.0, 0.0, 1.0),
	))
}
