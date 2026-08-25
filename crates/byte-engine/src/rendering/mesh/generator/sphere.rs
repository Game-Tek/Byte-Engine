use std::{
	borrow::Cow,
	hash::{Hash, Hasher as _},
	sync::Arc,
};

use maths_rs::{Vec3f, Vec4f, cross, normalize};

use crate::rendering::{mesh::generator::MeshGenerator, renderable::mesh::MeshSource};

/// The `SphereMeshGenerator` struct provides unbranded mesh-space streams for a UV sphere.
pub struct SphereMeshGenerator {
	radius: f32,
	segments: u32,
	rings: u32,
	vertex_positions: Vec<(f32, f32, f32)>,
}

impl Default for SphereMeshGenerator {
	fn default() -> Self {
		Self::new()
	}
}

impl SphereMeshGenerator {
	/// Creates a unit-radius sphere mesh generator.
	pub fn new() -> Self {
		Self::from_radius(1.0)
	}

	/// Creates a sphere mesh generator with the supplied mesh-space radius.
	pub fn from_radius(radius: f32) -> Self {
		let segments = 8;
		let rings = 8;
		let mut vertex_positions = Vec::with_capacity(((rings + 1) * (segments + 1)) as usize);
		for ring in 0..=rings {
			let theta = std::f32::consts::PI * ring as f32 / rings as f32;
			for segment in 0..=segments {
				let phi = 2.0 * std::f32::consts::PI * segment as f32 / segments as f32;
				vertex_positions.push((
					radius * theta.sin() * phi.cos(),
					radius * theta.cos(),
					radius * theta.sin() * phi.sin(),
				));
			}
		}
		Self {
			radius,
			segments,
			rings,
			vertex_positions,
		}
	}
}

impl MeshGenerator for SphereMeshGenerator {
	fn positions(&self) -> Cow<'_, [(f32, f32, f32)]> {
		Cow::Borrowed(&self.vertex_positions)
	}

	fn indices(&self) -> Cow<'_, [u32]> {
		let mut indices = Vec::with_capacity((self.rings * self.segments * 6) as usize);
		for ring in 0..self.rings {
			for segment in 0..self.segments {
				let top_left = ring * (self.segments + 1) + segment;
				let bottom_left = (ring + 1) * (self.segments + 1) + segment;
				indices.extend_from_slice(&[
					top_left,
					top_left + 1,
					bottom_left,
					bottom_left,
					top_left + 1,
					bottom_left + 1,
				]);
			}
		}
		Cow::Owned(indices)
	}

	fn normals(&self) -> Cow<'_, [(f32, f32, f32)]> {
		Cow::Owned(
			self.vertex_positions
				.iter()
				.map(|&(x, y, z)| {
					let normal = normalize(Vec3f::new(x, y, z));
					(normal.x, normal.y, normal.z)
				})
				.collect(),
		)
	}

	fn tangents(&self) -> Cow<'_, [Vec3f]> {
		Cow::Owned(
			(0..=self.rings)
				.flat_map(|_| {
					(0..=self.segments).map(|segment| {
						let phi = 2.0 * std::f32::consts::PI * segment as f32 / self.segments as f32;
						Vec3f::new(-phi.sin(), 0.0, phi.cos())
					})
				})
				.collect(),
		)
	}

	fn bitangents(&self) -> Cow<'_, [Vec3f]> {
		Cow::Owned(
			(0..=self.rings)
				.flat_map(|ring| {
					let theta = std::f32::consts::PI * ring as f32 / self.rings as f32;
					(0..=self.segments).map(move |segment| {
						let phi = 2.0 * std::f32::consts::PI * segment as f32 / self.segments as f32;
						cross(
							Vec3f::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin()),
							Vec3f::new(-phi.sin(), 0.0, phi.cos()),
						)
					})
				})
				.collect(),
		)
	}

	fn colors(&self) -> Option<Cow<'_, [Vec4f]>> {
		None
	}

	fn meshlet_indices(&self) -> Option<Cow<'_, [u8]>> {
		debug_assert!(
			self.segments > 0 && self.rings > 0,
			"Sphere topology is empty. The most likely cause is constructing a generator with zero segments or rings."
		);
		debug_assert!(
			self.rings
				.checked_add(1)
				.zip(self.segments.checked_add(1))
				.and_then(|(rings, segments)| rings.checked_mul(segments))
				.is_some_and(|vertex_count| vertex_count <= u8::MAX as u32 + 1),
			"Sphere meshlet index exceeds u8. The most likely cause is increasing sphere tessellation without widening meshlet indices."
		);
		Some(Cow::Owned(self.indices().iter().map(|index| *index as u8).collect()))
	}

	fn uvs(&self) -> Cow<'_, [(f32, f32)]> {
		Cow::Owned(
			(0..=self.rings)
				.flat_map(|ring| {
					(0..=self.segments)
						.map(move |segment| (segment as f32 / self.segments as f32, ring as f32 / self.rings as f32))
				})
				.collect(),
		)
	}

	fn hash(&self) -> u64 {
		let mut hasher = std::hash::DefaultHasher::new();
		self.radius.to_bits().hash(&mut hasher);
		self.rings.hash(&mut hasher);
		self.segments.hash(&mut hasher);
		hasher.finish()
	}
}

