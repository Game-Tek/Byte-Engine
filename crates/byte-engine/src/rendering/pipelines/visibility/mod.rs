//! The visibility-buffer scene pipeline: meshlet culling, a compact visibility buffer, per-material compute
//! shading, cascaded and local shadows, and GTAO.
//!
//! Read the module in this order:
//!
//! * [`manager`]: [`VisibilityPipelineManager`] owns the scene, adopts loaded resources, and prepares each frame.
//! * [`scene`]: what the renderer retains between frames, and the per-frame instance lists it derives.
//! * [`loader`]: the single loader object and request protocol that make every pipeline resource resident.
//! * [`geometry`]: GPU geometry buffers and the worker-side conversion of meshes into them.
//! * [`render_pass`]: the per-sink GPU passes.
//! * [`shader_generator`]: turns authored materials into material-evaluation compute shaders.
//! * [`layout`] and [`shader_data`]: the binding slots, limits, and GPU records everything above shares.
//!
//! Application wiring lives in [`crate::application::graphics::setup_pbr_visibility_shading_render_pipeline`].

mod geometry;
mod layout;
pub mod load;
mod loader;
mod manager;
mod mesh_dispatch;
mod render_pass;
mod scene;
mod shader_data;
mod shader_generator;
mod shadow_selection;
mod skinning;
mod slots;
#[cfg(test)]
mod tests;

pub(crate) use geometry::GeometryHandles;
pub use loader::MaterialPipelineConfig;
pub(crate) use loader::spawn as spawn_loader;
pub use manager::{
	CONE_SHADOW_MAP_POOL_CAPACITY_PARAMETER, POINT_SHADOW_MAP_POOL_CAPACITY_PARAMETER, VisibilityPipelineManager,
	VisibilityPipelineSettings,
};
pub use render_pass::GTAO_CONFIGURATION_PREFIX;
pub use shader_generator::{ScopeAccess, VisibilityShaderGenerator, VisibilityShaderScope};

/// Size of the shared upload arena used by visibility loader lanes.
pub const ASYNC_UPLOAD_BUFFER_BYTE_COUNT: usize = 1024 * 1024 * 32;
