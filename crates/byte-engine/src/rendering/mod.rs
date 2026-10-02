//! Rendering orchestration, scene pipelines, and composable post-processing.
//!
//! [`renderer::Renderer`] owns GHI resources and executes [`RenderPass`] values
//! for each [`Sink`]. Applications normally configure it through the setup
//! functions in [`crate::application::graphics`]. Implement
//! [`pipeline_manager::PipelineManager`] for scene rendering strategies and
//! [`RenderPass`] for sink-local post-processing.
//!
//! Use [`pipelines::simple`] for debugging or prototypes. The
//! [`pipelines::visibility`] pipeline is the primary material and lighting path.
//!
//! Follow the [physically based lighting reference](/docs/reference/lighting)
//! to create and submit scene lights with lux, candela, lumens, or nits.

use ::utils::Extent;
use math::direction_from_orientation;

#[doc(hidden)]
pub mod common_shader_generator;

mod environment;

/// Retained wireframe geometry for renderer and gameplay diagnostics.
pub mod debug;
pub mod device;

#[doc(hidden)]
pub mod lights;
pub mod loading;
pub mod particles;
#[doc(hidden)]
pub mod window;

/// Camera state used to derive scene views.
pub mod camera;
#[doc(hidden)]
pub mod mesh;

#[doc(hidden)]
pub mod renderable;

mod pipeline_compilation;
#[doc(hidden)]
pub mod pipeline_manager;
mod pose;
mod resource;

#[doc(hidden)]
pub mod renderer;

#[doc(hidden)]
pub mod render_pass;
#[doc(hidden)]
pub mod render_passes;
pub mod resource_loading;

#[cfg(test)]
pub(crate) mod shader_vm_test;

#[doc(hidden)]
pub mod pipelines;

/// Per-output render target state passed to render passes.
pub mod sink;
/// Projection and view matrix construction for cameras and lights.
pub mod view;

#[doc(hidden)]
pub mod csm;

pub use camera::Camera;
pub use debug::{DebugDepthMode, DebugMesh, DebugMeshRenderPass, DebugSceneManager, DebugShape};
pub use device::GraphicsDevice;
pub use environment::Environment;
pub use lights::{
	ConeLight, DirectionalLight, IesProfile, Light, LightClasses, LightColor, LocalEmission, PhotometricError,
	PhotometricIntensity, PointLight,
};
pub use particles::ParticleEmitter;
pub use pipeline_compilation::{PipelineKey, PipelineManagerClient, PipelineManagerServer, PipelineRef, PipelineState};
pub use pipeline_manager::PipelineManager;
pub use pipelines::{SimplePipelineManager, VisibilityPipelineManager};
pub use pose::UpdatePose;
pub use render_pass::{
	MainRenderTarget, ReadFromResult, RenderPass, RenderPassBuilder, RenderPassReturn, RenderPassState, RenderToResult,
};
pub use renderable::mesh::RenderableMesh;
pub use renderer::{RenderTargets, Renderer};
pub use resource::{Query, Resource};
pub use sink::Sink;
pub use view::View;
pub use window::{Features, Window};

use crate::space::{Orientable, Positionable};

/// Scene rendering stays linear and HDR until the final tone-mapping pass.
///
/// Scene color carries no coverage: the scene background is drawn before transparent surfaces blend over it. HDR
/// effect intermediates share the format. Its 6 bit mantissa is too coarse for display-referred [0, 1] values, so
/// tone mappers write [`DISPLAY_COLOR_FORMAT`] instead.
pub(crate) const SCENE_COLOR_FORMAT: ghi::Formats = ghi::Formats::RGBu11u11u10;

/// Display-referred color written by tone mapping and grading.
pub(crate) const DISPLAY_COLOR_FORMAT: ghi::Formats = ghi::Formats::RGBA16F;

/// Warns when a condition first holds, and again only after a frame where it did not.
///
/// Keep one `reported` flag per condition and call this every frame, so a lasting problem logs once instead of
/// every frame.
pub(crate) fn warn_once(reported: &mut bool, exceeded: bool, message: impl FnOnce() -> String) {
	if exceeded && !*reported {
		log::warn!("{}", message());
	}
	*reported = exceeded;
}

/// Builds a perspective [`View`] from a scene camera and render target extent.
pub fn make_perspective_view_from_camera<T: Positionable + Orientable>(camera: &Camera, po: &T, extent: Extent) -> View {
	let (camera_position, camera_orientation, fov_y) = (po.position(), po.orientation(), camera.vertical_fov());
	debug_assert!(
		extent.width() > 0 && extent.height() > 0,
		"Perspective extent is empty. The most likely cause is building a camera view before the render target is sized."
	);
	debug_assert!(
		fov_y.is_finite() && fov_y > math::Degrees::new(0.0) && fov_y < math::Degrees::new(180.0),
		"Camera field of view is invalid. The most likely cause is an unset, non-finite, or out-of-range perspective angle."
	);

	let aspect_ratio = extent.width() as f32 / extent.height() as f32;

	View::new_perspective(
		fov_y,
		aspect_ratio,
		0.1f32,
		100f32,
		camera_position,
		direction_from_orientation(camera_orientation),
	)
}
