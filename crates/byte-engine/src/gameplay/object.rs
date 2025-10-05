use std::future::join;

#[cfg(feature = "headed")]
use math::Matrix4;
use math::Vector3;
use utils::BoxedFuture;

use crate::{core::{entity::{get_entity_trait_for_type, EntityBuilder, EntityTrait}, Entity, EntityHandle}, physics::{self, body::{Body, BodyTypes}, collider::{Collider, CollisionShapes}, CollisionEvent}, rendering::{mesh::generator::{MeshGenerator, SphereMeshGenerator}, RenderableMesh}};

#[cfg(feature = "headed")]
use crate::rendering::{mesh::{self}, renderable::mesh::MeshSource};

use super::{Positionable, Transform, Transformable};

/// An object represents a physical entity in the game world.
/// It has physics and is rendered as a mesh.
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
		}).r#as(|h| h).r#as(|h| h as EntityHandle<dyn Body>).r#as(|h| h as EntityHandle<dyn RenderableMesh>)
	}

	pub fn from_mesh_source(mesh_source: MeshSource) -> Self {
		Object {
			source: mesh_source,
			transform: Transform::default(),
			velocity: Vector3::default(),
			collision: CollisionEvent{},
			body_type: BodyTypes::Dynamic,
		}
	}

	pub fn new_generated(mesh: Box<dyn MeshGenerator>) -> Self {
		Object {
			source: MeshSource::Generated(mesh),
			transform: Transform::default(),
			velocity: Vector3::default(),
			collision: CollisionEvent{},
			body_type: BodyTypes::Dynamic,
		}
	}

	pub fn get_transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}
}

impl Entity for Object {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self).r#as(|h| h).r#as(|h| h as EntityHandle<dyn Body>).r#as(|h| h as EntityHandle<dyn RenderableMesh>)
	}
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
	fn get_velocity(&self) -> Vector3 { self.velocity }
	fn get_body_type(&self) -> BodyTypes { self.body_type }
	fn get_mass(&self) -> f32 {
    	1f32
	}
}

#[cfg(feature = "headed")]
impl RenderableMesh for Object {
	fn get_mesh(&self) -> &MeshSource {
		&self.source
	}
}
