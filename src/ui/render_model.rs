pub struct UIRenderModel {

}

impl UIRenderModel {
	pub fn new() -> Self {
		UIRenderModel {

		}
	}

	pub fn new_as_system() -> EntityReturn<'static, Self> {
		EntityReturn::new(UIRenderModel::new())
	}
	
}

use crate::{rendering::rendering_domain::RenderingDomain, orchestrator::{Entity, EntityReturn, EntitySubscriber, OrchestratorReference, EntityHandle}};

use super::Text;

impl RenderingDomain for UIRenderModel {
}

impl Entity for UIRenderModel {
}

impl EntitySubscriber<dyn Text> for UIRenderModel {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<dyn Text>, params: &dyn Text) {
		
	}

	fn on_update(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<dyn Text>, params: &dyn Text) {
		
	}
}