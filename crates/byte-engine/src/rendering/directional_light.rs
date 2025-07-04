use maths_rs::Vec3f;

use crate::{core::{Entity, EntityHandle}, inspector::Inspectable};

use super::cct;

#[derive(Debug, Clone, PartialEq)]
pub struct DirectionalLight {
	pub direction: Vec3f,
	pub color: Vec3f,
}

impl DirectionalLight {
	pub fn new(direction: Vec3f, cct: f32) -> Self {
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
