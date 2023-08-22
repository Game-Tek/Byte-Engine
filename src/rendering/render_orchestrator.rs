//! The render orchestrator is responsible for managing the render passes and the render domains.

use crate::{insert_return_length, orchestrator::{System, self, Entity}};

#[derive(Clone)]
pub struct RenderPass {
	name: String,
}

#[derive(Clone)]
pub struct RenderOrchestrator {
	render_passes: Vec<RenderPass>,
}

impl RenderOrchestrator {
	pub fn new(orchestrator: orchestrator::OrchestratorReference) -> orchestrator::EntityReturn<RenderOrchestrator> {
		Some((Self {
			render_passes: Vec::new(),
		}, vec![]))
	}

	pub fn add_render_pass(&mut self, name: &str, depends_on: &[&str]) -> usize {
		insert_return_length(&mut self.render_passes, RenderPass {
			name: name.to_string(),
		})
	}

	pub fn get_render_pass(&self, name: &str) -> Option<&RenderPass> {
		self.render_passes.iter().find(|pass| pass.name == name)
	}
}

impl Entity for RenderOrchestrator {}
impl System for RenderOrchestrator {}
	