use crate::orchestrator;

pub mod render_orchestrator;
pub mod shader_generator;
pub mod common_shader_generator;
pub mod visibility_shader_generator;
pub mod render_system;

pub mod directional_light;
pub mod point_light;

pub mod cct;
mod vulkan_render_system;

pub fn create_render_system(orchestrator: &orchestrator::Orchestrator) -> orchestrator::EntityHandle<render_system::RenderSystemImplementation> {
	orchestrator.spawn_entity(vulkan_render_system::VulkanRenderSystem::new_as_system()).unwrap()
}