//! Mesh component

use crate::orchestrator::{Component, ComponentHandle, Orchestrator};

pub struct Mesh {

}

impl Component<Mesh> for Mesh {
	fn new(orchestrator: &mut Orchestrator) -> ComponentHandle<Mesh> {
		orchestrator.make_object(Mesh {})
	}
}

impl Mesh {
	pub fn new(orchestrator: &mut Orchestrator, resource_id: &str) -> ComponentHandle<Mesh> {
		orchestrator.make_object(Mesh {})
	}
}