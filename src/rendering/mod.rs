use crate::orchestrator;

pub mod render_orchestrator;
pub mod shader_generator;
pub mod common_shader_generator;
pub mod visibility_shader_generator;
pub mod render_system;
mod vulkan_render_system;

pub fn create_render_system(orchestrator: &orchestrator::Orchestrator) -> orchestrator::EntityHandle<dyn render_system::RenderSystem> {
	orchestrator::EntityHandle::from_contents(orchestrator.spawn_entity(vulkan_render_system::VulkanRenderSystem::new_as_system).unwrap().get_contents())
}