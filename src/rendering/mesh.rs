//! Mesh component module

use core::{entity::EntityBuilder, listener::Listener};

use crate::core::{orchestrator, Entity};

pub trait RenderEntity: Entity {
	fn get_transform(&self) -> maths_rs::Mat4f;
	fn get_material_id(&self) -> &'static str;
	fn get_resource_id(&self) -> &'static str;
}

pub struct Mesh {
	resource_id: &'static str,
	material_id: &'static str,
	transform: maths_rs::Mat4f,
}

impl Entity for Mesh {
	fn call_listeners(&self, listener: &core::listener::BasicListener, handle: core::EntityHandle<Self>) where Self: Sized {
		listener.invoke_for(handle.clone(), self);
		listener.invoke_for(handle.clone() as core::EntityHandle<dyn RenderEntity>, self as &dyn RenderEntity);
	}
}

impl RenderEntity for Mesh {
	fn get_transform(&self) -> maths_rs::Mat4f { self.transform }
	fn get_material_id(&self) -> &'static str { self.material_id }
	fn get_resource_id(&self) -> &'static str { self.resource_id }
}

impl Mesh {
	pub fn new(resource_id: &'static str, material_id: &'static str, transform: maths_rs::Mat4f) -> EntityBuilder<'static, Self> {
		Self {
			resource_id,
			material_id,
			transform,
		}.into()
	}

	fn set_transform(&mut self, value: maths_rs::Mat4f) { self.transform = value; }
	pub fn get_transform(&self) -> maths_rs::Mat4f { self.transform }

	pub fn get_resource_id(&self) -> &'static str { self.resource_id }
	pub fn get_material_id(&self) -> &'static str { self.material_id }
}