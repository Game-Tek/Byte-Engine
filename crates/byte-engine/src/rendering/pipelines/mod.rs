//! Scene rendering strategies installed into the renderer.
//!
//! Use [`simple`] for diagnostics and early prototypes. Use [`visibility`] for
//! the main material, lighting, and visibility-buffer pipeline configured by
//! [`crate::application::graphics::setup_pbr_visibility_shading_render_pipeline`].

pub mod simple;
pub mod visibility;
