use crate::orchestrator;

pub mod render_orchestrator;
pub mod shader_generator;
pub mod common_shader_generator;
pub mod visibility_shader_generator;
pub mod render_system;

pub mod directional_light;
pub mod point_light;
pub mod mesh;

pub mod cct;
mod vulkan_render_system;

pub mod world_render_domain;
pub mod visibility_model;

pub mod renderer;

pub mod tonemap_render_pass;

pub mod shadow_render_pass;
pub mod ssao_render_pass;
pub mod aces_tonemap_render_pass;

pub(crate) mod shader_compilation;

pub fn create_render_system(orchestrator: &orchestrator::Orchestrator) -> orchestrator::EntityHandle<render_system::RenderSystemImplementation> {
	orchestrator.spawn_entity(vulkan_render_system::VulkanRenderSystem::new_as_system()).unwrap()
}