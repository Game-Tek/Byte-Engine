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
	shadow_near_override: Option<f32>,
	shadow_far_override: Option<f32>,
}

impl ConeLight {
	/// Creates a cone light whose intensity fades smoothly between the inner and outer half angles.
	///
	/// The renderer derives the shadow range from the resolved luminous intensity. Use
	/// [`Self::with_shadow_near`], [`Self::with_shadow_far`], or [`Self::with_shadow_range`] to
	/// override that range. Submit the light through
	/// [`crate::gameplay::world::DefaultWorld::light_factory_mut`] to make it available to the
	/// active rendering pipeline.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color or intensity contains an invalid physical value.
	/// Invalid directions or angles panic because they cannot form a valid cone view.
	pub fn new(
		position: Vector3,
		direction: Vector3,
		color: LightColor,
		intensity: PhotometricIntensity,
		inner_angle: f32,
		outer_angle: f32,
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
		let chromaticity = color.resolve()?;
		let candela = intensity.cone_candela(inner_angle, outer_angle)?;
		Ok(Self {
			position,
			direction,
			color: Vector3::new(chromaticity.x * candela, chromaticity.y * candela, chromaticity.z * candela),
			inner_angle,
			outer_angle,
			shadow_near_override: None,
			shadow_far_override: None,
		})
	}

	/// Overrides the renderer-derived near clipping distance for this light's shadow view.
	pub fn with_shadow_near(mut self, shadow_near: f32) -> Self {
		self.shadow_near_override = Some(shadow_near);
		self
	}

	/// Overrides the renderer-derived far clipping distance for this light's shadow view.
	pub fn with_shadow_far(mut self, shadow_far: f32) -> Self {
		self.shadow_far_override = Some(shadow_far);
		self
	}

	/// Overrides both renderer-derived clipping distances for this light's shadow view.
	pub fn with_shadow_range(mut self, shadow_near: f32, shadow_far: f32) -> Self {
		self.shadow_near_override = Some(shadow_near);
		self.shadow_far_override = Some(shadow_far);
		self
	}

	/// Returns the optional near clipping-distance override for the renderer.
	pub(crate) fn shadow_near_override(&self) -> Option<f32> {
		self.shadow_near_override
	}

	/// Returns the optional far clipping-distance override for the renderer.
	pub(crate) fn shadow_far_override(&self) -> Option<f32> {
		self.shadow_far_override
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
	fn cone_light_keeps_shadow_range_overrides() {
		let light = ConeLight::new(
			Vector3::new(1.0, 2.0, 3.0),
			Vector3::new(0.0, -1.0, 0.0),
			LightColor::Kelvin(4_500.0),
			intensity(),
			0.25,
			0.5,
		)
		.expect("physical cone light")
		.with_shadow_range(0.2, 75.0);

		assert_eq!(light.shadow_near_override(), Some(0.2));
		assert_eq!(light.shadow_far_override(), Some(75.0));
	}

	#[test]
	fn individual_shadow_range_overrides_replace_only_their_endpoint() {
		let light = ConeLight::new(
			Vector3::zero(),
			Vector3::unit_z(),
			LightColor::Kelvin(4_500.0),
			intensity(),
			0.25,
			0.5,
		)
		.expect("physical cone light")
		.with_shadow_range(0.2, 75.0)
		.with_shadow_near(0.4)
		.with_shadow_far(50.0);

		assert_eq!(light.shadow_near_override(), Some(0.4));
		assert_eq!(light.shadow_far_override(), Some(50.0));
	}

	#[test]
	fn cone_light_wider_than_a_perspective_view_remains_valid_but_unshadowed() {
		let light = ConeLight::new(
			Vector3::zero(),
			Vector3::unit_z(),
			LightColor::Kelvin(4_500.0),
			intensity(),
			0.25,
			std::f32::consts::PI,
		)
		.expect("physical cone light");

		assert!(!light.supports_shadow_mapping());
	}
}
