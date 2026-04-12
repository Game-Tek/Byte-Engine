//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

pub mod render_pass;
pub mod scene_manager;
pub mod shader_generator;

use math::ShaderMatrix4;
pub use render_pass::RenderPass;
pub use scene_manager::SceneManager;

pub use render_pass::RenderPass as SimpleRenderPass;
pub use scene_manager::SceneManager as SimpleSceneManager;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CameraShaderData {
	vp: ShaderMatrix4,
}
