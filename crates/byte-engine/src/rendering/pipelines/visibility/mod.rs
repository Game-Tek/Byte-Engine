//! The visibility-buffer scene pipeline: meshlet culling, a compact visibility buffer, per-material compute
//! shading, cascaded and local shadows, and GTAO.
//!
//! Read the module in this order:
//!
//! * [`manager`]: [`VisibilityPipelineManager`] owns the scene, adopts loaded resources, and prepares each frame.
//! * [`scene`]: what the renderer retains between frames, and the per-frame instance lists it derives.
//! * [`resources`]: asynchronous loading of meshes, materials, textures, and environments.
//! * [`geometry`]: GPU geometry buffers and the worker-side conversion of meshes into them.
//! * [`render_pass`]: the per-sink GPU passes.
//! * [`shader_generator`]: turns authored materials into material-evaluation compute shaders.
//! * [`layout`] and [`shader_data`]: the binding slots, limits, and GPU records everything above shares.
//!
//! Application wiring lives in [`crate::application::graphics::setup_pbr_visibility_shading_render_pipeline`].

mod geometry;
mod layout;
pub mod load;
mod manager;
mod mesh_dispatch;
mod render_pass;
mod resources;
mod scene;
mod shader_data;
mod shader_generator;
mod shadow_selection;
mod skinning;
#[cfg(test)]
mod tests;

pub use manager::{
	CONE_SHADOW_MAP_POOL_CAPACITY_PARAMETER, POINT_SHADOW_MAP_POOL_CAPACITY_PARAMETER, VisibilityPipelineManager,
	VisibilityPipelineSettings,
};
pub use render_pass::GTAO_CONFIGURATION_PREFIX;
pub use resources::{ASYNC_UPLOAD_BUFFER_BYTE_COUNT, MaterialPipelineConfig, VisibilityResourcePreparer};
pub use shader_generator::{ScopeAccess, VisibilityShaderGenerator, VisibilityShaderScope};
