//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

pub mod pipeline_manager;
pub mod render_pass;
pub mod shader_generator;

use math::ShaderMatrix4;
pub use pipeline_manager::PipelineManager;
pub use pipeline_manager::PipelineManager as SimplePipelineManager;
pub use render_pass::RenderPass;
pub use render_pass::RenderPass as SimpleRenderPass;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CameraShaderData {
	vp: ShaderMatrix4,
}
