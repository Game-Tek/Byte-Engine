//! Mesh component module

use crate::core::{entity::EntityBuilder, listener::{BasicListener, Listener}, Entity, EntityHandle};
use crate::{core::orchestrator, gameplay::Transform, math};

use std::future::join;

use maths_rs::{mat::{MatRotate3D, MatScale, MatTranslate}, normalize};
use utils::BoxedFuture;

pub trait RenderEntity: Entity {
	fn get_transform(&self) -> maths_rs::Mat4f;
	fn get_resource_id(&self) -> &'static str;
}

pub struct Mesh {
	resource_id: &'static str,
	transform: Transform,
}

impl Entity for Mesh {
	fn call_listeners<'a>(&'a self, listener: &'a BasicListener, handle: EntityHandle<Self>) -> () where Self: Sized {
		let se = listener.invoke_for(handle.clone(), self);
		let re = listener.invoke_for(handle.clone() as EntityHandle<dyn RenderEntity>, self as &dyn RenderEntity);
	}
}

impl RenderEntity for Mesh {
	fn get_transform(&self) -> maths_rs::Mat4f { self.transform.get_matrix() }
	fn get_resource_id(&self) -> &'static str { self.resource_id }
}

impl Mesh {
	pub fn new(resource_id: &'static str, transform: Transform) -> EntityBuilder<'static, Self> {
		Self {
			resource_id,
			transform,
		}.into()
	}

	pub fn get_resource_id(&self) -> &'static str { self.resource_id }

	pub fn set_orientation(&mut self, orientation: maths_rs::Vec3f) {
		self.transform.set_orientation(normalize(orientation));
	}

	pub fn transform(&self) -> &Transform {
		&self.transform
	}

	pub fn transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}
}