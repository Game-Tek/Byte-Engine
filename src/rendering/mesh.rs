//! Mesh component module

use crate::orchestrator;

pub struct Mesh {
	resource_id: &'static str,
	material_id: &'static str,
	transform: maths_rs::Mat4f,
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
	pub const fn transform() -> orchestrator::EventDescription<Self, maths_rs::Mat4f> { orchestrator::EventDescription::new() }

	pub fn get_resource_id(&self) -> &'static str { self.resource_id }
	pub fn get_material_id(&self) -> &'static str { self.material_id }
}