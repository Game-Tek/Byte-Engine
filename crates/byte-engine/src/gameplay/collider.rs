use crate::core::{entity::{get_entity_trait_for_type, EntityBuilder, EntityTrait}, event::Event, listener::{BasicListener, Listener}, Entity, EntityHandle};
use std::future::join;

use maths_rs::Vec3f;

use crate::physics;

pub struct Sphere {
	radius: f32,
}

pub struct Cube {
	/// The half-size of the cube
	size: Vec3f,
}

impl Sphere {
	pub fn new(radius: f32) -> Self {
		Self {
			radius,
		}
	}
}

impl Cube {
	// pub fn new(size: Vec3f) -> Self {
	// 	Self {
	// 		size,
	// 	}
	// }

	pub fn new(size: Vec3f) -> EntityBuilder<'static, Self> {
		Self {
			size,
		}.into()
	}
}

impl Entity for Sphere {
	fn call_listeners<'a>(&'a self, listener: &'a BasicListener, handle: EntityHandle<Self>) -> () where Self: Sized {
		let se = listener.invoke_for(handle.clone(), self);
		let pe = listener.invoke_for(handle.clone() as EntityHandle<dyn physics::PhysicsEntity>, self);
	}
}

impl physics::PhysicsEntity for Sphere {
	fn on_collision(&mut self) -> Option<&mut Event<EntityHandle<dyn physics::PhysicsEntity>>> { None }
	fn get_position(&self) -> maths_rs::Vec3f { maths_rs::Vec3f::new(0.0, -0.5, 0.0) }
	fn set_position(&mut self, position: maths_rs::Vec3f) {}
	fn get_velocity(&self) -> maths_rs::Vec3f { maths_rs::Vec3f::new(0.0, 0.0, 0.0) }
	fn get_body_type(&self) -> physics::BodyTypes { physics::BodyTypes::Static }
	fn get_collision_shape(&self) -> physics::CollisionShapes { physics::CollisionShapes::Sphere { radius: self.radius } }
}

impl Entity for Cube {
	fn get_traits(&self) -> Vec<EntityTrait> { vec![unsafe { get_entity_trait_for_type::<dyn physics::PhysicsEntity>() }] }
	
	fn call_listeners<'a>(&'a self, listener: &'a BasicListener, handle: EntityHandle<Self>) -> () where Self: Sized {
		let se = listener.invoke_for(handle.clone(), self);
		let pe = listener.invoke_for(handle.clone() as EntityHandle<dyn physics::PhysicsEntity>, self);
	}
}

impl physics::PhysicsEntity for Cube {
	fn on_collision(&mut self) -> Option<&mut Event<EntityHandle<dyn physics::PhysicsEntity>>> { None }
	fn get_position(&self) -> maths_rs::Vec3f { maths_rs::Vec3f::new(0.0, -0.5, 0.0) }
	fn set_position(&mut self, position: maths_rs::Vec3f) {}
	fn get_velocity(&self) -> maths_rs::Vec3f { maths_rs::Vec3f::new(0.0, 0.0, 0.0) }
	fn get_body_type(&self) -> physics::BodyTypes { physics::BodyTypes::Static }
	fn get_collision_shape(&self) -> physics::CollisionShapes { physics::CollisionShapes::Cube { size: self.size } }
}