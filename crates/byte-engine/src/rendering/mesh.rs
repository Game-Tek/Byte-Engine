//! Mesh component module

use crate::core::EntityHandle;
use crate::core::{entity::EntityBuilder, Entity};
use crate::{core::orchestrator, gameplay::Transform};

use std::{borrow::Cow, future::join};

use math::{normalize, Vector3, Vector4, Matrix4};
use utils::BoxedFuture;

pub trait MeshGenerator {
	fn vertices(&self) -> Cow<[(f32, f32, f32)]>;
	fn normals(&self) -> Cow<[(f32, f32, f32)]>;
	fn uvs(&self) -> Cow<[(f32, f32)]>;
	fn indices(&self) -> Cow<[u32]>;
	fn tangents(&self) -> Cow<[Vector3]>;
	fn bitangents(&self) -> Cow<[Vector3]>;
	fn colors(&self) -> Option<Cow<[Vector4]>> { None }
	fn meshlet_indices(&self) -> Option<Cow<[u8]>> { None }
}

pub enum MeshSource {
	Resource(&'static str),
	Generated(Box<dyn MeshGenerator>),
}

pub trait RenderEntity: Entity {
	fn get_transform(&self) -> Matrix4;
	fn get_mesh(&self) -> &MeshSource;
}

pub struct Mesh {
	source: MeshSource,
	transform: Transform,
}

impl Entity for Mesh {}

impl RenderEntity for Mesh {
	fn get_transform(&self) -> Matrix4 { self.transform.get_matrix() }
	fn get_mesh(&self) -> &MeshSource {
		&self.source
	}
}

impl Mesh {
	pub fn new(resource_id: &'static str, transform: Transform) -> EntityBuilder<'static, Self> {
		Self {
			source: MeshSource::Resource(resource_id),
			transform,
		}.into()
	}

	pub fn create(resource_id: &'static str, transform: Transform) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self {
			source: MeshSource::Resource(resource_id),
			transform,
		}).r#as(|h| h).r#as(|h| h as EntityHandle<dyn RenderEntity>)
	}

	pub fn new_generated(generator: Box<dyn MeshGenerator>, transform: Transform) -> EntityBuilder<'static, Self> {
		Self {
			source: MeshSource::Generated(generator),
			transform,
		}.into()
	}

	pub fn set_orientation(&mut self, orientation: Vector3) {
		self.transform.set_orientation(normalize(orientation));
	}

	pub fn transform(&self) -> &Transform {
		&self.transform
	}

	pub fn transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}
}
