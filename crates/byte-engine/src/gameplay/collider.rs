use crate::{core::{entity::{get_entity_trait_for_type, EntityBuilder, EntityTrait}, Entity, EntityHandle}, gameplay::Positionable, physics::{body::{Body, BodyTypes}, collider::{Collider, CollisionShapes}, CollisionEvent}};

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

	pub fn create(radius: f32) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self::new(radius)).r#as::<Self>().r#as::<dyn Body>()
	}
}

impl Cube {
	pub fn new(size: Vec3f) -> Self {
		Self {
			size,
		}
	}

	pub fn create(size: Vec3f) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self::new(size)).r#as::<Self>().r#as::<dyn Body>()
	}
}

impl Entity for Sphere {}

impl Positionable for Sphere {
	fn get_position(&self) -> crate::Vector3 {
		todo!()
	}

	fn set_position(&mut self, position: crate::Vector3) {
		todo!()
	}
}

impl Collider for Sphere {
	fn shape(&self) -> CollisionShapes { CollisionShapes::Sphere { radius: self.radius } }
}

impl Body for Sphere {
	fn on_collision(&mut self) -> Option<&mut CollisionEvent> { None }
	fn get_velocity(&self) -> maths_rs::Vec3f { maths_rs::Vec3f::new(0.0, 0.0, 0.0) }
	fn get_body_type(&self) -> BodyTypes { BodyTypes::Static }
}

impl Entity for Cube {
}

impl Positionable for Cube {
	fn get_position(&self) -> crate::Vector3 {
		todo!()
	}

	fn set_position(&mut self, position: crate::Vector3) {
		todo!()
	}
}

impl Collider for Cube {
	fn shape(&self) -> CollisionShapes { CollisionShapes::Cube { size: self.size } }
}

impl Body for Cube {
	fn on_collision(&mut self) -> Option<&mut CollisionEvent> { None }
	fn get_velocity(&self) -> maths_rs::Vec3f { maths_rs::Vec3f::new(0.0, 0.0, 0.0) }
	fn get_body_type(&self) -> BodyTypes { BodyTypes::Static }
}
