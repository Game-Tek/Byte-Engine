/// The `ConeLight` struct provides photometric settings for local lighting constrained to a cone.
///
/// Use it for spotlights, flashlights, and other emitters that need a soft transition between a
/// fully lit inner cone and an unlit outer cone. Cone angles are half angles measured in radians
/// from the associated [`crate::gameplay::Transform`]'s forward direction.
#[derive(Debug, Clone, PartialEq)]
pub struct ConeLight {
	pub color: Vec3f,
	pub inner_angle: Radians,
	pub outer_angle: Radians,
	ies_profile: Option<IesProfile>,
	shadow_near_override: Option<f32>,
	shadow_far_override: Option<f32>,
}

impl ConeLight {
	/// Creates a cone light whose intensity fades smoothly between the inner and outer half angles.
	///
	/// The renderer derives the shadow range from the resolved luminous intensity. Use
	/// [`Self::with_shadow_near`], [`Self::with_shadow_far`], or [`Self::with_shadow_range`] to
	/// override that range. Next, pass the returned light to
	/// [`crate::core::factory::Creator::create`] on [`crate::gameplay::world::DefaultWorld`] to make it
	/// available to the active rendering pipeline.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color or intensity contains an invalid physical value.
	/// Invalid angles panic because they cannot form a valid cone view.
	pub fn new(
		color: LightColor,
		intensity: PhotometricIntensity,
		inner_angle: Radians,
		outer_angle: Radians,
	) -> Result<Self, PhotometricError> {
		Self::validate_angles(inner_angle, outer_angle);
		let chromaticity = color.resolve()?;
		let candela = intensity.cone_candela(inner_angle, outer_angle)?;
		Ok(Self {
			color: Vec3f::new(chromaticity.x * candela, chromaticity.y * candela, chromaticity.z * candela),
			inner_angle,
			outer_angle,
			ies_profile: None,
			shadow_near_override: None,
			shadow_far_override: None,
		})
	}

	/// Creates a cone light whose calibrated intensity and angular distribution come from a baked IES profile.
	///
	/// The associated [`crate::gameplay::Transform`] maps local `+Z` to the emission axis, local `+X`
	/// to the IES C0 tangent, and local `+Y` to the C90 tangent. `color` tints the profile with unit
	/// luminance. `dimmer` is a
	/// linear fraction from `0.0` for off through `1.0` for the measured output. The visibility pipeline
	/// resolves `ies_profile_resource_id` asynchronously, then uses its dimmed candela scale and intensity
	/// map. Until that upload completes, the light uses its dimmed unit-luminance color as a low-intensity
	/// fallback. The cone cutoff still applies after the IES lookup.
	///
	/// Next, pass the returned light to [`crate::core::factory::Creator::create`] on
	/// [`crate::gameplay::world::DefaultWorld`].
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when `color` cannot resolve to a physical chromaticity.
	///
	/// # Panics
	///
	/// Panics when the cone angles are invalid, `dimmer` is outside `0.0..=1.0`, or
	/// `ies_profile_resource_id` is empty. The most likely cause is an invalid cone shape, dimmer, or
	/// missing baked `.ies` resource path.
	pub fn new_ies(
		color: LightColor,
		dimmer: f32,
		ies_profile_resource_id: impl Into<String>,
		inner_angle: Radians,
		outer_angle: Radians,
	) -> Result<Self, PhotometricError> {
		Self::validate_angles(inner_angle, outer_angle);
		let chromaticity = color.resolve()?;
		Ok(Self {
			color: chromaticity,
			inner_angle,
			outer_angle,
			ies_profile: Some(IesProfile::new(ies_profile_resource_id, dimmer)),
			shadow_near_override: None,
			shadow_far_override: None,
		})
	}

	/// Returns the optional IES profile that supplies this cone light's intensity distribution.
	pub fn ies_profile(&self) -> Option<&IesProfile> {
		self.ies_profile.as_ref()
	}

	/// Validates the angular range shared by uniform and IES-backed cone lights.
	fn validate_angles(inner_angle: Radians, outer_angle: Radians) {
		assert!(
			inner_angle.is_finite() && outer_angle.is_finite() && inner_angle >= Radians::new(0.0) && inner_angle < outer_angle,
			"Invalid cone light angles. The most likely cause is that the angles are not finite or the inner angle is not smaller than the outer angle."
		);
		assert!(
			outer_angle <= Radians::new(std::f32::consts::PI),
			"Invalid cone light outer angle. The most likely cause is that the supplied half angle exceeds pi radians."
		);
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
		self.outer_angle < Radians::new(std::f32::consts::FRAC_PI_2)
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
	use math::Radians;

	use super::ConeLight;
	use crate::rendering::lights::{IesProfile, LightColor, PhotometricIntensity};

	fn intensity() -> PhotometricIntensity {
		PhotometricIntensity::LuminousIntensity {
			candela: 100.0,
			reference_distance_m: 1.0,
		}
	}

	#[test]
	fn ies_cone_keeps_its_profile() {
		let light = ConeLight::new_ies(
			LightColor::Kelvin(4_500.0),
			0.25,
			"lights/office.ies",
			Radians::new(0.25),
			Radians::new(0.5),
		)
		.expect("physical IES cone light");

		assert_eq!(
			light.ies_profile().map(|profile| profile.resource_id()),
			Some("lights/office.ies")
		);
		assert_eq!(light.ies_profile().map(IesProfile::dimmer), Some(0.25));
	}

	#[test]
	fn cone_light_keeps_shadow_range_overrides() {
		let light = ConeLight::new(
			LightColor::Kelvin(4_500.0),
			intensity(),
			Radians::new(0.25),
			Radians::new(0.5),
		)
		.expect("physical cone light")
		.with_shadow_range(0.2, 75.0);

		assert_eq!(light.shadow_near_override(), Some(0.2));
		assert_eq!(light.shadow_far_override(), Some(75.0));
	}

	#[test]
	fn individual_shadow_range_overrides_replace_only_their_endpoint() {
		let light = ConeLight::new(
			LightColor::Kelvin(4_500.0),
			intensity(),
			Radians::new(0.25),
			Radians::new(0.5),
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
			LightColor::Kelvin(4_500.0),
			intensity(),
			Radians::new(0.25),
			Radians::new(std::f32::consts::PI),
		)
		.expect("physical cone light");

		assert!(!light.supports_shadow_mapping());
	}
}

use math::Radians;
use maths_rs::Vec3f;

use super::{IesProfile, LightColor, PhotometricError, PhotometricIntensity};
use crate::{
	core::{Entity, EntityHandle},
	inspector::Inspectable,
	rendering::lights::{Light, LightClasses},
};
