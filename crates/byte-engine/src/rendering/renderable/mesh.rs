use std::sync::Arc;

use math::{normalize, Vector3};

use crate::{core::{Entity, EntityHandle}, gameplay::{transform::Transform, Transformable}, rendering::mesh::generator::{BoxMeshGenerator, MeshGenerator, SphereMeshGenerator}};

pub trait RenderableMesh: Transformable {
	fn get_mesh(&self) -> &MeshSource;
}

#[derive(Clone)]
pub enum MeshSource {
	Resource(&'static str),
	Generated(Arc<dyn MeshGenerator>),
}

impl MeshSource {
	pub fn sphere(radius: f32) -> Self {
		MeshSource::Generated(Arc::new(SphereMeshGenerator::from_radius(radius)))
	}

	pub fn r#box(size: Vector3) -> Self {
		MeshSource::Generated(Arc::new(BoxMeshGenerator::from_size(size)))
	}
}

impl Into<MeshSource> for Arc<dyn MeshGenerator> {
	fn into(self) -> MeshSource {
		MeshSource::Generated(self)
	}
}

#[derive(Clone)]
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
