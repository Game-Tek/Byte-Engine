use math::Vector3;

use crate::core::{Entity};

use super::cct;

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

impl Entity for PointLight {}
