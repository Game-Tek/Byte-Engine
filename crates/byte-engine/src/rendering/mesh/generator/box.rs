use std::{borrow::Cow, hash::{Hash as _, Hasher}};

use math::{Vector3, Vector4};

use crate::rendering::{mesh::generator::MeshGenerator, renderable::mesh::MeshSource};

pub struct BoxMeshGenerator {
	size: Vector3,
}

impl BoxMeshGenerator {
	/// Create a new box mesh generator with a default size of 1x1x1.
	pub fn new() -> Self {
		Self {
			size: Vector3::new(1.0, 1.0, 1.0),
		}
	}

	pub fn from_size(size: Vector3) -> Self {
		Self {
			size,
		}
	}
}

impl MeshGenerator for BoxMeshGenerator {
	fn positions(&self) -> Cow<'_, [(f32, f32, f32)]> {
		let x = self.size.x;
		let y = self.size.y;
		let z = self.size.z;
		std::borrow::Cow::Owned(vec![
			(-x, -y, -z),
			(x, -y, -z),
			(x, y, -z),
			(-x, y, -z),
			(-x, -y, z),
			(x, -y, z),
			(x, y, z),
			(-x, y, z),
		])
	}

	fn normals(&self) -> Cow<'_, [(f32, f32, f32)]> {
		std::borrow::Cow::Owned(vec![
			(0.0, 0.0, -1.0),
			(0.0, 0.0, -1.0),
			(0.0, 0.0, -1.0),
			(0.0, 0.0, -1.0),
			(0.0, 0.0, 1.0),
			(0.0, 0.0, 1.0),
			(0.0, 0.0, 1.0),
			(0.0, 0.0, 1.0),
		])
	}

	fn tangents(&self) -> Cow<'_, [Vector3]> {
		std::borrow::Cow::Owned(vec![
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
		])
	}

	fn bitangents(&self) -> std::borrow::Cow<'_, [Vector3]> {
		std::borrow::Cow::Owned(vec![
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
		std::borrow::Cow::Borrowed(&[
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
		std::borrow::Cow::Borrowed(&[
			0, 2, 1,
			0, 3, 2,
			4, 5, 6,
			4, 6, 7,
			0, 1, 5,
			0, 5, 4,
			1, 2, 6,
			1, 6, 5,
			2, 3, 7,
			2, 7, 6,
			3, 0, 4,
			3, 4, 7,
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

impl Into<Box<dyn MeshGenerator>> for BoxMeshGenerator {
	fn into(self) -> Box<dyn MeshGenerator> {
		Box::new(self)
	}
}

impl Into<MeshSource> for BoxMeshGenerator {
	fn into(self) -> MeshSource {
		Into::<Box<dyn MeshGenerator>>::into(self).into()
	}
}
