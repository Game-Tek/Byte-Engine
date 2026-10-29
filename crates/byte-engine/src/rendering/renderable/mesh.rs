//! Renderable mesh payloads and their geometry sources.
//!
//! Use [`RenderableMesh`] to publish resource-backed or generated geometry to a
//! rendering pipeline. Transform updates are published separately through the
//! world transform channel.

use std::hash::{Hash as _, Hasher as _};
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
	///
	/// The ID is shared, not copied, by every system that reads the mesh, so pass an `Arc<str>` to share one ID
	/// between many meshes.
	pub fn resource(id: impl Into<Arc<str>>) -> Self {
		Self {
			source: MeshSource::Resource(id.into()),
		}
	}

	/// Creates a renderable mesh backed by a procedural mesh generator.
	pub fn generated(generator: Box<dyn MeshGenerator>) -> Self {
		Self {
			source: MeshSource::Generated(generator),
		}
	}

	/// Creates a procedurally generated sphere with the requested radius.
	pub fn sphere(radius: f32) -> Self {
		Self::generated(Box::new(SphereMeshGenerator::from_radius(radius)))
	}

	/// Creates a procedurally generated box from unbranded mesh-space half extents.
	pub fn r#box(size: Vec3f) -> Self {
		Self::generated(Box::new(BoxMeshGenerator::from_size(size)))
	}

	/// Returns the geometry source consumed by the rendering pipeline.
	pub fn source(&self) -> &MeshSource {
		&self.source
	}
}

/// The `MeshSource` enum selects resource-backed or procedurally generated geometry.
#[derive(Clone)]
pub enum MeshSource {
	Resource(Arc<str>),
	Generated(Box<dyn MeshGenerator>),
}

/// The `MeshKey` enum identifies one geometry source independently of renderer storage or scene instances.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MeshKey {
	/// A resource mesh, by the hash of its ID, so the key stays `Copy` and hashes as fast as a generated one.
	Resource(u64),
	Generated(u64),
}

impl std::fmt::Display for MeshKey {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Resource(hash) => write!(formatter, "resource:{hash:016x}"),
			Self::Generated(hash) => write!(formatter, "generated:{hash}"),
		}
	}
}

impl MeshSource {
	/// Returns the allocation-free logical key shared by every renderer implementation.
	pub(crate) fn key(&self) -> MeshKey {
		match self {
			Self::Resource(id) => MeshKey::Resource(resource_key(id)),
			Self::Generated(generator) => MeshKey::Generated(generator.hash()),
		}
	}

	pub fn sphere(radius: f32) -> Self {
		MeshSource::Generated(Box::new(SphereMeshGenerator::from_radius(radius)))
	}

	/// Creates a generated box from unbranded mesh-space half extents.
	pub fn r#box(size: Vec3f) -> Self {
		MeshSource::Generated(Box::new(BoxMeshGenerator::from_size(size)))
	}
}

/// Hashes a resource mesh's ID into its key, the way generators hash their parameters.
pub(crate) fn resource_key(id: &str) -> u64 {
	let mut hasher = std::hash::DefaultHasher::new();
	id.hash(&mut hasher);
	hasher.finish()
}

impl From<Box<dyn MeshGenerator>> for MeshSource {
	fn from(generator: Box<dyn MeshGenerator>) -> Self {
		MeshSource::Generated(generator)
	}
}
