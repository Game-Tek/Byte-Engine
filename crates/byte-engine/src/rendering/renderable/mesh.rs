//! Renderable mesh payloads and their geometry sources.
//!
//! Use [`RenderableMesh`] to publish resource-backed or generated geometry to a
//! rendering pipeline. Transform updates are published separately through the
//! world transform channel.

use std::hash::{Hash as _, Hasher as _};
use std::sync::Arc;

use maths_rs::Vec3f;

use crate::gameplay::Transform;
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

	/// Creates a renderable mesh that draws the resource mesh `id` once at each of `placements`, as one mesh.
	///
	/// Use it for copies that never move, such as a patch of grass: the renderer handles the patch as a whole instead
	/// of each copy. Place the entity at the origin, since the placements are world transforms. The renderer builds
	/// the merged geometry in memory, and holds only the detail level the patch is drawn at.
	pub fn merged(id: impl Into<Arc<str>>, placements: impl Into<Arc<[Transform]>>) -> Self {
		Self {
			source: MeshSource::Merged(MergedMesh::new(id.into(), placements.into())),
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

/// The `MeshSource` enum selects resource-backed, procedurally generated, or merged geometry.
#[derive(Clone)]
pub enum MeshSource {
	Resource(Arc<str>),
	Generated(Box<dyn MeshGenerator>),
	/// Copies of one resource mesh, drawn as one mesh; see [`RenderableMesh::merged`].
	Merged(MergedMesh),
}

/// The `MergedMesh` struct names the copies of a resource mesh that a renderer draws as one mesh.
///
/// Create one through [`RenderableMesh::merged`]. A renderer reads the mesh with [`Self::resource`] and repeats it at
/// each of [`Self::placements`].
#[derive(Clone)]
pub struct MergedMesh {
	resource: Arc<str>,
	placements: Arc<[Transform]>,
	/// Hashes the resource and every placement, worked out once since each key lookup would otherwise hash them all.
	hash: u64,
}

impl MergedMesh {
	fn new(resource: Arc<str>, placements: Arc<[Transform]>) -> Self {
		let mut hasher = std::hash::DefaultHasher::new();
		resource.hash(&mut hasher);
		for placement in placements.iter() {
			let (position, orientation) = (placement.get_position(), placement.get_orientation().to_array());
			let values = [position.x(), position.y(), position.z()]
				.into_iter()
				.chain(placement.scale().to_array())
				.chain(orientation);
			for value in values {
				value.to_bits().hash(&mut hasher);
			}
		}
		Self {
			resource,
			placements,
			hash: hasher.finish(),
		}
	}

	/// Returns the ID of the resource mesh every copy repeats.
	pub fn resource(&self) -> &Arc<str> {
		&self.resource
	}

	/// Returns the world transform of each copy.
	pub fn placements(&self) -> &Arc<[Transform]> {
		&self.placements
	}
}

/// The `MeshKey` enum identifies one geometry source independently of renderer storage or scene instances.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MeshKey {
	/// A resource mesh, by the hash of its ID, so the key stays `Copy` and hashes as fast as a generated one.
	Resource(u64),
	Generated(u64),
	/// A merged mesh, by the hash of its resource and placements.
	Merged(u64),
}

impl std::fmt::Display for MeshKey {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Resource(hash) => write!(formatter, "resource:{hash:016x}"),
			Self::Generated(hash) => write!(formatter, "generated:{hash}"),
			Self::Merged(hash) => write!(formatter, "merged:{hash:016x}"),
		}
	}
}

impl MeshSource {
	/// Returns the allocation-free logical key shared by every renderer implementation.
	pub(crate) fn key(&self) -> MeshKey {
		match self {
			Self::Resource(id) => MeshKey::Resource(resource_key(id)),
			Self::Generated(generator) => MeshKey::Generated(generator.hash()),
			Self::Merged(merged) => MeshKey::Merged(merged.hash),
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
