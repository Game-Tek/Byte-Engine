use std::{
	borrow::Cow,
	hash::{Hash, Hasher as _},
	sync::Arc,
};

use math::{cross, normalize, Vector3, Vector4};

use crate::rendering::{mesh::generator::MeshGenerator, renderable::mesh::MeshSource};

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
	pub fn new() -> Self {
		SphereMeshGenerator::from_radius(1.0)
	}

	pub fn from_radius(radius: f32) -> Self {
		let segments = 8;
		let rings = 8;

		let mut vertices = Vec::new();

		for ring in 0..=rings {
			let theta = std::f32::consts::PI * ring as f32 / rings as f32;
			let sin_theta = theta.sin();
			let cos_theta = theta.cos();

			for segment in 0..=segments {
				let phi = 2.0 * std::f32::consts::PI * segment as f32 / segments as f32;
				let sin_phi = phi.sin();
				let cos_phi = phi.cos();

				let x = radius * sin_theta * cos_phi;
				let y = radius * cos_theta;
				let z = radius * sin_theta * sin_phi;

				vertices.push((x, y, z));
			}
		}

		SphereMeshGenerator {
			radius,
			segments,
			rings,
			vertex_positions: vertices,
		}
	}
}

impl MeshGenerator for SphereMeshGenerator {
	fn positions(&self) -> Cow<'_, [(f32, f32, f32)]> {
		Cow::Borrowed(&self.vertex_positions)
	}

	fn indices(&self) -> Cow<'_, [u32]> {
		let segments = self.segments;
		let rings = self.rings;
		let mut indices = Vec::new();

		for ring in 0..rings {
			for segment in 0..segments {
				let i = ring * (segments + 1) + segment;
				let j = (ring + 1) * (segments + 1) + segment;

				indices.push(i);
				indices.push(i + 1);
				indices.push(j);

				indices.push(j);
				indices.push(i + 1);
				indices.push(j + 1);
			}
		}

		Cow::Owned(indices)
	}

	fn normals(&self) -> Cow<'_, [(f32, f32, f32)]> {
		Cow::Owned(
			self.vertex_positions
				.iter()
				.map(|&(x, y, z)| {
					let normal = normalize(Vector3::new(x, y, z));
					(normal.x, normal.y, normal.z)
				})
				.collect(),
		)
	}

	fn tangents(&self) -> Cow<'_, [Vector3]> {
		let segments = self.segments;
		let rings = self.rings;
		let mut tangents = Vec::new();

		for ring in 0..=rings {
			for segment in 0..=segments {
				let phi = 2.0 * std::f32::consts::PI * segment as f32 / segments as f32;
				let sin_phi = phi.sin();
				let cos_phi = phi.cos();

				// Tangent is perpendicular to the radial direction in the XZ plane
				let tangent = Vector3::new(-sin_phi, 0.0, cos_phi);
				tangents.push(tangent);
			}
		}

		Cow::Owned(tangents)
	}

	fn bitangents(&self) -> Cow<'_, [Vector3]> {
		let mut bitangents = Vec::with_capacity(((self.rings + 1) * (self.segments + 1)) as usize);

		for ring in 0..=self.rings {
			let theta = std::f32::consts::PI * ring as f32 / self.rings as f32;
			let normal_y = theta.cos();
			let radial = theta.sin();

			for segment in 0..=self.segments {
				let phi = 2.0 * std::f32::consts::PI * segment as f32 / self.segments as f32;
				let normal = Vector3::new(radial * phi.cos(), normal_y, radial * phi.sin());
				let tangent = Vector3::new(-phi.sin(), 0.0, phi.cos());

				// Keep the generated tangent frame right-handed: tangent x bitangent = normal.
				bitangents.push(cross(normal, tangent));
			}
		}

		Cow::Owned(bitangents)
	}

	fn colors(&self) -> Option<Cow<'_, [Vector4]>> {
		None
	}

	fn meshlet_indices(&self) -> Option<Cow<'_, [u8]>> {
		let segments = self.segments;
		let rings = self.rings;
		let mut indices = Vec::new();

		for ring in 0..rings {
			for segment in 0..segments {
				let i = (ring * (segments + 1) + segment) as u8;
				let j = ((ring + 1) * (segments + 1) + segment) as u8;

				indices.push(i);
				indices.push(i + 1);
				indices.push(j);

				indices.push(j);
				indices.push(i + 1);
				indices.push(j + 1);
			}
		}

		Some(Cow::Owned(indices))
	}

	fn uvs(&self) -> Cow<'_, [(f32, f32)]> {
		let segments = self.segments;
		let rings = self.rings;
		let mut uvs = Vec::new();

		for ring in 0..=rings {
			let v = ring as f32 / rings as f32;

			for segment in 0..=segments {
				let u = segment as f32 / segments as f32;
				uvs.push((u, v));
			}
		}

		Cow::Owned(uvs)
	}

	fn hash(&self) -> u64 {
		let mut hasher = std::hash::DefaultHasher::new();
		(self.radius.to_bits()).hash(&mut hasher);
		self.rings.hash(&mut hasher);
		self.segments.hash(&mut hasher);
		hasher.finish()
	}
}

