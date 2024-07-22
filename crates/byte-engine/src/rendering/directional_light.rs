use maths_rs::Vec3f;

use crate::core::{Entity};

use super::cct;

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

impl Entity for DirectionalLight {}