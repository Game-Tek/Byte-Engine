//! Mesh component module

use crate::orchestrator;

#[derive(component_derive::Component)]
pub struct Mesh{
	resource_id: &'static str,
	material_id: &'static str,
	#[field] transform: maths_rs::Mat4f,
}

impl orchestrator::Entity for Mesh {}

impl orchestrator::Component for Mesh {
	// type Parameters<'a> = MeshParameters;
}

impl Mesh {
	pub fn new(resource_id: &'static str, material_id: &'static str, transform: maths_rs::Mat4f) -> Self {
		Self {
			resource_id,
			material_id,
			transform,
		}
	}

	fn set_transform(&mut self, value: maths_rs::Mat4f) { self.transform = value; }
	pub fn get_transform(&self) -> maths_rs::Mat4f { self.transform }
	pub const fn transform() -> orchestrator::Property2<Self, maths_rs::Mat4f> { orchestrator::Property2 { getter: Mesh::get_transform, setter: Mesh::set_transform } }

	pub fn get_resource_id(&self) -> &'static str { self.resource_id }
	pub fn get_material_id(&self) -> &'static str { self.material_id }
}