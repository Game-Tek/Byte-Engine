#![expect(
	clippy::disallowed_types,
	reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"
)]
use std::{
	borrow::Cow,
	hash::{Hash, Hasher as _},
	sync::Arc,
};

use maths_rs::{Vec3f, Vec4f, cross, normalize};
use resource_management::processors::mesh::{LightmapUvSettings, inset_lightmap_uv};

use crate::rendering::{mesh::generator::MeshGenerator, renderable::mesh::MeshSource};

/// The `SphereMeshGenerator` struct provides unbranded mesh-space streams for a UV sphere.
#[derive(Clone)]
pub struct SphereMeshGenerator {
	radius: f32,
	segments: u32,
	rings: u32,
	vertex_positions: Vec<(f32, f32, f32)>,
	// Shared, not owned: the bus clones this ID into every listener, and the render thread, loader tasks, and inspector
	// each keep it in long-lived maps. A numeric key cannot replace the string, since debug builds bake from it on demand.
	material: Option<Arc<str>>,
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
			material: None,
		}
	}

	/// Draws the sphere with the material resource `material` instead of the default generated-mesh material, such as
	/// to preview a material on it.
	pub fn with_material(mut self, material: impl Into<Arc<str>>) -> Self {
		self.material = Some(material.into());
		self
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

	fn lightmap_uvs(&self) -> Option<Cow<'_, [(f32, f32)]>> {
		// The latitude-longitude UVs already form one chart without overlaps, which the border moves off the edges.
		let settings = LightmapUvSettings::default();
		Some(Cow::Owned(
			self.uvs()
				.iter()
				.map(|&(u, v)| {
					let [u, v] = inset_lightmap_uv([u, v], settings);
					(u, v)
				})
				.collect(),
		))
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
		self.material.hash(&mut hasher);
		hasher.finish()
	}

	fn material(&self) -> Option<&str> {
		self.material.as_deref()
	}

	fn clone_box(&self) -> Box<dyn MeshGenerator> {
		Box::new(self.clone())
	}
}

impl From<SphereMeshGenerator> for Box<dyn MeshGenerator> {
	fn from(value: SphereMeshGenerator) -> Self {
		Box::new(value)
	}
}

impl From<SphereMeshGenerator> for MeshSource {
	fn from(value: SphereMeshGenerator) -> Self {
		MeshSource::Generated(Box::new(value))
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::{Vec3f, length};

	use super::SphereMeshGenerator;
	use crate::rendering::mesh::generator::MeshGenerator;

	fn vector(value: (f32, f32, f32)) -> Vec3f {
		Vec3f::new(value.0, value.1, value.2)
	}

	/// Every vertex lies one radius from the center, and each row's UVs span the square, with the seam's vertex
	/// duplicated.
	#[test]
	fn vertices_lie_on_the_radius_and_uvs_duplicate_the_longitude_seam() {
		let large = SphereMeshGenerator::from_radius(2.5);
		assert!(
			large
				.positions()
				.iter()
				.all(|position| (length(vector(*position)) - 2.5).abs() < 1e-4)
		);

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

	/// Meshes that share a hash are one mesh, so the hash follows the radius and the material a sphere names.
	#[test]
	fn the_hash_follows_the_radius_and_the_material() {
		let plain = SphereMeshGenerator::new();
		let painted = SphereMeshGenerator::new().with_material("paint.bema");
		assert_eq!(plain.hash(), SphereMeshGenerator::from_radius(1.0).hash());
		assert_ne!(plain.hash(), SphereMeshGenerator::from_radius(2.0).hash());
		assert_eq!((plain.material(), painted.material()), (None, Some("paint.bema")));
		assert_ne!(plain.hash(), painted.hash());
		assert!(plain.colors().is_none());
	}
}
