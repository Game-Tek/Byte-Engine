pub mod r#box;
pub mod capsule;
pub mod sphere;

use std::borrow::Cow;

pub use r#box::BoxMeshGenerator;
pub use capsule::CapsuleMeshGenerator;
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

	/// Returns the lightmap UVs of the vertices, laid out as mesh processing lays out a baked mesh's
	/// `Vertex.LightmapUV` stream, or `None` for a mesh that lightmaps skip.
	///
	/// Both the renderer and the lightmap bake read them, so a lightmap lines up with the mesh the renderer draws.
	/// Include them in [`Self::hash`] when they can change for the same streams.
	fn lightmap_uvs(&self) -> Option<Cow<'_, [(f32, f32)]>> {
		None
	}

	/// Returns the meshlet indices of the vertices.
	fn meshlet_indices(&self) -> Option<Cow<'_, [u8]>> {
		None
	}

	/// Returns the material resource the mesh is drawn with, or `None` for the pipeline's default generated-mesh material.
	///
	/// Include it in [`Self::hash`], since meshes that share a hash share their prepared geometry and material.
	fn material(&self) -> Option<&str> {
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

	#[test]
	fn generated_lightmap_uvs_stay_inside_the_square_without_overlapping() {
		use maths_rs::Vec3f;

		use super::{BoxMeshGenerator, CapsuleMeshGenerator, MeshGenerator, SphereMeshGenerator};

		let generators: [Box<dyn MeshGenerator>; 3] = [
			Box::new(BoxMeshGenerator::from_size(Vec3f::new(2.0, 0.5, 1.0))),
			Box::new(SphereMeshGenerator::from_radius(1.5)),
			Box::new(CapsuleMeshGenerator::new(0.5, 2.0)),
		];
		const SIDE: usize = 128;
		for generator in &generators {
			let uvs = generator.lightmap_uvs().expect("generated meshes have lightmap UVs");
			assert_eq!(uvs.len(), generator.positions().len());
			// One texel of the default resolution lies between every UV and the edges.
			for &(u, v) in uvs.iter() {
				assert!((1.0 / 64.0 * 0.999..=1.0 - 1.0 / 64.0 * 0.999).contains(&u), "u {u}");
				assert!((1.0 / 64.0 * 0.999..=1.0 - 1.0 / 64.0 * 0.999).contains(&v), "v {v}");
			}
			// A texel center that lies strictly inside two triangles marks an overlap.
			let mut covered = vec![0u8; SIDE * SIDE];
			for triangle in generator.indices().as_chunks::<3>().0 {
				let [a, b, c] = triangle.map(|index| {
					let (u, v) = uvs[index as usize];
					[u * SIDE as f32, v * SIDE as f32]
				});
				let edge =
					|p: [f32; 2], q: [f32; 2], r: [f32; 2]| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
				for y in 0..SIDE {
					for x in 0..SIDE {
						let point = [x as f32 + 0.5, y as f32 + 0.5];
						let weights = [edge(a, b, point), edge(b, c, point), edge(c, a, point)];
						if weights.iter().all(|&w| w > 1e-3) || weights.iter().all(|&w| w < -1e-3) {
							covered[y * SIDE + x] += 1;
						}
					}
				}
			}
			assert!(covered.iter().all(|&count| count <= 1), "generated lightmap UVs overlap");
		}
	}
}
