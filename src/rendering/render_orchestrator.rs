//! The render orchestrator is responsible for managing the render passes and the render domains.

use utils::insert_return_length;

use crate::core::{orchestrator::{self,}, Entity, entity::EntityBuilder};

#[derive(Clone)]
pub struct RenderPass {
	name: String,
}

#[derive(Clone)]
pub struct RenderOrchestrator {
	render_passes: Vec<RenderPass>,
}

impl RenderOrchestrator {
	pub fn new<'a>() -> EntityBuilder<'a, RenderOrchestrator> {
		EntityBuilder::new(Self { render_passes: Vec::new(), })
	}

	pub fn add_render_pass(&mut self, name: &str, _depends_on: &[&str]) -> usize {
		insert_return_length(&mut self.render_passes, RenderPass {
			name: name.to_string(),
		})
	}

	pub fn get_render_pass(&self, name: &str) -> Option<&RenderPass> {
		self.render_passes.iter().find(|pass| pass.name == name)
	}
}

impl Entity for RenderOrchestrator {}