//! Renderable mesh payloads and their geometry sources.
//!
//! Use [`RenderableMesh`] to publish resource-backed or generated geometry to a
//! rendering pipeline. Transform updates are published separately through the
//! world transform channel.

use std::sync::Arc;

use maths_rs::Vec3f;

use crate::rendering::mesh::generator::{BoxMeshGenerator, MeshGenerator, SphereMeshGenerator};

/// The `RenderableMesh` struct carries geometry source data from gameplay to rendering.
#[derive(Clone)]
pub struct RenderableMesh {
	source: MeshSource,
}

impl RenderableMesh {
	/// Creates a renderable mesh backed by a named resource.
	pub fn resource(id: &'static str) -> Self {
		Self {
			source: MeshSource::Resource(id),
		}
	}

	/// Creates a renderable mesh backed by a procedural mesh generator.
	pub fn generated(generator: Arc<dyn MeshGenerator>) -> Self {
		Self {
			source: MeshSource::Generated(generator),
		}
	}

	/// Creates a procedurally generated sphere with the requested radius.
	pub fn sphere(radius: f32) -> Self {
		Self::generated(Arc::new(SphereMeshGenerator::from_radius(radius)))
	}

	/// Creates a procedurally generated box from unbranded mesh-space half extents.
	pub fn r#box(size: Vec3f) -> Self {
		Self::generated(Arc::new(BoxMeshGenerator::from_size(size)))
	}

	/// Returns the geometry source consumed by the rendering pipeline.
	pub fn source(&self) -> &MeshSource {
		&self.source
	}
}

/// The `MeshSource` enum selects resource-backed or procedurally generated geometry.
#[derive(Clone)]
pub enum MeshSource {
	Resource(&'static str),
	Generated(Arc<dyn MeshGenerator>),
}

impl MeshSource {
	pub fn sphere(radius: f32) -> Self {
		MeshSource::Generated(Arc::new(SphereMeshGenerator::from_radius(radius)))
	}

	/// Creates a generated box from unbranded mesh-space half extents.
	pub fn r#box(size: Vec3f) -> Self {
		MeshSource::Generated(Arc::new(BoxMeshGenerator::from_size(size)))
	}
}

impl From<Arc<dyn MeshGenerator>> for MeshSource {
	fn from(generator: Arc<dyn MeshGenerator>) -> Self {
		MeshSource::Generated(generator)
	}
}
