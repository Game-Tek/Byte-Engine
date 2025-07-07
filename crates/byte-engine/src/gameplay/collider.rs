use math::Vector3;

use crate::{core::{entity::{get_entity_trait_for_type, EntityBuilder, EntityTrait}, Entity, EntityHandle}, gameplay::Positionable, physics::{body::{Body, BodyTypes}, collider::{Collider, CollisionShapes}, CollisionEvent}};

use crate::physics;

pub struct Sphere {
	radius: f32,
}

pub struct Cube {
	/// The half-size of the cube
	size: Vector3,
}

impl Sphere {
	pub fn new(radius: f32) -> Self {
		Self {
			radius,
		}
	}

	pub fn create(radius: f32) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self::new(radius)).r#as(|h| h).r#as(|h| h as EntityHandle<dyn Body>)
	}
}

impl Cube {
	pub fn new(size: Vector3) -> Self {
		Self {
			size,
		}
	}

	pub fn create(size: Vector3) -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self::new(size)).r#as(|h| h).r#as(|h| h as EntityHandle<dyn Body>)
	}
}

impl Entity for Sphere {}

impl Positionable for Sphere {
	fn get_position(&self) -> Vector3 {
		todo!()
	}

	fn set_position(&mut self, position: Vector3) {
		todo!()
	}
}

impl Collider for Sphere {
	fn shape(&self) -> CollisionShapes { CollisionShapes::Sphere { radius: self.radius } }
}

impl Body for Sphere {
	fn on_collision(&mut self) -> Option<&mut CollisionEvent> { None }
	fn get_velocity(&self) -> Vector3 { Vector3::new(0.0, 0.0, 0.0) }
	fn get_body_type(&self) -> BodyTypes { BodyTypes::Static }
	fn get_mass(&self) -> f32 {
		1f32
	}
}

impl Entity for Cube {
}

impl Positionable for Cube {
	fn get_position(&self) -> Vector3 {
		todo!()
	}

	fn set_position(&mut self, position: Vector3) {
		todo!()
	}
}

impl Collider for Cube {
	fn shape(&self) -> CollisionShapes { CollisionShapes::Cube { size: self.size } }
}

impl Body for Cube {
	fn on_collision(&mut self) -> Option<&mut CollisionEvent> { None }
	fn get_velocity(&self) -> Vector3 { Vector3::new(0.0, 0.0, 0.0) }
	fn get_body_type(&self) -> BodyTypes { BodyTypes::Static }
	fn get_mass(&self) -> f32 {
		1f32
	}
}
