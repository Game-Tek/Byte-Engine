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
//! See the [rendering guide](https://byte-engine.0x44491229.dev/docs/develop/design/rendering)
//! for the relationships between render orchestrators, systems, domains, and models.

use ::utils::Extent;
use ghi::context::ContextCreate as _;

use crate::space::Positionable as _;

#[doc(hidden)]
pub mod common_shader_generator;

#[doc(hidden)]
pub mod lights;
#[doc(hidden)]
pub mod window;

/// Camera state used to derive scene views.
pub mod camera;
#[doc(hidden)]
pub mod mesh;

#[doc(hidden)]
pub mod renderable;

#[doc(hidden)]
pub mod cct;

#[doc(hidden)]
pub mod pipeline_manager;
mod pose;
#[doc(hidden)]
pub mod world_render_domain;

#[doc(hidden)]
pub mod renderer;

#[doc(hidden)]
pub mod framebuffer;
#[doc(hidden)]
pub mod render_pass;
#[doc(hidden)]
pub mod render_passes;
#[doc(hidden)]
pub mod shader_store;

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

#[doc(hidden)]
pub mod utils;

pub use camera::Camera;
pub use lights::{ConeLight, DirectionalLight, Light, LightClasses, Lights, PointLight};
pub use pipeline_manager::PipelineManager;
pub use pipelines::{SimplePipelineManager, SimpleRenderPass, VisibilityPipelineManager};
pub use pose::UpdatePose;
pub use render_pass::{FramePrepare, ReadFromResult, RenderPass, RenderPassBuilder, RenderPassReturn, RenderToResult};
pub use renderable::mesh::RenderableMesh;
pub use renderer::{RenderTargets, Renderer, Settings};
pub use sink::Sink;
pub use view::View;
pub use window::{Features, Window};

/// Maps a shader resource binding to a GHI shader binding descriptor.
pub fn map_shader_binding_to_shader_binding_descriptor(
	b: &resource_management::shader::generator::CompiledShaderBinding,
) -> ghi::ShaderResourceDescriptor {
	use resource_management::shader::besl::evaluation::{BindingKind, TextureView};

	let kind = match b.kind {
		BindingKind::StorageBuffer => ghi::ResourceKind::StorageBuffer,
		BindingKind::CombinedImageSampler { .. } => ghi::ResourceKind::CombinedImageSampler,
		BindingKind::StorageImage => ghi::ResourceKind::StorageImage,
	};
	let descriptor = ghi::ShaderResourceDescriptor::new(
		ghi::ResourceSlot::new(b.slot),
		kind,
		b.count,
		if b.read {
			ghi::AccessPolicies::READ
		} else {
			ghi::AccessPolicies::empty()
		} | if b.write {
			ghi::AccessPolicies::WRITE
		} else {
			ghi::AccessPolicies::empty()
		},
	);

	match b.kind {
		BindingKind::CombinedImageSampler { view } => descriptor.texture_view_type(match view {
			TextureView::Texture2D => ghi::TextureViewTypes::Texture2D,
			TextureView::Texture2DArray => ghi::TextureViewTypes::Texture2DArray,
			TextureView::Texture3D => ghi::TextureViewTypes::Texture3D,
		}),
		_ => descriptor,
	}
}

/// Compiles shader source and creates a GHI shader handle for render pipeline setup.
///
/// Returns an error when shader compilation or GHI shader creation fails. The
/// most likely cause is invalid shader source or a binding interface that does
/// not match the selected shader stage.
pub fn create_shader_from_source(
	context: &mut ghi::implementation::Context,
	name: Option<&str>,
	source: ghi::shader::ShaderSource,
	stage: ghi::ShaderTypes,
	resource_descriptors: impl IntoIterator<Item = ghi::ShaderResourceDescriptor>,
) -> Result<ghi::ShaderHandle, String> {
	let compiled = ghi::shader::compile(name.unwrap_or(""), source)?;
	context
		.create_shader(name, compiled.as_source(), stage, resource_descriptors)
		.map_err(|_| "Failed to create shader. The most likely cause is an incompatible shader interface.".to_string())
}

/// Builds a perspective [`View`] from a scene camera and render target extent.
pub fn make_perspective_view_from_camera(camera: &Camera, extent: Extent) -> View {
	let (camera_position, camera_orientation, fov_y) = (camera.position(), camera.get_direction(), camera.get_fov());

	let aspect_ratio = extent.width() as f32 / extent.height() as f32;

	View::new_perspective(fov_y, aspect_ratio, 0.1f32, 100f32, camera_position, camera_orientation)
}
