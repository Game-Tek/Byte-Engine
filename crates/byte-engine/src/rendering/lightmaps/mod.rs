//! Baked lightmaps: the GPU path tracer that bakes a level's indirect diffuse light into one image.
//!
//! BELD runs the bake when it bakes a `.lightmap.bead` asset. The bake reads the level, meshes, and materials the
//! renderer reads, prepares them with the visibility pipeline's own code, and traces them with the materials'
//! closest-hit shaders, so the light it stores lines up with what the renderer draws. The renderer then reads the image
//! through the [`Lightmap`](crate::rendering::Lightmap) component a level's `lightmap` kind publishes.

mod atlas;
mod bake;
mod bead;
mod gpu;
mod guides;
mod scene;
mod texels;
#[cfg(test)]
mod tests;

pub use bake::LightmapAssetHandler;

/// The stems, under `byte_engine/rendering/lightmaps/`, of the shaders only the lightmap bake uses, which a bake without
/// lightmaps leaves out.
pub(crate) const LIGHTMAP_BAKE_SHADERS: [&str; 2] = ["lightmap-trace", "lightmap-dilate"];
