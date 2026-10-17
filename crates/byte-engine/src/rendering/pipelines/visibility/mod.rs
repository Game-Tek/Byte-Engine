//! The visibility-buffer scene pipeline: meshlet culling, a compact visibility buffer, per-material compute
//! shading, cascaded and local shadows, and GTAO.
//!
//! Read the module in this order:
//!
//! * `features`: [`VisibilityFeatures`], the optional features a project's `config.json` builds into its shaders.
//! * `manager`: [`VisibilityPipelineManager`] owns the scene, adopts loaded resources, and prepares each frame.
//! * `scene`: what the renderer retains between frames, and the per-frame instance lists it derives.
//! * `loader`: the single loader object and request protocol that make every pipeline resource resident.
//! * `geometry`: GPU geometry buffers, sized by [`GeometryCapacity`], and the worker-side conversion of meshes into
//!   them.
//! * `lod`: which of a primitive's baked detail levels each instance draws.
//! * `render_pass`: the per-sink GPU passes.
//! * `shader_generator`: [`VisibilityShaderGenerator`] turns authored materials into material-evaluation compute
//!   shaders.
//! * `layout` and `shader_data`: the binding slots, limits, and GPU records everything above shares.
//!
//! Application wiring lives in [`crate::application::graphics::setup_pbr_visibility_shading_render_pipeline`].

mod features;
mod geometry;
mod layout;
mod loader;
mod lod;
mod manager;
mod mesh_dispatch;
mod render_pass;
mod scene;
mod set_layouts;
mod shader_data;
mod shader_generator;
mod shadow_selection;
mod shadow_updates;
mod skinning;
#[cfg(test)]
mod tests;

pub use features::{GTAO_ENABLED_PARAMETER, RENDER_PARAMETERS, VisibilityFeatures};
pub use geometry::GeometryCapacity;
pub(crate) use geometry::GeometryHandles;
pub(crate) use loader::spawn as spawn_loader;
pub use manager::{
	DIRECTIONAL_SHADOW_DISTANCE_PARAMETER, DIRECTIONAL_SHADOW_FITTING_PARAMETER, DIRECTIONAL_SHADOW_RESOLUTION_PARAMETER,
	DIRECTIONAL_SHADOW_SPLIT_BLEND_PARAMETER, GEOMETRY_CAPACITY_PARAMETER_PREFIX, SHADOW_MAP_BUDGET_PARAMETER,
	VisibilityPipelineManager, VisibilityPipelineSettings,
};
pub use shader_generator::VisibilityShaderGenerator;
