//! Mesh component module

use crate::orchestrator;

#[derive(component_derive::Component)]
pub struct Mesh{
	pub resource_id: &'static str,
	pub material_id: &'static str,
	#[field] pub transform: maths_rs::Mat4f,
}

impl orchestrator::Entity for Mesh {}

impl orchestrator::Component for Mesh {
	// type Parameters<'a> = MeshParameters;
}

impl Mesh {
	fn set_transform(&mut self, _orchestrator: orchestrator::OrchestratorReference, value: maths_rs::Mat4f) { self.transform = value; }

	fn get_transform(&self) -> maths_rs::Mat4f { self.transform }

	pub const fn transform() -> orchestrator::Property<(), Self, maths_rs::Mat4f> { orchestrator::Property::Component { getter: Mesh::get_transform, setter: Mesh::set_transform } }
}