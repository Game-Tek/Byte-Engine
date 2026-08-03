use std::sync::Arc;

use math::Vector;

use super::transform::Transform;
#[cfg(feature = "headed")]
use crate::rendering::{mesh, renderable::mesh::MeshSource};
use crate::{
	physics::{
		body::{Body, BodyTypes},
		collider::{Collider, Shapes},
		LocalSpace,
	},
	rendering::{
		mesh::generator::{MeshGenerator, SphereMeshGenerator},
		RenderableMesh,
	},
	space::Transformable,
};

/// The `Object` struct combines a renderable mesh with a physical body for the default game world.
#[derive(Clone)]
pub struct Object {
	source: MeshSource,
	transform: Transform,
	velocity: Vector,
	body_type: BodyTypes,
	collider: Shapes,
	friction: f32,
}

impl Object {
	/// Creates a resource-backed object with a spherical collider.
	pub fn new(resource_id: &'static str, transform: Transform, body_type: BodyTypes, velocity: Vector) -> Self {
		Self {
			source: MeshSource::Resource(resource_id),
			transform,
			velocity,
			body_type,
			collider: Shapes::Sphere { radius: 1.0 },
			friction: 0.5,
		}
	}

	/// Creates a generated sphere whose renderer and collider share `radius`.
	pub fn sphere(radius: f32) -> Self {
		Self {
			source: MeshSource::Generated(Arc::new(SphereMeshGenerator::from_radius(radius))),
			transform: Transform::default(),
			velocity: Vector::zero(),
			body_type: BodyTypes::Dynamic,
			collider: Shapes::Sphere { radius },
			friction: 0.5,
		}
	}

	/// Creates a generated box from local half-extents.
	pub fn r#box(size: Vector<LocalSpace>) -> Self {
		Self {
			source: MeshSource::Generated(Arc::new(mesh::generator::BoxMeshGenerator::from_size(size.into_maths()))),
			transform: Transform::default(),
			velocity: Vector::zero(),
			body_type: BodyTypes::Dynamic,
			collider: Shapes::Cube { size },
			friction: 0.5,
		}
	}

	/// Creates a dynamic object from an existing mesh source.
	pub fn from_mesh_source(mesh_source: MeshSource) -> Self {
		Self::new_generated_or_resource(mesh_source)
	}

	fn new_generated_or_resource(source: MeshSource) -> Self {
		Self {
			source,
			transform: Transform::default(),
			velocity: Vector::zero(),
			body_type: BodyTypes::Dynamic,
			collider: Shapes::Sphere { radius: 1.0 },
			friction: 0.5,
		}
	}

	/// Creates a dynamic object from a generated mesh.
	pub fn new_generated(mesh: Arc<dyn MeshGenerator>) -> Self {
		Self::new_generated_or_resource(MeshSource::Generated(mesh))
	}

	/// Returns mutable access to the gameplay transform.
	pub fn get_transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}

	/// Returns mutable access to the body type.
	pub fn body_type_mut(&mut self) -> &mut BodyTypes {
		&mut self.body_type
	}

	/// Replaces the world-space linear velocity.
	pub fn set_velocity(&mut self, velocity: Vector) {
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
		self.collider.clone()
	}
	fn friction(&self) -> f32 {
		self.friction
	}
}

impl Body for Object {
	fn velocity(&self) -> Vector {
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
