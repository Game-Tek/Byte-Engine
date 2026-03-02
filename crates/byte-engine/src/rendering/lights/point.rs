use math::Vector3;

use crate::{
	core::{Entity, EntityHandle},
	inspector::Inspectable,
	rendering::lights::{Light, LightClasses},
};

use super::super::cct;

/// The `PointLight` struct represents a point light source in a scene.
///
/// It is used to simulate light that comes from a single point, such as a light bulb.
#[derive(Debug, Clone, Copy)]
pub struct PointLight {
	pub position: Vector3,
	pub color: Vector3,
}

impl PointLight {
	pub fn new(position: Vector3, cct: f32) -> Self {
		Self {
			position,
			color: cct::rgb_from_temperature(cct),
		}
	}
}

impl Light for PointLight {
	fn class(&self) -> LightClasses {
		LightClasses::Point
	}
}

impl Inspectable for PointLight {
	fn as_string(&self) -> String {
		format!("{:?}", self)
	}
}
