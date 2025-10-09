pub mod sphere;
pub mod r#box;

pub use sphere::SphereMeshGenerator;
pub use r#box::BoxMeshGenerator;

use std::borrow::Cow;

use math::{Vector3, Vector4};

/// The `MeshGenerator` trait defines a mesh generator capable of serving as a source of mesh data.
pub trait MeshGenerator: Send {
	/// Returns the positions of the vertices.
	fn positions(&self) -> Cow<'_, [(f32, f32, f32)]>;

	/// Returns the normals of the vertices.
	fn normals(&self) -> Cow<'_, [(f32, f32, f32)]>;

	/// Returns the UV coordinates of the vertices.
	fn uvs(&self) -> Cow<'_, [(f32, f32)]>;

	/// Returns the indices of the vertices.
	fn indices(&self) -> Cow<'_, [u32]>;

	/// Returns the tangents of the vertices.
	fn tangents(&self) -> Cow<'_, [Vector3]>;

	/// Returns the bitangents of the vertices.
	fn bitangents(&self) -> Cow<'_, [Vector3]>;

	/// Returns the colors of the vertices.
	fn colors(&self) -> Option<Cow<'_, [Vector4]>> { None }

	/// Returns the meshlet indices of the vertices.
	fn meshlet_indices(&self) -> Option<Cow<'_, [u8]>> { None }

	/// Returns a hash that uniquely identifies the mesh. If the consumer of this generator already has a mesh whose id matches this it can safely reuse the existing mesh.
	fn hash(&self) -> u64;
}
