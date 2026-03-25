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
	/// Create a new box mesh generator with a default size of 1x1x1.
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
			0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11, 12, 13, 14, 12, 14, 15, 16, 17, 18, 16, 18, 19, 20, 21,
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

impl Into<Arc<dyn MeshGenerator>> for BoxMeshGenerator {
	fn into(self) -> Arc<dyn MeshGenerator> {
		Arc::new(self)
	}
}

impl Into<MeshSource> for BoxMeshGenerator {
	fn into(self) -> MeshSource {
		Into::<Arc<dyn MeshGenerator>>::into(self).into()
	}
}
