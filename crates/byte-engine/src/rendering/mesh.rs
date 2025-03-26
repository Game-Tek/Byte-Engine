//! Mesh component module

use crate::core::{entity::EntityBuilder, listener::{BasicListener, Listener}, Entity, EntityHandle};
use crate::{core::orchestrator, gameplay::Transform, math};

use std::{borrow::Cow, future::join};

use maths_rs::{mat::{MatRotate3D, MatScale, MatTranslate}, normalize};
use utils::BoxedFuture;

pub trait MeshGenerator {
	fn vertices(&self) -> Cow<[maths_rs::Vec3f]>;
	fn normals(&self) -> Cow<[maths_rs::Vec3f]>;
	fn uvs(&self) -> Cow<[maths_rs::Vec2f]>;
	fn indices(&self) -> Cow<[u32]>;
	fn colors(&self) -> Cow<[maths_rs::Vec4f]>;
	fn tangents(&self) -> Cow<[maths_rs::Vec3f]>;
	fn bitangents(&self) -> Cow<[maths_rs::Vec3f]>;
	fn meshlet_indices(&self) -> Cow<[u8]>;
}

pub enum MeshSource {
	Resource(&'static str),
	Generated(Box<dyn MeshGenerator>),
}

pub trait RenderEntity: Entity {
	fn get_transform(&self) -> maths_rs::Mat4f;
	fn get_mesh(&self) -> MeshSource;
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
	fn get_mesh(&self) -> MeshSource {
		MeshSource::Resource(self.resource_id)
	}
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