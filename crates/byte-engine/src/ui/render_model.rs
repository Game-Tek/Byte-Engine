pub struct UIRenderModel {

}

impl UIRenderModel {
	pub fn new() -> Self {
		UIRenderModel {

		}
	}

}

use crate::{core::{Entity, listener::Listener}, gameplay::transform::TransformationUpdate, rendering::{Viewport, render_pass::{RenderPassBuilder, RenderPassFunction}, scene_manager::SceneManager}};
use utils::Box;
use super::Text;

impl SceneManager for UIRenderModel {
	fn prepare(&mut self, frame: &mut ghi::Frame, viewports: &[Viewport], _transforms_listener: &mut dyn Listener<TransformationUpdate>) -> Option<Vec<Box<dyn RenderPassFunction>>> {
		None
	}

	fn create_view(&mut self, id: usize, render_pass_builder: &mut RenderPassBuilder) {
        todo!()
    }
}

impl Entity for UIRenderModel {
}

// impl EntitySubscriber<dyn Text> for UIRenderModel {
// 	async fn on_create<'a>(&'a mut self, orchestrator: OrchestratorReference, handle: EntityHandle<dyn Text>, params: &dyn Text) {

// 	}

// 	async fn on_update(&'static mut self, orchestrator: OrchestratorReference, handle: EntityHandle<dyn Text>, params: &dyn Text) {

// 	}
// }
