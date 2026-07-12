use std::{future::join, sync::Arc};

#[cfg(feature = "headed")]
use math::Matrix4;
use math::Vector3;
use utils::BoxedFuture;

use super::transform::Transform;
#[cfg(feature = "headed")]
use crate::rendering::{
	mesh::{self},
	renderable::mesh::MeshSource,
};
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

/// An object represents a physical entity in the game world.
/// It has physics and is rendered as a mesh.
#[derive(Clone)]
pub struct Object {
	source: MeshSource,
	transform: Transform,
	velocity: Vector3,
	body_type: BodyTypes,
	collider: Shapes,
	friction: f32,
}

impl Object {
	pub fn new<'a>(resource_id: &'static str, transform: Transform, body_type: BodyTypes, velocity: Vector3) -> Self {
		Object {
			source: MeshSource::Resource(resource_id),
			transform,
			velocity,
			body_type,
			collider: Shapes::Sphere { radius: 1.0 },
			friction: 0.5,
		}
	}

	pub fn sphere(radius: f32) -> Self {
		Object {
			source: MeshSource::Generated(Arc::new(SphereMeshGenerator::from_radius(radius))),
			transform: Transform::default(),
			velocity: Vector3::default(),
			body_type: BodyTypes::Dynamic,
			collider: Shapes::Sphere { radius },
			friction: 0.5,
		}
	}

	pub fn r#box(size: Vector3) -> Self {
		Object {
			source: MeshSource::Generated(Arc::new(mesh::generator::BoxMeshGenerator::from_size(size))),
			transform: Transform::default(),
			velocity: Vector3::default(),
			body_type: BodyTypes::Dynamic,
			collider: Shapes::Cube { size },
			friction: 0.5,
		}
	}

	pub fn from_mesh_source(mesh_source: MeshSource) -> Self {
		Object {
			source: mesh_source,
			transform: Transform::default(),
			velocity: Vector3::default(),
			body_type: BodyTypes::Dynamic,
			collider: Shapes::Sphere { radius: 1.0 },
			friction: 0.5,
		}
	}

	pub fn new_generated(mesh: Arc<dyn MeshGenerator>) -> Self {
		Object {
			source: MeshSource::Generated(mesh),
			transform: Transform::default(),
			velocity: Vector3::default(),
			body_type: BodyTypes::Dynamic,
			collider: Shapes::Sphere { radius: 1.0 },
			friction: 0.5,
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
		self.collider.clone()
	}

	fn friction(&self) -> f32 {
		self.friction
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

#[cfg(test)]
mod tests {
	use math::Vector3;

	use super::Object;
	use crate::{
		physics::{
			body::{Body, BodyTypes},
			collider::{Collider, Shapes},
		},
		rendering::{
			mesh::generator::MeshGenerator,
			renderable::mesh::{MeshSource, RenderableMesh},
		},
		space::{Positionable, Scalable, Transformable},
	};

	#[test]
	fn sphere_constructor_keeps_render_and_collision_radius_in_sync() {
		let object = Object::sphere(2.5);
		assert!(matches!(object.shape(), Shapes::Sphere { radius } if radius == 2.5));
		assert_eq!(object.body_type(), BodyTypes::Dynamic);
		assert_eq!(object.velocity(), Vector3::new(0.0, 0.0, 0.0));
		assert_eq!(object.friction(), 0.5);
		match object.get_mesh() {
			MeshSource::Generated(generator) => {
				let positions = generator.positions();
				assert!(positions.iter().any(|&(_, y, _)| (y - 2.5).abs() < 1e-5));
			}
			MeshSource::Resource(_) => {
				panic!("Expected generated sphere geometry. The most likely cause is a mismatched object constructor.")
			}
		}
	}

	#[test]
	fn box_constructor_keeps_render_and_collision_extents_in_sync() {
		let size = Vector3::new(1.0, 2.0, 3.0);
		let object = Object::r#box(size);
		assert!(matches!(object.shape(), Shapes::Cube { size: collider_size } if collider_size == size));
		match object.get_mesh() {
			MeshSource::Generated(generator) => assert!(generator
				.positions()
				.iter()
				.all(|&(x, y, z)| x.abs() == size.x && y.abs() == size.y && z.abs() == size.z)),
			MeshSource::Resource(_) => {
				panic!("Expected generated box geometry. The most likely cause is a mismatched object constructor.")
			}
		}
	}

	#[test]
	fn object_physics_and_transform_mutators_are_observable_through_traits() {
		let mut object = Object::new(
			"mesh.resource",
			crate::gameplay::Transform::default(),
			BodyTypes::Static,
			Vector3::new(1.0, 2.0, 3.0),
		);
		assert!(matches!(object.get_mesh(), MeshSource::Resource("mesh.resource")));
		assert_eq!(object.body_type(), BodyTypes::Static);
		assert_eq!(object.velocity(), Vector3::new(1.0, 2.0, 3.0));

		*object.body_type_mut() = BodyTypes::Kinematic;
		object.set_velocity(Vector3::new(4.0, 5.0, 6.0));
		object.set_position(Vector3::new(7.0, 8.0, 9.0));
		object.set_scale(Vector3::new(2.0, 3.0, 4.0));
		assert_eq!(object.body_type(), BodyTypes::Kinematic);
		assert_eq!(object.velocity(), Vector3::new(4.0, 5.0, 6.0));
		assert_eq!(object.position(), Vector3::new(7.0, 8.0, 9.0));
		assert_eq!(object.transform().scale(), Vector3::new(2.0, 3.0, 4.0));
	}

	#[test]
	fn cloned_objects_have_independent_transform_and_velocity_state() {
		let original = Object::sphere(1.0);
		let mut clone = original.clone();
		clone.set_position(Vector3::new(1.0, 2.0, 3.0));
		clone.set_velocity(Vector3::new(4.0, 5.0, 6.0));

		assert_eq!(original.position(), Vector3::new(0.0, 0.0, 0.0));
		assert_eq!(original.velocity(), Vector3::new(0.0, 0.0, 0.0));
		assert_eq!(clone.position(), Vector3::new(1.0, 2.0, 3.0));
		assert_eq!(clone.velocity(), Vector3::new(4.0, 5.0, 6.0));
	}
}
