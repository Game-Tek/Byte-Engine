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

use ::utils::Extent;
use ghi::context::ContextCreate as _;

use crate::space::Positionable as _;

pub mod common_shader_generator;

pub mod lights;
pub mod window;

pub mod camera;
pub mod mesh;

pub mod renderable;

pub mod cct;

pub mod pipeline_manager;
pub mod world_render_domain;

pub mod renderer;

pub mod framebuffer;
pub mod render_pass;
pub mod render_passes;
pub mod shader_store;

pub mod pipelines;

pub mod sink;
pub mod view;

pub mod csm;

pub mod utils;

pub use camera::Camera;
pub use render_pass::RenderPass;
pub use renderable::mesh::RenderableMesh;
pub use sink::Sink;
pub use view::View;

/// Maps a shader resource binding to a GHI shader binding descriptor.
pub fn map_shader_binding_to_shader_binding_descriptor(
	b: &resource_management::shader::generator::CompiledShaderBinding,
) -> ghi::shader::BindingDescriptor {
	ghi::shader::BindingDescriptor::new(
		b.set,
		b.binding,
		if b.read {
			ghi::AccessPolicies::READ
		} else {
			ghi::AccessPolicies::empty()
		} | if b.write {
			ghi::AccessPolicies::WRITE
		} else {
			ghi::AccessPolicies::empty()
		},
	)
}

pub fn create_shader_from_source(
	context: &mut ghi::implementation::Context,
	name: Option<&str>,
	source: ghi::shader::ShaderSource,
	stage: ghi::ShaderTypes,
	binding_descriptors: impl IntoIterator<Item = ghi::shader::BindingDescriptor>,
) -> Result<ghi::ShaderHandle, String> {
	let compiled = ghi::shader::compile(name.unwrap_or(""), source)?;
	context
		.create_shader(name, compiled.as_source(), stage, binding_descriptors)
		.map_err(|_| "Failed to create shader. The most likely cause is an incompatible shader interface.".to_string())
}

pub fn make_perspective_view_from_camera(camera: &Camera, extent: Extent) -> View {
	let (camera_position, camera_orientation, fov_y) = (camera.position(), camera.get_direction(), camera.get_fov());

	let aspect_ratio = extent.width() as f32 / extent.height() as f32;

	View::new_perspective(fov_y, aspect_ratio, 0.1f32, 100f32, camera_position, camera_orientation)
}
