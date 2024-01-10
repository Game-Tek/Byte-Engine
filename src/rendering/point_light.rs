use maths_rs::Vec3f;

use crate::core::{orchestrator::{Component,}, Entity};

use super::cct;

pub struct PointLight {
	pub position: Vec3f,
	pub color: Vec3f,
}

impl PointLight {
	pub fn new(position: Vec3f, cct: f32) -> Self {
		Self {
			position,
			color: cct::rgb_from_temperature(cct),
		}
	}
}

impl Entity for PointLight {}
impl Component for PointLight {}