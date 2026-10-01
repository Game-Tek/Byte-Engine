//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

#[doc(hidden)]
pub mod pipeline_manager;
#[doc(hidden)]
pub mod render_pass;
pub(crate) mod resource_manager;

pub use pipeline_manager::PipelineManager;
pub use pipeline_manager::PipelineManager as SimplePipelineManager;
pub use render_pass::RenderPass;

#[repr(C)]
/// The `CameraShaderData` struct shares simple-pipeline camera data
/// with generated shader code.
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraShaderData {
	vp: ghi::pod::Mat4f,
}
