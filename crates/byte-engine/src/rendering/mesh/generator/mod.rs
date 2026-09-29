pub mod r#box;
pub mod sphere;

use std::borrow::Cow;

pub use r#box::BoxMeshGenerator;
use maths_rs::{Vec3f, Vec4f};
pub use sphere::SphereMeshGenerator;

/// The `MeshGenerator` trait defines a mesh generator capable of serving as a source of mesh data.
pub trait MeshGenerator: Send + Sync {
	/// Returns the positions of the vertices.
	fn positions(&self) -> Cow<'_, [(f32, f32, f32)]>;

	/// Returns the normals of the vertices.
	fn normals(&self) -> Cow<'_, [(f32, f32, f32)]>;

	/// Returns the UV coordinates of the vertices.
	fn uvs(&self) -> Cow<'_, [(f32, f32)]>;

	/// Returns the indices of the vertices.
	fn indices(&self) -> Cow<'_, [u32]>;

	/// Returns the tangents of the vertices.
	fn tangents(&self) -> Cow<'_, [Vec3f]>;

	/// Returns the bitangents of the vertices.
	fn bitangents(&self) -> Cow<'_, [Vec3f]>;

	/// Returns the colors of the vertices.
	fn colors(&self) -> Option<Cow<'_, [Vec4f]>> {
		None
	}

	/// Returns the meshlet indices of the vertices.
	fn meshlet_indices(&self) -> Option<Cow<'_, [u8]>> {
		None
	}

	/// Returns a hash that uniquely identifies the mesh. If the consumer of this generator already has a mesh whose id matches this it can safely reuse the existing mesh.
	fn hash(&self) -> u64;

	/// Copies this generator into a new single-owner box.
	///
	/// [`MeshSource`](crate::rendering::renderable::mesh::MeshSource) uses this to stay
	/// cloneable, because message channels hand each listener its own copy.
	fn clone_box(&self) -> Box<dyn MeshGenerator>;
}

impl Clone for Box<dyn MeshGenerator> {
	fn clone(&self) -> Self {
		self.clone_box()
	}
}

/// The `GeneratedIndexError` enum reports why a generator's indices cannot be drawn as a 16-bit triangle list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GeneratedIndexError {
	/// The index count is not a multiple of three.
	NotTriangles,
	/// An index addresses a vertex the generator did not return.
	OutOfRange { index: u32, vertex_count: usize },
	/// An index does not fit in 16 bits.
	IndexLimit { index: u32 },
}

impl std::fmt::Display for GeneratedIndexError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NotTriangles => f.write_str(
				"Generated mesh indices are invalid. The most likely cause is that the mesh generator returned a triangle list whose index count is not divisible by three.",
			),
			Self::OutOfRange { index, vertex_count } => write!(
				f,
				"Generated mesh index {index} references a missing vertex. The most likely cause is that the generator returned an index outside its {vertex_count} positions."
			),
			Self::IndexLimit { index } => write!(
				f,
				"Generated mesh index {index} exceeds the u16 vertex-index limit. The most likely cause is that one generated primitive contains more than 65536 vertices."
			),
		}
	}
}

/// Checks that generated `indices` form a triangle list over `vertex_count` vertices that 16-bit indices address.
///
/// Every renderer that draws generated meshes validates them here, then applies only its own limits, so the
/// narrowing `index as u16` is lossless afterwards.
pub(crate) fn validate_triangle_indices(indices: &[u32], vertex_count: usize) -> Result<(), GeneratedIndexError> {
	if !indices.len().is_multiple_of(3) {
		return Err(GeneratedIndexError::NotTriangles);
	}
	for &index in indices {
		if index as usize >= vertex_count {
			return Err(GeneratedIndexError::OutOfRange { index, vertex_count });
		}
		if u16::try_from(index).is_err() {
			return Err(GeneratedIndexError::IndexLimit { index });
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{GeneratedIndexError, validate_triangle_indices};

	#[test]
	fn generated_indices_must_be_addressable_16_bit_triangles() {
		assert_eq!(validate_triangle_indices(&[0, 2, 1], 3), Ok(()));
		assert_eq!(validate_triangle_indices(&[0, 1], 3), Err(GeneratedIndexError::NotTriangles));
		assert_eq!(
			validate_triangle_indices(&[0, 1, 3], 3),
			Err(GeneratedIndexError::OutOfRange {
				index: 3,
				vertex_count: 3
			})
		);
		let beyond_u16 = u16::MAX as u32 + 1;
		assert_eq!(
			validate_triangle_indices(&[0, 1, beyond_u16], beyond_u16 as usize + 1),
			Err(GeneratedIndexError::IndexLimit { index: beyond_u16 })
		);
	}
}
