use std::future::join;

use maths_rs::mat::{MatScale, MatTranslate};
use utils::BoxedFuture;

use crate::{core::{entity::{get_entity_trait_for_type, EntityBuilder, EntityTrait}, Entity, EntityHandle}, physics::{self, body::{Body, BodyTypes}, collider::{Collider, CollisionShapes}, CollisionEvent}, rendering::mesh::MeshSource, Vector3};

#[cfg(not(feature = "headless"))]
use crate::rendering::mesh::{self};

use super::{Positionable, Transform, Transformable};

pub struct Object {
	source: MeshSource,
	transform: Transform,
	velocity: Vector3,
	collision: CollisionEvent,
	body_type: BodyTypes,
}

impl Object {
	pub fn new<'a>(resource_id: &'static str, transform: Transform, body_type: BodyTypes, velocity: Vector3) -> EntityBuilder<'a, Self> {
		EntityBuilder::new_from_closure_with_parent(move |parent| {
			Object {
				source: MeshSource::Resource(resource_id),
				transform,
				velocity,
				collision: CollisionEvent{},
				body_type,
			}
		}).r#as::<Self>().r#as::<dyn Body>().r#as::<dyn mesh::RenderEntity>()
	}

	pub fn get_transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}
}

impl Entity for Object {}

impl Positionable for Object {
	fn get_position(&self) -> Vector3 { self.transform.position }
	fn set_position(&mut self, position: Vector3) { self.transform.position = position; }
}

impl Transformable for Object {
	fn get_transform(&self) -> &Transform { &self.transform }
	fn get_transform_mut(&mut self) -> &mut Transform { &mut self.transform }
}

impl Collider for Object {
	fn shape(&self) -> CollisionShapes {
		CollisionShapes::Sphere {
			radius: 0.1,
		}
	}
}

impl Body for Object {
	fn on_collision(&mut self) -> Option<&mut CollisionEvent> { Some(&mut self.collision) }
	fn get_velocity(&self) -> maths_rs::Vec3f { self.velocity }
	fn get_body_type(&self) -> BodyTypes { self.body_type }
}

#[cfg(not(feature = "headless"))]
impl mesh::RenderEntity for Object {
	fn get_transform(&self) -> maths_rs::Mat4f { (&self.transform).into() }
	fn get_mesh(&self) -> &mesh::MeshSource {
		&self.source
	}
}
