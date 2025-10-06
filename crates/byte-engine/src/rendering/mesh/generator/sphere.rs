use std::{borrow::Cow, hash::{Hash, Hasher as _}};

use math::{normalize, Vector3, Vector4};

use crate::rendering::{mesh::generator::MeshGenerator, renderable::mesh::MeshSource};

pub struct SphereMeshGenerator {
	radius: f32,
	segments: u32,
	rings: u32,
	vertex_positions: Vec<(f32, f32, f32)>,
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
	fn positions(&self) -> Cow<[(f32, f32, f32)]> {
		Cow::Borrowed(&self.vertex_positions)
	}

	fn indices(&self) -> Cow<[u32]> {
		let segments = self.segments;
		let rings = self.rings;
		let mut indices = Vec::new();

		for ring in 0..rings {
			for segment in 0..segments {
				let i = ring * (segments + 1) + segment;
				let j = (ring + 1) * (segments + 1) + segment;

				indices.push(i);
				indices.push(j);
				indices.push(i + 1);

				indices.push(j);
				indices.push(j + 1);
				indices.push(i + 1);
			}
		}

		Cow::Owned(indices)
	}

	fn normals(&self) -> Cow<[(f32, f32, f32)]> {
		let segments = self.segments as usize;
		let rings = self.rings as usize;
		let mut normals = Vec::new();

		let vertices = &self.vertex_positions;

		for ring in 0..rings {
			for segment in 0..segments {
				let i = ring * (segments + 1) + segment;
				let j = (ring + 1) * (segments + 1) + segment;

				let normal = normalize(Vector3::new(vertices[j].0 - vertices[i].0, vertices[j].1 - vertices[i].1, vertices[j].2 - vertices[i].2));
				normals.push((normal.x, normal.y, normal.z));
			}
		}

		Cow::Owned(normals)
	}

	fn tangents(&self) -> Cow<[Vector3]> {
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

	fn bitangents(&self) -> Cow<[Vector3]> {
    	let segments = self.segments;
		let rings = self.rings;
		let mut bitangents = Vec::new();

		for ring in 0..=rings {
			for segment in 0..=segments {
				let phi = 2.0 * std::f32::consts::PI * segment as f32 / segments as f32;
				let sin_phi = phi.sin();
				let cos_phi = phi.cos();

				// Bitangent is perpendicular to the tangent and normal in the YZ plane
				let bitangent = Vector3::new(0.0, cos_phi, sin_phi);
				bitangents.push(bitangent);
			}
		}

		Cow::Owned(bitangents)
	}

	fn colors(&self) -> Option<Cow<[Vector4]>> {
    	None
	}

	fn meshlet_indices(&self) -> Option<Cow<[u8]>> {
    	let segments = self.segments;
		let rings = self.rings;
		let mut indices = Vec::new();

		for ring in 0..rings {
			for segment in 0..segments {
				let i = (ring * (segments + 1) + segment) as u8;
				let j = ((ring + 1) * (segments + 1) + segment) as u8;

				indices.push(i);
				indices.push(j);
				indices.push(i + 1);

				indices.push(j);
				indices.push(j + 1);
				indices.push(i + 1);
			}
		}

		Some(Cow::Owned(indices))
	}

	fn uvs(&self) -> Cow<[(f32, f32)]> {
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

impl Into<Box<dyn MeshGenerator>> for SphereMeshGenerator {
	fn into(self) -> Box<dyn MeshGenerator> {
		Box::new(self)
	}
}

impl Into<MeshSource> for SphereMeshGenerator {
	fn into(self) -> MeshSource {
		Into::<Box<dyn MeshGenerator>>::into(self).into()
	}
}
