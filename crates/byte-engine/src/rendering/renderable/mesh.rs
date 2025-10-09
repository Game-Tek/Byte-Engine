use math::{normalize, Vector3};

use crate::{core::{entity::EntityBuilder, Entity, EntityHandle}, gameplay::{Transform, Transformable}, rendering::mesh::generator::{BoxMeshGenerator, MeshGenerator, SphereMeshGenerator}};

pub trait RenderableMesh: Transformable + Entity + Send {
	fn get_mesh(&self) -> &MeshSource;
}

pub enum MeshSource {
	Resource(&'static str),
	Generated(Box<dyn MeshGenerator>),
}

impl MeshSource {
	pub fn sphere(radius: f32) -> Self {
		MeshSource::Generated(Box::new(SphereMeshGenerator::from_radius(radius)))
	}

	pub fn r#box(size: Vector3) -> Self {
		MeshSource::Generated(Box::new(BoxMeshGenerator::from_size(size)))
	}
}

impl Into<MeshSource> for Box<dyn MeshGenerator> {
	fn into(self) -> MeshSource {
		MeshSource::Generated(self)
	}
}

pub struct Mesh {
	source: MeshSource,
	transform: Transform,
}

impl Entity for Mesh {}

impl Transformable for Mesh {
	fn transform(&self) -> &Transform {
		&self.transform
	}

	fn transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}
}

impl RenderableMesh for Mesh {
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
		}).r#as(|h| h).r#as(|h| h as EntityHandle<dyn RenderableMesh>)
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
