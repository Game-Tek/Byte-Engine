use math::Vector3;

use crate::{core::{Entity, EntityHandle}, inspector::Inspectable};

use super::cct;

#[derive(Debug, Clone, PartialEq)]
pub struct DirectionalLight {
	pub direction: Vector3,
	pub color: Vector3,
}

impl DirectionalLight {
	pub fn new(direction: Vector3, cct: f32) -> Self {
		Self {
			direction,
			color: cct::rgb_from_temperature(cct),
		}
	}
}

impl Entity for DirectionalLight {
	fn builder(self) -> crate::core::entity::EntityBuilder<'static, Self> where Self: Sized {
    	crate::core::entity::EntityBuilder::new(self).r#as(|h| h).r#as(|h| h as EntityHandle<dyn Inspectable>)
	}
}

impl Inspectable for DirectionalLight {
	fn as_string(&self) -> String {
		format!("{:?}", self)
	}
}
