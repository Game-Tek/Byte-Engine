//! Mesh component module

use crate::core::{orchestrator, Entity};

pub struct Mesh {
	resource_id: &'static str,
	material_id: &'static str,
	transform: maths_rs::Mat4f,
}

impl Entity for Mesh {}

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

	pub fn get_resource_id(&self) -> &'static str { self.resource_id }
	pub fn get_material_id(&self) -> &'static str { self.material_id }
}