use std::{future::join, sync::Arc};

#[cfg(feature = "headed")]
use math::Matrix4;
use math::Vector3;
use utils::BoxedFuture;

use crate::{
	core::{Entity, EntityHandle},
	physics::{
		self,
		body::{Body, BodyTypes},
		collider::{Collider, Shapes},
	},
	rendering::{
		mesh::generator::{MeshGenerator, SphereMeshGenerator},
		RenderableMesh,
	},
	space::Transformable,
};

#[cfg(feature = "headed")]
use crate::rendering::{
	mesh::{self},
	renderable::mesh::MeshSource,
};

use super::transform::Transform;

/// An object represents a physical entity in the game world.
/// It has physics and is rendered as a mesh.
#[derive(Clone)]
pub struct Object {
	source: MeshSource,
	transform: Transform,
	velocity: Vector3,
	body_type: BodyTypes,
	collider: Shapes,
}

impl Object {
	pub fn new<'a>(resource_id: &'static str, transform: Transform, body_type: BodyTypes, velocity: Vector3) -> Self {
		Object {
			source: MeshSource::Resource(resource_id),
			transform,
			velocity,
			body_type,
			collider: Shapes::Sphere { radius: 1.0 },
		}
	}

	pub fn sphere(radius: f32) -> Self {
		Object {
			source: MeshSource::Generated(Arc::new(SphereMeshGenerator::from_radius(radius))),
			transform: Transform::default(),
			velocity: Vector3::default(),
			body_type: BodyTypes::Dynamic,
			collider: Shapes::Sphere { radius },
		}
	}

	pub fn r#box(size: Vector3) -> Self {
		Object {
			source: MeshSource::Generated(Arc::new(mesh::generator::BoxMeshGenerator::from_size(size))),
			transform: Transform::default(),
			velocity: Vector3::default(),
			body_type: BodyTypes::Dynamic,
			collider: Shapes::Cube { size },
		}
	}

	pub fn from_mesh_source(mesh_source: MeshSource) -> Self {
		Object {
			source: mesh_source,
			transform: Transform::default(),
			velocity: Vector3::default(),
			body_type: BodyTypes::Dynamic,
			collider: Shapes::Sphere { radius: 1.0 },
		}
	}

	pub fn new_generated(mesh: Arc<dyn MeshGenerator>) -> Self {
		Object {
			source: MeshSource::Generated(mesh),
			transform: Transform::default(),
			velocity: Vector3::default(),
			body_type: BodyTypes::Dynamic,
			collider: Shapes::Sphere { radius: 1.0 },
		}
	}

	pub fn get_transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}

	pub fn body_type_mut(&mut self) -> &mut BodyTypes {
		&mut self.body_type
	}

	pub fn set_velocity(&mut self, velocity: Vector3) {
		self.velocity = velocity;
	}
}

impl Transformable for Object {
	fn transform(&self) -> &Transform {
		&self.transform
	}
	fn transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}
}

impl Collider for Object {
	fn shape(&self) -> Shapes {
		self.collider
	}
}

impl Body for Object {
	fn velocity(&self) -> Vector3 {
		self.velocity
	}
	fn body_type(&self) -> BodyTypes {
		self.body_type
	}
}

#[cfg(feature = "headed")]
impl RenderableMesh for Object {
	fn get_mesh(&self) -> &MeshSource {
		&self.source
	}
}
