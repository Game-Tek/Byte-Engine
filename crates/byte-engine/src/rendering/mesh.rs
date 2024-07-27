//! Mesh component module

use core::{entity::EntityBuilder, listener::Listener};
use std::future::join;

use maths_rs::{mat::{MatRotate3D, MatScale, MatTranslate}, normalize};
use utils::BoxedFuture;

use crate::{core::{orchestrator, Entity}, gameplay::Transform, math};

pub trait RenderEntity: Entity {
	fn get_transform(&self) -> maths_rs::Mat4f;
	fn get_resource_id(&self) -> &'static str;
}

pub struct Mesh {
	resource_id: &'static str,
	transform: Transform,
}

impl Entity for Mesh {
	fn call_listeners<'a>(&'a self, listener: &'a core::listener::BasicListener, handle: core::EntityHandle<Self>) -> BoxedFuture<'a, ()> where Self: Sized { Box::pin(async move {
		let se = listener.invoke_for(handle.clone(), self);
		let re = listener.invoke_for(handle.clone() as core::EntityHandle<dyn RenderEntity>, self as &dyn RenderEntity);
		join!(se, re).await;
	}) }
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