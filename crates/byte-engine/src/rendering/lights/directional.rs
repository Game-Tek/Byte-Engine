use math::Vector3;

use crate::{core::{Entity, EntityHandle}, inspector::Inspectable, rendering::lights::{Light, LightClasses}};

use super::super::cct;

/// The `DirectionalLight` struct represents a directional light source in a scene.
///
/// It is used to simulate light that comes from a distant source, such as the sun.
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

impl Light for DirectionalLight {
	fn class(&self) -> LightClasses {
		LightClasses::Directional
	}
}

impl Entity for DirectionalLight {
	fn builder(self) -> crate::core::entity::EntityBuilder<'static, Self> where Self: Sized {
    	crate::core::entity::EntityBuilder::new(self).r#as(|h| h).r#as(|h| h as EntityHandle<dyn Inspectable>).r#as(|h| h as EntityHandle<dyn Light>)
	}
}

impl Inspectable for DirectionalLight {
	fn as_string(&self) -> String {
		format!("{:?}", self)
	}
}
