use math::Vector3;

use super::{LightColor, PhotometricError, PhotometricIntensity};
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
	pub shadow_near: f32,
	pub shadow_far: f32,
}

impl ConeLight {
	/// Creates a cone light whose intensity fades smoothly between the inner and outer half angles.
	///
	/// `shadow_near` and `shadow_far` bound the light's perspective shadow view. Submit the light
	/// through [`crate::gameplay::world::DefaultWorld::light_factory_mut`] to make it available to
	/// the active rendering pipeline.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color or intensity contains an invalid physical value.
	/// Invalid directions, angles, or shadow ranges panic because they cannot form a valid cone view.
	pub fn new(
		position: Vector3,
		direction: Vector3,
		color: LightColor,
		intensity: PhotometricIntensity,
		inner_angle: f32,
		outer_angle: f32,
		shadow_near: f32,
		shadow_far: f32,
	) -> Result<Self, PhotometricError> {
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
		assert!(
			shadow_near.is_finite() && shadow_far.is_finite() && shadow_near > 0.0 && shadow_far > shadow_near,
			"Invalid cone light shadow range. The most likely cause is that the clipping distances are non-finite, nonpositive, or not ordered near before far."
		);

		let chromaticity = color.resolve()?;
		let candela = intensity.cone_candela(inner_angle, outer_angle)?;
		Ok(Self {
			position,
			direction,
			color: Vector3::new(chromaticity.x * candela, chromaticity.y * candela, chromaticity.z * candela),
			inner_angle,
			outer_angle,
			shadow_near,
			shadow_far,
		})
	}

	/// Returns whether this light's cone fits in one perspective shadow view.
	pub fn supports_shadow_mapping(&self) -> bool {
		self.outer_angle < std::f32::consts::FRAC_PI_2
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

#[cfg(test)]
mod tests {
	use math::{Base as _, VecN as _, Vector3};

	use super::ConeLight;
	use crate::rendering::lights::{LightColor, PhotometricIntensity};

	fn intensity() -> PhotometricIntensity {
		PhotometricIntensity::LuminousIntensity {
			candela: 100.0,
			reference_distance_m: 1.0,
		}
	}

	#[test]
	fn cone_light_preserves_explicit_shadow_range() {
		let light = ConeLight::new(
			Vector3::new(1.0, 2.0, 3.0),
			Vector3::new(0.0, -1.0, 0.0),
			LightColor::TemperatureKelvin(4_500.0),
			intensity(),
			0.25,
			0.5,
			0.2,
			75.0,
		)
		.expect("physical cone light");

		assert_eq!(light.shadow_near, 0.2);
		assert_eq!(light.shadow_far, 75.0);
	}

	#[test]
	#[should_panic(expected = "Invalid cone light shadow range")]
	fn cone_light_rejects_inverted_shadow_range() {
		ConeLight::new(
			Vector3::zero(),
			Vector3::unit_z(),
			LightColor::TemperatureKelvin(4_500.0),
			intensity(),
			0.25,
			0.5,
			10.0,
			1.0,
		)
		.expect("the shadow-range assertion should run before photometric resolution");
	}

	#[test]
	fn cone_light_wider_than_a_perspective_view_remains_valid_but_unshadowed() {
		let light = ConeLight::new(
			Vector3::zero(),
			Vector3::unit_z(),
			LightColor::TemperatureKelvin(4_500.0),
			intensity(),
			0.25,
			std::f32::consts::PI,
			0.1,
			100.0,
		)
		.expect("physical cone light");

		assert!(!light.supports_shadow_mapping());
	}
}
