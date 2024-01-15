pub struct UIRenderModel {

}

impl UIRenderModel {
	pub fn new() -> Self {
		UIRenderModel {

		}
	}

	pub fn new_as_system<'a>() -> EntityReturn<'a, Self> {
		EntityReturn::new(UIRenderModel::new())
	}
	
}

use crate::{rendering::rendering_domain::RenderingDomain, core::{orchestrator::{EntityReturn, EntitySubscriber, OrchestratorReference,}, Entity, entity::EntityHandle}};

use super::Text;

impl RenderingDomain for UIRenderModel {
}

impl Entity for UIRenderModel {
}

// impl EntitySubscriber<dyn Text> for UIRenderModel {
// 	async fn on_create<'a>(&'a mut self, orchestrator: OrchestratorReference, handle: EntityHandle<dyn Text>, params: &dyn Text) {
		
// 	}

// 	async fn on_update(&'static mut self, orchestrator: OrchestratorReference, handle: EntityHandle<dyn Text>, params: &dyn Text) {
		
// 	}
// }