use maths_rs::mat::{MatScale, MatTranslate};

use crate::{core::{entity::{get_entity_trait_for_type, EntityBuilder, EntityTrait}, event::Event, listener::{BasicListener, EntitySubscriber, Listener}, spawn, spawn_as_child, Entity, EntityHandle}, physics, rendering::mesh, Vector3};

pub struct Object {
	position: Vector3,
	velocity: Vector3,
	collision: Event<EntityHandle<dyn physics::PhysicsEntity>>,
}

impl Object {
	pub fn new<'a>(position: Vector3, velocity: Vector3) -> EntityBuilder<'a, Self> {
		let transform = maths_rs::Mat4f::from_translation(position) * maths_rs::Mat4f::from_scale(Vector3::new(0.05, 0.05, 0.05));

		EntityBuilder::new_from_closure_with_parent(move |parent| {
			Object {
				position,
				velocity,
				collision: Default::default(),
			}
		})
	}
}

impl Entity for Object {
	fn get_traits(&self) -> Vec<EntityTrait> { vec![unsafe { get_entity_trait_for_type::<dyn physics::PhysicsEntity>() }] }

	fn call_listeners(&self, listener: &BasicListener, handle: EntityHandle<Self>,) where Self: Sized {
		listener.invoke_for(handle.clone(), self);
		let s: EntityHandle<dyn physics::PhysicsEntity> = handle.clone();
		listener.invoke_for(s, self);
		let s: EntityHandle<dyn mesh::RenderEntity> = handle.clone();
		listener.invoke_for(s, self);
	}
}

impl physics::PhysicsEntity for Object {
	fn on_collision(&mut self) -> &mut Event<EntityHandle<dyn physics::PhysicsEntity>> { &mut self.collision }
	fn get_position(&self) -> maths_rs::Vec3f { self.position }
	fn set_position(&mut self, position: maths_rs::Vec3f) { self.position = position; }
	fn get_velocity(&self) -> maths_rs::Vec3f { self.velocity }
}

impl mesh::RenderEntity for Object {
	fn get_transform(&self) -> maths_rs::Mat4f { maths_rs::Mat4f::from_translation(self.position) * maths_rs::Mat4f::from_scale(Vector3::new(0.05, 0.05, 0.05)) }
	fn get_resource_id(&self) -> &'static str { "Box.glb" }
}