use std::{
	borrow::Cow,
	hash::{Hash as _, Hasher},
	sync::Arc,
};

use math::Vector3;

use crate::rendering::{mesh::generator::MeshGenerator, renderable::mesh::MeshSource};

pub struct BoxMeshGenerator {
	size: Vector3,
}

impl BoxMeshGenerator {
	/// Creates a box mesh generator with a default size of 1 by 1 by 1.
	pub fn new() -> Self {
		Self {
			size: Vector3::new(1.0, 1.0, 1.0),
		}
	}

	pub fn from_size(size: Vector3) -> Self {
		Self { size }
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
		Cow::Owned(vec![
			(0.0, 0.0, 1.0),
			(0.0, 0.0, 1.0),
			(0.0, 0.0, 1.0),
			(0.0, 0.0, 1.0),
			(0.0, 0.0, -1.0),
			(0.0, 0.0, -1.0),
			(0.0, 0.0, -1.0),
			(0.0, 0.0, -1.0),
			(0.0, -1.0, 0.0),
			(0.0, -1.0, 0.0),
			(0.0, -1.0, 0.0),
			(0.0, -1.0, 0.0),
			(0.0, 1.0, 0.0),
			(0.0, 1.0, 0.0),
			(0.0, 1.0, 0.0),
			(0.0, 1.0, 0.0),
			(1.0, 0.0, 0.0),
			(1.0, 0.0, 0.0),
			(1.0, 0.0, 0.0),
			(1.0, 0.0, 0.0),
			(-1.0, 0.0, 0.0),
			(-1.0, 0.0, 0.0),
			(-1.0, 0.0, 0.0),
			(-1.0, 0.0, 0.0),
		])
	}

	fn tangents(&self) -> Cow<'_, [Vector3]> {
		Cow::Owned(vec![
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(-1.0, 0.0, 0.0),
			Vector3::new(-1.0, 0.0, 0.0),
			Vector3::new(-1.0, 0.0, 0.0),
			Vector3::new(-1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(0.0, 0.0, -1.0),
			Vector3::new(0.0, 0.0, -1.0),
			Vector3::new(0.0, 0.0, -1.0),
			Vector3::new(0.0, 0.0, -1.0),
			Vector3::new(0.0, 0.0, 1.0),
			Vector3::new(0.0, 0.0, 1.0),
			Vector3::new(0.0, 0.0, 1.0),
			Vector3::new(0.0, 0.0, 1.0),
		])
	}

	fn bitangents(&self) -> std::borrow::Cow<'_, [Vector3]> {
		Cow::Owned(vec![
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 0.0, 1.0),
			Vector3::new(0.0, 0.0, 1.0),
			Vector3::new(0.0, 0.0, 1.0),
			Vector3::new(0.0, 0.0, 1.0),
			Vector3::new(0.0, 0.0, -1.0),
			Vector3::new(0.0, 0.0, -1.0),
			Vector3::new(0.0, 0.0, -1.0),
			Vector3::new(0.0, 0.0, -1.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
		])
	}

	fn uvs(&self) -> Cow<'_, [(f32, f32)]> {
		Cow::Borrowed(&[
			(0.0, 0.0),
			(1.0, 0.0),
			(1.0, 1.0),
			(0.0, 1.0),
			(0.0, 0.0),
			(1.0, 0.0),
			(1.0, 1.0),
			(0.0, 1.0),
			(0.0, 0.0),
			(1.0, 0.0),
			(1.0, 1.0),
			(0.0, 1.0),
			(0.0, 0.0),
			(1.0, 0.0),
			(1.0, 1.0),
			(0.0, 1.0),
			(0.0, 0.0),
			(1.0, 0.0),
			(1.0, 1.0),
			(0.0, 1.0),
			(0.0, 0.0),
			(1.0, 0.0),
			(1.0, 1.0),
			(0.0, 1.0),
		])
	}

	fn indices(&self) -> std::borrow::Cow<'_, [u32]> {
		Cow::Borrowed(&[
			0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 10, 9, 8, 11, 10, 12, 13, 14, 12, 14, 15, 16, 17, 18, 16, 18, 19, 20, 21,
			22, 20, 22, 23,
		])
	}

	fn hash(&self) -> u64 {
		let mut hasher = std::hash::DefaultHasher::new();
		(self.size.x.to_bits()).hash(&mut hasher);
		(self.size.y.to_bits()).hash(&mut hasher);
		(self.size.z.to_bits()).hash(&mut hasher);
		hasher.finish()
	}
}

impl From<BoxMeshGenerator> for Arc<dyn MeshGenerator> {
	fn from(val: BoxMeshGenerator) -> Self {
		Arc::new(val)
	}
}

impl From<BoxMeshGenerator> for MeshSource {
	fn from(val: BoxMeshGenerator) -> Self {
		Into::<Arc<dyn MeshGenerator>>::into(val).into()
	}
}

#[cfg(test)]
mod tests {
	use math::{cross, dot, length, Vector3};

	use super::BoxMeshGenerator;
	use crate::rendering::mesh::generator::MeshGenerator;

	fn vector(tuple: (f32, f32, f32)) -> Vector3 {
		Vector3::new(tuple.0, tuple.1, tuple.2)
	}

	#[test]
	fn box_streams_describe_six_independent_quad_faces() {
		let generator = BoxMeshGenerator::from_size(Vector3::new(2.0, 3.0, 4.0));
		let positions = generator.positions();
		let normals = generator.normals();
		let tangents = generator.tangents();
		let bitangents = generator.bitangents();
		let uvs = generator.uvs();
		let indices = generator.indices();

		assert_eq!(positions.len(), 24);
		assert_eq!(normals.len(), positions.len());
		assert_eq!(tangents.len(), positions.len());
		assert_eq!(bitangents.len(), positions.len());
		assert_eq!(uvs.len(), positions.len());
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
		let tangents = generator.tangents();
		let bitangents = generator.bitangents();
		let indices = generator.indices();

		for triangle in indices.chunks_exact(3) {
			let a = vector(positions[triangle[0] as usize]);
			let b = vector(positions[triangle[1] as usize]);
			let c = vector(positions[triangle[2] as usize]);
			let normal = vector(normals[triangle[0] as usize]);
			assert!(dot(cross(b - a, c - a), normal) > 0.0);
		}

		for ((normal, tangent), bitangent) in normals.iter().zip(tangents.iter()).zip(bitangents.iter()) {
			let normal = vector(*normal);
			assert!((length(normal) - 1.0).abs() < 1e-6);
			assert!(dot(normal, *tangent).abs() < 1e-6);
			assert!(dot(normal, *bitangent).abs() < 1e-6);
			assert!(dot(cross(*tangent, *bitangent), normal) > 0.9999);
		}
	}

	#[test]
	fn hash_changes_for_each_size_axis() {
		let base = BoxMeshGenerator::from_size(Vector3::new(1.0, 1.0, 1.0)).hash();
		assert_eq!(base, BoxMeshGenerator::new().hash());
		assert_ne!(base, BoxMeshGenerator::from_size(Vector3::new(2.0, 1.0, 1.0)).hash());
		assert_ne!(base, BoxMeshGenerator::from_size(Vector3::new(1.0, 2.0, 1.0)).hash());
		assert_ne!(base, BoxMeshGenerator::from_size(Vector3::new(1.0, 1.0, 2.0)).hash());
		assert!(BoxMeshGenerator::new().colors().is_none());
		assert!(BoxMeshGenerator::new().meshlet_indices().is_none());
	}
}
