//! Mesh component module

use crate::core::EntityHandle;
use crate::core::{entity::EntityBuilder, Entity};
use crate::{core::orchestrator, gameplay::Transform};

use std::{borrow::Cow, future::join};

use math::{normalize, Vector3, Vector4, Matrix4};
use utils::BoxedFuture;

pub trait MeshGenerator {
	fn vertices(&self) -> Cow<[(f32, f32, f32)]>;
	fn normals(&self) -> Cow<[(f32, f32, f32)]>;
	fn uvs(&self) -> Cow<[(f32, f32)]>;
	fn indices(&self) -> Cow<[u32]>;
	fn tangents(&self) -> Cow<[Vector3]>;
	fn bitangents(&self) -> Cow<[Vector3]>;
	fn colors(&self) -> Option<Cow<[Vector4]>> { None }
	fn meshlet_indices(&self) -> Option<Cow<[u8]>> { None }
}

pub enum MeshSource {
	Resource(&'static str),
	Generated(Box<dyn MeshGenerator>),
}

impl MeshSource {
	pub fn sphere(radius: f32) -> Self {
		MeshSource::Generated(Box::new(SphereMeshGenerator::new(radius)))
	}
}

pub struct SphereMeshGenerator {
	radius: f32,
	vertex_positions: Vec<(f32, f32, f32)>,
}

impl SphereMeshGenerator {
	pub fn new(radius: f32) -> Self {
		let segments = 32;
		let rings = 16;
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
			vertex_positions: vertices,
		}
	}
}

impl MeshGenerator for SphereMeshGenerator {
	fn vertices(&self) -> Cow<[(f32, f32, f32)]> {
		Cow::Borrowed(&self.vertex_positions)
	}

	fn indices(&self) -> Cow<[u32]> {
		let segments = 32;
		let rings = 16;
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
		let segments = 32;
		let rings = 16;
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
		let segments = 32;
		let rings = 16;
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
    	let segments = 32;
		let rings = 16;
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
    	None
	}

	fn uvs(&self) -> Cow<[(f32, f32)]> {
		let segments = 32;
		let rings = 16;
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
}

pub trait RenderEntity: Entity {
	fn get_transform(&self) -> Matrix4;
	fn get_mesh(&self) -> &MeshSource;
}

pub struct Mesh {
	source: MeshSource,
	transform: Transform,
}

impl Entity for Mesh {}

impl RenderEntity for Mesh {
	fn get_transform(&self) -> Matrix4 { self.transform.get_matrix() }
	fn get_mesh(&self) -> &MeshSource {
		&self.source
	}
}

impl Mesh {
	pub fn new(resource_id: &'static str, transform: Transform) -> EntityBuilder<'static, Self> {
		Self {
			source: MeshSource::Resource(resource_id),
			transform,
		}.into()
	}

	pub fn create(resource_id: &'static str, transform: Transform) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self {
			source: MeshSource::Resource(resource_id),
			transform,
		}).r#as(|h| h).r#as(|h| h as EntityHandle<dyn RenderEntity>)
	}

	pub fn new_generated(generator: Box<dyn MeshGenerator>, transform: Transform) -> EntityBuilder<'static, Self> {
		Self {
			source: MeshSource::Generated(generator),
			transform,
		}.into()
	}

	pub fn set_orientation(&mut self, orientation: Vector3) {
		self.transform.set_orientation(normalize(orientation));
	}

	pub fn transform(&self) -> &Transform {
		&self.transform
	}

	pub fn transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}
}
