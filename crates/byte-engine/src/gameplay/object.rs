use std::future::join;

use maths_rs::mat::{MatScale, MatTranslate};
use utils::BoxedFuture;

use crate::{core::{entity::{get_entity_trait_for_type, EntityBuilder, EntityTrait}, event::Event, listener::{BasicListener, EntitySubscriber, Listener}, spawn, spawn_as_child, Entity, EntityHandle}, physics, Vector3};

#[cfg(not(feature = "headless"))]
use crate::rendering::mesh::{self};

use super::Transform;

pub struct Object {
	resource_id: &'static str,
	transform: Transform,
	velocity: Vector3,
	collision: Event<EntityHandle<dyn physics::PhysicsEntity>>,
	body_type: physics::BodyTypes,
}

impl Object {
	pub fn new<'a>(resource_id: &'static str, transform: Transform, body_type: physics::BodyTypes, velocity: Vector3) -> EntityBuilder<'a, Self> {
		EntityBuilder::new_from_closure_with_parent(move |parent| {
			Object {
				resource_id,
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

	fn call_listeners<'a>(&'a self, listener: &'a BasicListener, handle: EntityHandle<Self>,) -> BoxedFuture<'a, ()> where Self: Sized { Box::pin(async move {
		let same = listener.invoke_for(handle.clone(), self);
		let s: EntityHandle<dyn physics::PhysicsEntity> = handle.clone();
		let pe = listener.invoke_for(s, self);

		#[cfg(not(feature = "headless"))]
		{
			let s: EntityHandle<dyn mesh::RenderEntity> = handle.clone();
			let re = listener.invoke_for(s, self);
	
			join!(same, pe, re).await;
		}

		#[cfg(feature = "headless")]
		{
			join!(same, pe).await;
		}
	}) }
}

impl physics::PhysicsEntity for Object {
	fn on_collision(&mut self) -> &mut Event<EntityHandle<dyn physics::PhysicsEntity>> { &mut self.collision }
	fn get_position(&self) -> maths_rs::Vec3f { self.transform.get_position() }
	fn set_position(&mut self, position: maths_rs::Vec3f) { self.transform.set_position(position); }
	fn get_velocity(&self) -> maths_rs::Vec3f { self.velocity }
	fn get_body_type(&self) -> physics::BodyTypes { self.body_type }
}

#[cfg(not(feature = "headless"))]
impl mesh::RenderEntity for Object {
	fn get_transform(&self) -> maths_rs::Mat4f { (&self.transform).into() }
	fn get_resource_id(&self) -> &'static str { self.resource_id }
}