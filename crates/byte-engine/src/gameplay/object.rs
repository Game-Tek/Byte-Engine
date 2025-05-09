use std::future::join;

use maths_rs::mat::{MatScale, MatTranslate};
use utils::BoxedFuture;

use crate::{core::{entity::{get_entity_trait_for_type, Caller, EntityBuilder, EntityTrait}, event::Event, listener::{BasicListener, EntitySubscriber, Listener}, spawn, spawn_as_child, Entity, EntityHandle}, physics, rendering::mesh::MeshSource, Vector3};

#[cfg(not(feature = "headless"))]
use crate::rendering::mesh::{self};

use super::{Positionable, Transform, Transformable};

pub struct Object {
	source: MeshSource,
	transform: Transform,
	velocity: Vector3,
	collision: Event<EntityHandle<dyn physics::PhysicsEntity>>,
	body_type: physics::BodyTypes,
}

impl Object {
	pub fn new<'a>(resource_id: &'static str, transform: Transform, body_type: physics::BodyTypes, velocity: Vector3) -> EntityBuilder<'a, Self> {
		EntityBuilder::new_from_closure_with_parent(move |parent| {
			Object {
				source: MeshSource::Resource(resource_id),
				transform,
				velocity,
				collision: Default::default(),
				body_type,
			}
		})
	}

	pub fn get_transform_mut(&mut self) -> &mut Transform {
		&mut self.transform
	}
}

impl Entity for Object {
	fn get_traits(&self) -> Vec<EntityTrait> { vec![unsafe { get_entity_trait_for_type::<dyn physics::PhysicsEntity>() }] }

	fn call_listeners<'a>(&'a self, caller: Caller, handle: EntityHandle<Self>,) -> () where Self: Sized {
		caller.call(handle.clone(), self);
		caller.call(handle.clone() as EntityHandle<dyn physics::PhysicsEntity>, self);

		#[cfg(not(feature = "headless"))]
		{
			let re = caller.call(handle.clone() as EntityHandle<dyn mesh::RenderEntity>, self);
		}
	}
}

impl Positionable for Object {
	fn get_position(&self) -> Vector3 { self.transform.position }
	fn set_position(&mut self, position: Vector3) { self.transform.position = position; }
}

impl Transformable for Object {
	fn get_transform(&self) -> &Transform { &self.transform }
	fn get_transform_mut(&mut self) -> &mut Transform { &mut self.transform }
}

impl physics::PhysicsEntity for Object {
	fn on_collision(&mut self) -> Option<&mut Event<EntityHandle<dyn physics::PhysicsEntity>>> { Some(&mut self.collision) }
	fn get_position(&self) -> maths_rs::Vec3f { self.transform.get_position() }
	fn set_position(&mut self, position: maths_rs::Vec3f) { self.transform.set_position(position); }
	fn get_velocity(&self) -> maths_rs::Vec3f { self.velocity }
	fn get_body_type(&self) -> physics::BodyTypes { self.body_type }
	fn get_collision_shape(&self) -> physics::CollisionShapes {
		physics::CollisionShapes::Sphere {
			radius: 0.1,
		}
	}
}

#[cfg(not(feature = "headless"))]
impl mesh::RenderEntity for Object {
	fn get_transform(&self) -> maths_rs::Mat4f { (&self.transform).into() }
	fn get_mesh(&self) -> &mesh::MeshSource {
		&self.source
	}
}