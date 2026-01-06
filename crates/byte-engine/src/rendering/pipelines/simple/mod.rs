//! The simple render model provides a simplified rendering model for Byte-Engine applications. Useful for debugging and prototyping.

pub mod render_pass;
pub mod scene_manager;
pub mod shader_generator;

pub use render_pass::RenderPass;
pub use scene_manager::SceneManager;

pub use render_pass::RenderPass as SimpleRenderPass;
pub use scene_manager::SceneManager as SimpleSceneManager;
