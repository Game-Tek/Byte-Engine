//! Reusable post-scene rendering passes.
//!
//! Tone mapping, bloom, LUT processing, sky rendering, and utility passes in
//! this module implement [`crate::rendering::RenderPass`]. Install them through
//! [`crate::rendering::renderer::Renderer`] or the corresponding helpers in
//! [`crate::application::graphics`].

#[cfg(test)]
mod bilateral_blur;
pub mod blit;

pub mod aces;
pub mod agx;
pub mod bloom;
pub mod color_grading;
pub mod lut;
pub mod sky;
pub mod smaa;

pub mod serial;

mod tone_map;