impl From<SphereMeshGenerator> for Arc<dyn MeshGenerator> {
	fn from(val: SphereMeshGenerator) -> Self {
		Arc::new(val)
	}
}

impl From<SphereMeshGenerator> for MeshSource {
	fn from(val: SphereMeshGenerator) -> Self {
		Into::<Arc<dyn MeshGenerator>>::into(val).into()
	}
}

#[cfg(test)]
mod tests {
	use math::{cross, dot, length, Vector3};

	use super::SphereMeshGenerator;
	use crate::rendering::mesh::generator::MeshGenerator;

	fn vector(tuple: (f32, f32, f32)) -> Vector3 {
		Vector3::new(tuple.0, tuple.1, tuple.2)
	}

	#[test]
	fn generated_sphere_has_consistent_stream_lengths_and_valid_indices() {
		let sphere = SphereMeshGenerator::from_radius(2.0);
		let positions = sphere.positions();
		let normals = sphere.normals();
		let tangents = sphere.tangents();
		let bitangents = sphere.bitangents();
		let uvs = sphere.uvs();
		let indices = sphere.indices();

		assert_eq!(positions.len(), 81);
		assert_eq!(normals.len(), positions.len());
		assert_eq!(tangents.len(), positions.len());
		assert_eq!(bitangents.len(), positions.len());
		assert_eq!(uvs.len(), positions.len());
		assert_eq!(indices.len(), 8 * 8 * 6);
		assert!(indices.iter().all(|index| (*index as usize) < positions.len()));
	}

	#[test]
	fn every_vertex_lies_on_radius_and_has_a_right_handed_orthonormal_frame() {
		let sphere = SphereMeshGenerator::from_radius(2.5);
		let positions = sphere.positions();
		let normals = sphere.normals();
		let tangents = sphere.tangents();
		let bitangents = sphere.bitangents();

		for (((position, normal), tangent), bitangent) in positions
			.iter()
			.zip(normals.iter())
			.zip(tangents.iter())
			.zip(bitangents.iter())
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
			let first_position = vector(positions[first]);
			let last_position = vector(positions[last]);
			assert!(length(first_position - last_position) < 1e-4);
		}
	}

	#[test]
	fn meshlet_indices_match_the_primary_topology_without_truncation() {
		let sphere = SphereMeshGenerator::new();
		let indices = sphere.indices();
		let meshlet_indices = sphere.meshlet_indices().expect("sphere meshlet topology");

		assert_eq!(meshlet_indices.len(), indices.len());
		assert!(indices.iter().all(|index| *index <= u8::MAX as u32));
		assert!(indices
			.iter()
			.zip(meshlet_indices.iter())
			.all(|(index, meshlet_index)| *index == u32::from(*meshlet_index)));
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
