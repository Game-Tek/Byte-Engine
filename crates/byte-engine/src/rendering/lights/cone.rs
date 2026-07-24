use math::Vector3;

use super::super::cct;
use crate::{
	core::{Entity, EntityHandle},
	inspector::Inspectable,
	rendering::lights::{Light, LightClasses},
};

/// The `ConeLight` struct provides local lighting constrained to a directed cone.
///
/// Use it for spotlights, flashlights, and other emitters that need a soft transition
/// between a fully lit inner cone and an unlit outer cone. Cone angles are half angles
/// measured in radians from `direction`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConeLight {
	pub position: Vector3,
	pub direction: Vector3,
	pub color: Vector3,
	pub inner_angle: f32,
	pub outer_angle: f32,
}

impl ConeLight {
	/// Creates a cone light whose intensity fades smoothly between the inner and outer half angles.
	pub fn new(position: Vector3, direction: Vector3, cct: f32, inner_angle: f32, outer_angle: f32) -> Self {
		// Reject directions that would become undefined when normalized during material evaluation.
		let direction_length_squared = direction.x * direction.x + direction.y * direction.y + direction.z * direction.z;
		assert!(
			direction_length_squared.is_finite() && direction_length_squared > f32::EPSILON,
			"Invalid cone light direction. The most likely cause is that the direction is zero or contains a non-finite component."
		);
		assert!(
			inner_angle.is_finite() && outer_angle.is_finite() && inner_angle >= 0.0 && inner_angle < outer_angle,
			"Invalid cone light angles. The most likely cause is that the angles are not finite or the inner angle is not smaller than the outer angle."
		);
		assert!(
			outer_angle <= std::f32::consts::PI,
			"Invalid cone light outer angle. The most likely cause is that the supplied half angle exceeds pi radians."
		);

		Self {
			position,
			direction,
			color: cct::rgb_from_temperature(cct),
			inner_angle,
			outer_angle,
		}
	}
}

impl Light for ConeLight {
	fn class(&self) -> LightClasses {
		LightClasses::Cone
	}
}

impl Inspectable for ConeLight {
	fn as_string(&self) -> String {
		format!("{:?}", self)
	}
}