impl From<SphereMeshGenerator> for Arc<dyn MeshGenerator> {
	fn from(value: SphereMeshGenerator) -> Self {
		Arc::new(value)
	}
}

impl From<SphereMeshGenerator> for MeshSource {
	fn from(value: SphereMeshGenerator) -> Self {
		Into::<Arc<dyn MeshGenerator>>::into(value).into()
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::{Vec3f, cross, dot, length};

	use super::SphereMeshGenerator;
	use crate::rendering::mesh::generator::MeshGenerator;

	fn vector(value: (f32, f32, f32)) -> Vec3f {
		Vec3f::new(value.0, value.1, value.2)
	}

	#[test]
	fn generated_sphere_has_consistent_stream_lengths_and_valid_indices() {
		let sphere = SphereMeshGenerator::from_radius(2.0);
		let positions = sphere.positions();

		assert_eq!(positions.len(), 81);
		assert_eq!(sphere.normals().len(), positions.len());
		assert_eq!(sphere.tangents().len(), positions.len());
		assert_eq!(sphere.bitangents().len(), positions.len());
		assert_eq!(sphere.uvs().len(), positions.len());
		assert_eq!(sphere.indices().len(), 8 * 8 * 6);
		assert!(sphere.indices().iter().all(|index| (*index as usize) < positions.len()));
	}

	#[test]
	fn every_vertex_has_a_right_handed_orthonormal_frame() {
		let sphere = SphereMeshGenerator::from_radius(2.5);
		for (((position, normal), tangent), bitangent) in sphere
			.positions()
			.iter()
			.zip(sphere.normals().iter())
			.zip(sphere.tangents().iter())
			.zip(sphere.bitangents().iter())
		{
			let position = vector(*position);
			let normal = vector(*normal);

			assert!((length(position) - 2.5).abs() < 1e-4);
			assert!((length(normal) - 1.0).abs() < 1e-4);
			assert!((length(*tangent) - 1.0).abs() < 1e-4);
			assert!((length(*bitangent) - 1.0).abs() < 1e-4);
			assert!(dot(normal, *tangent).abs() < 1e-4);
			assert!(dot(normal, *bitangent).abs() < 1e-4);
			assert!(dot(*tangent, *bitangent).abs() < 1e-4);
			assert!(dot(cross(*tangent, *bitangent), normal) > 0.9999);
		}
	}

	#[test]
	fn uvs_cover_each_row_and_duplicate_the_longitude_seam() {
		let sphere = SphereMeshGenerator::new();
		let positions = sphere.positions();
		let uvs = sphere.uvs();

		for row in 0..=8 {
			let first = row * 9;
			let last = first + 8;

			assert_eq!(uvs[first], (0.0, row as f32 / 8.0));
			assert_eq!(uvs[last], (1.0, row as f32 / 8.0));
			assert!(length(vector(positions[first]) - vector(positions[last])) < 1e-4);
		}
	}

	#[test]
	fn meshlet_indices_match_primary_topology_without_truncation() {
		let sphere = SphereMeshGenerator::new();
		let indices = sphere.indices();
		let meshlet_indices = sphere.meshlet_indices().expect("sphere meshlet topology");

		assert_eq!(meshlet_indices.len(), indices.len());
		assert!(indices.iter().all(|index| *index <= u8::MAX as u32));
		assert!(
			indices
				.iter()
				.zip(meshlet_indices.iter())
				.all(|(index, meshlet_index)| *index == u32::from(*meshlet_index))
		);
	}

	#[test]
	fn hash_is_stable_for_equal_geometry_and_changes_with_radius() {
		assert_eq!(
			SphereMeshGenerator::from_radius(1.0).hash(),
			SphereMeshGenerator::from_radius(1.0).hash()
		);
		assert_ne!(
			SphereMeshGenerator::from_radius(1.0).hash(),
			SphereMeshGenerator::from_radius(2.0).hash()
		);
		assert!(SphereMeshGenerator::new().colors().is_none());
	}
}
