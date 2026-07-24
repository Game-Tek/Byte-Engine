//! Renderable mesh contracts and the standard transform-backed mesh entity.
//!
//! Use [`Mesh`] for common resource or generated geometry. Implement
//! [`RenderableMesh`] on custom entities that own a transform and expose a
//! [`MeshSource`], then create them through
//! [`crate::gameplay::world::DefaultWorld::renderable_factory_mut`].

use std::sync::Arc;

use math::{normalize, Vector3};

use crate::{
	core::{Entity, EntityHandle},
	gameplay::transform::Transform,
	rendering::mesh::generator::{BoxMeshGenerator, MeshGenerator, SphereMeshGenerator},
	space::Transformable,
};

/// The [`RenderableMesh`] trait supplies geometry and transform state to scene pipeline managers.
pub trait RenderableMesh: Transformable {
	fn get_mesh(&self) -> &MeshSource;
}

#[derive(Clone)]
/// The [`MeshSource`] enum selects resource-backed or procedurally generated
/// geometry.
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

impl From<Arc<dyn MeshGenerator>> for MeshSource {
	fn from(generator: Arc<dyn MeshGenerator>) -> Self {
		MeshSource::Generated(generator)
	}
}

#[derive(Clone)]
/// The [`Mesh`] struct provides the standard transformable renderable entity.
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
