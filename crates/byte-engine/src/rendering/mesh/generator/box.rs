use std::{
	borrow::Cow,
	hash::{Hash as _, Hasher},
	sync::Arc,
};

use maths_rs::Vec3f;

use crate::rendering::{mesh::generator::MeshGenerator, renderable::mesh::MeshSource};

/// The `BoxMeshGenerator` struct provides unbranded mesh-space streams for a six-faced box.
pub struct BoxMeshGenerator {
	size: Vec3f,
}

impl BoxMeshGenerator {
	/// Creates a box mesh generator with a default half extent of one on each axis.
	pub fn new() -> Self {
		Self {
			size: Vec3f::new(1.0, 1.0, 1.0),
		}
	}

	/// Creates a box mesh generator from unbranded mesh-space half extents.
	pub fn from_size(size: Vec3f) -> Self {
		Self { size }
	}
}

impl Default for BoxMeshGenerator {
	fn default() -> Self {
		Self::new()
	}
}

impl MeshGenerator for BoxMeshGenerator {
	fn positions(&self) -> Cow<'_, [(f32, f32, f32)]> {
		let x = self.size.x;
		let y = self.size.y;
		let z = self.size.z;
		Cow::Owned(vec![
			(-x, -y, z),
			(x, -y, z),
			(x, y, z),
			(-x, y, z),
			(x, -y, -z),
			(-x, -y, -z),
			(-x, y, -z),
			(x, y, -z),
			(-x, -y, z),
			(x, -y, z),
			(x, -y, -z),
			(-x, -y, -z),
			(-x, y, z),
			(x, y, z),
			(x, y, -z),
			(-x, y, -z),
			(x, -y, z),
			(x, -y, -z),
			(x, y, -z),
			(x, y, z),
			(-x, -y, -z),
			(-x, -y, z),
			(-x, y, z),
			(-x, y, -z),
		])
	}

	fn normals(&self) -> Cow<'_, [(f32, f32, f32)]> {
		Cow::Owned(
			vec![(0.0, 0.0, 1.0); 4]
				.into_iter()
				.chain(vec![(0.0, 0.0, -1.0); 4])
				.chain(vec![(0.0, -1.0, 0.0); 4])
				.chain(vec![(0.0, 1.0, 0.0); 4])
				.chain(vec![(1.0, 0.0, 0.0); 4])
				.chain(vec![(-1.0, 0.0, 0.0); 4])
				.collect(),
		)
	}

	fn tangents(&self) -> Cow<'_, [Vec3f]> {
		Cow::Owned(
			vec![Vec3f::new(1.0, 0.0, 0.0); 4]
				.into_iter()
				.chain(vec![Vec3f::new(-1.0, 0.0, 0.0); 4])
				.chain(vec![Vec3f::new(1.0, 0.0, 0.0); 8])
				.chain(vec![Vec3f::new(0.0, 0.0, -1.0); 4])
				.chain(vec![Vec3f::new(0.0, 0.0, 1.0); 4])
				.collect(),
		)
	}

	fn bitangents(&self) -> Cow<'_, [Vec3f]> {
		Cow::Owned(
			vec![Vec3f::new(0.0, 1.0, 0.0); 8]
				.into_iter()
				.chain(vec![Vec3f::new(0.0, 0.0, 1.0); 4])
				.chain(vec![Vec3f::new(0.0, 0.0, -1.0); 4])
				.chain(vec![Vec3f::new(0.0, 1.0, 0.0); 8])
				.collect(),
		)
	}

	fn uvs(&self) -> Cow<'_, [(f32, f32)]> {
		Cow::Owned(
			(0..6)
				.flat_map(|_| [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])
				.collect(),
		)
	}

	fn indices(&self) -> Cow<'_, [u32]> {
		Cow::Borrowed(&[
			0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 10, 9, 8, 11, 10, 12, 13, 14, 12, 14, 15, 16, 17, 18, 16, 18, 19, 20, 21,
			22, 20, 22, 23,
		])
	}

	fn hash(&self) -> u64 {
		let mut hasher = std::hash::DefaultHasher::new();
		self.size.x.to_bits().hash(&mut hasher);
		self.size.y.to_bits().hash(&mut hasher);
		self.size.z.to_bits().hash(&mut hasher);
		hasher.finish()
	}
}

impl From<BoxMeshGenerator> for Arc<dyn MeshGenerator> {
	fn from(value: BoxMeshGenerator) -> Self {
		Arc::new(value)
	}
}

impl From<BoxMeshGenerator> for MeshSource {
	fn from(value: BoxMeshGenerator) -> Self {
		Into::<Arc<dyn MeshGenerator>>::into(value).into()
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::{cross, dot, length, Vec3f};

	use super::BoxMeshGenerator;
	use crate::rendering::mesh::generator::MeshGenerator;

	fn vector(value: (f32, f32, f32)) -> Vec3f {
		Vec3f::new(value.0, value.1, value.2)
	}

	#[test]
	fn box_streams_describe_six_independent_quad_faces() {
		let generator = BoxMeshGenerator::from_size(Vec3f::new(2.0, 3.0, 4.0));
		let positions = generator.positions();

		assert_eq!(positions.len(), 24);
		assert_eq!(generator.normals().len(), positions.len());
		assert_eq!(generator.tangents().len(), positions.len());
		assert_eq!(generator.bitangents().len(), positions.len());
		assert_eq!(generator.uvs().len(), positions.len());
		let indices = generator.indices();

		assert_eq!(indices.len(), 36);
		assert!(indices.iter().all(|index| (*index as usize) < positions.len()));
		assert!(positions
			.iter()
			.all(|&(x, y, z)| x.abs() == 2.0 && y.abs() == 3.0 && z.abs() == 4.0));
	}

	#[test]
	fn triangle_winding_and_tangent_frames_point_outward() {
		let generator = BoxMeshGenerator::new();
		let positions = generator.positions();
		let normals = generator.normals();
		for triangle in generator.indices().chunks_exact(3) {
			let a = vector(positions[triangle[0] as usize]);
			let b = vector(positions[triangle[1] as usize]);
			let c = vector(positions[triangle[2] as usize]);

			assert!(dot(cross(b - a, c - a), vector(normals[triangle[0] as usize])) > 0.0);
		}
		for ((normal, tangent), bitangent) in normals
			.iter()
			.zip(generator.tangents().iter())
			.zip(generator.bitangents().iter())
		{
			let normal = vector(*normal);

			assert!((length(normal) - 1.0).abs() < 1e-6);
			assert!(dot(normal, *tangent).abs() < 1e-6);
			assert!(dot(normal, *bitangent).abs() < 1e-6);
			assert!(dot(cross(*tangent, *bitangent), normal) > 0.9999);
		}
	}

	#[test]
	fn hash_changes_for_each_size_axis() {
		let base = BoxMeshGenerator::from_size(Vec3f::new(1.0, 1.0, 1.0)).hash();

		assert_eq!(base, BoxMeshGenerator::new().hash());
		assert_ne!(base, BoxMeshGenerator::from_size(Vec3f::new(2.0, 1.0, 1.0)).hash());
		assert_ne!(base, BoxMeshGenerator::from_size(Vec3f::new(1.0, 2.0, 1.0)).hash());
		assert_ne!(base, BoxMeshGenerator::from_size(Vec3f::new(1.0, 1.0, 2.0)).hash());
		assert!(BoxMeshGenerator::new().colors().is_none());
		assert!(BoxMeshGenerator::new().meshlet_indices().is_none());
	}
}
