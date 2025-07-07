use std::borrow::Cow;

use math::{Matrix4, Vector3, Vector4};

use crate::core::{entity::EntityBuilder, Entity, EntityHandle};

use super::mesh::{MeshGenerator, MeshSource, RenderEntity};

pub struct Cube {
	generator: MeshSource,
}

impl Cube {
	pub fn new() -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self {
			generator: MeshSource::Generated(Box::new(CubeMeshGenerator {})),
		}).r#as(|h| h).r#as(|h| h as EntityHandle<dyn RenderEntity>)
	}
}

impl Entity for Cube {
}

impl RenderEntity for Cube {
	fn get_transform(&self) -> Matrix4 {
		Matrix4::identity()
	}

	fn get_mesh(&self) -> &MeshSource {
		&self.generator
	}
}

struct CubeMeshGenerator {}

impl MeshGenerator for CubeMeshGenerator {
	fn vertices(&self) -> Cow<'_, [(f32, f32, f32)]> {
		std::borrow::Cow::Owned(vec![
			(-1.0, -1.0, -1.0),
			(1.0, -1.0, -1.0),
			(1.0, 1.0, -1.0),
			(-1.0, 1.0, -1.0),
			(-1.0, -1.0, 1.0),
			(1.0, -1.0, 1.0),
			(1.0, 1.0, 1.0),
			(-1.0, 1.0, 1.0),
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

	fn tangents(&self) -> Cow<[Vector3]> {
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

	fn bitangents(&self) -> std::borrow::Cow<[Vector3]> {
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
		std::borrow::Cow::Owned(vec![
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

	fn colors(&self) -> Option<std::borrow::Cow<[Vector4]>> {
		Some(std::borrow::Cow::Owned(vec![
			Vector4::new(1.0, 0.0, 0.0, 1.0),
			Vector4::new(0.0, 1.0, 0.0, 1.0),
			Vector4::new(0.0, 0.0, 1.0, 1.0),
			Vector4::new(1.0, 1.0, 1.0, 1.0),
			Vector4::new(1.0, 1.0, 1.0, 1.0),
			Vector4::new(1.0, 1.0, 1.0, 1.0),
			Vector4::new(1.0, 1.0, 1.0, 1.0),
			Vector4::new(1.0, 1.0, 1.0, 1.0),
		]))
	}

	fn indices(&self) -> std::borrow::Cow<[u32]> {
		std::borrow::Cow::Owned(vec![
			0, 1, 2,
			0, 2, 3,
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

	fn meshlet_indices(&self) -> Option<std::borrow::Cow<[u8]>> {
		Some(std::borrow::Cow::Owned(vec![
			0, 1, 2, 3,
			4, 5, 6, 7,
			8, 9, 10, 11,
			12, 13, 14, 15,
			16, 17, 18, 19,
			20, 21, 22, 23,
		]))
	}
}
