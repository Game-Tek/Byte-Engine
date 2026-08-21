use maths_rs::Vec3f;

use super::{IesProfile, LightColor, PhotometricError, PhotometricIntensity};
use crate::{
	core::{Entity, EntityHandle},
	inspector::Inspectable,
	rendering::lights::{Light, LightClasses},
};

/// The `PointLight` struct provides photometric settings for omnidirectional local emitters.
///
/// Use the associated [`crate::gameplay::Transform`] to place the light and orient an optional IES profile.
#[derive(Debug, Clone, PartialEq)]
pub struct PointLight {
	pub color: Vec3f,
	ies_profile: Option<IesProfile>,
	shadow_near_override: Option<f32>,
	shadow_far_override: Option<f32>,
}

impl PointLight {
	/// Creates a point light whose GPU color is luminous intensity in candela.
	///
	/// The renderer derives cube-shadow coverage from the resolved luminous intensity. Use
	/// [`Self::with_shadow_near`], [`Self::with_shadow_far`], or [`Self::with_shadow_range`] to
	/// override that range. Submit the returned light through
	/// [`crate::gameplay::world::DefaultWorld::light_factory_mut`] to make it available to the active
	/// rendering pipeline.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color or intensity contains an invalid physical value.
	pub fn new(color: LightColor, intensity: PhotometricIntensity) -> Result<Self, PhotometricError> {
		let chromaticity = color.resolve()?;
		let candela = intensity.point_candela()?;
		Ok(Self {
			color: Vec3f::new(chromaticity.x * candela, chromaticity.y * candela, chromaticity.z * candela),
			ies_profile: None,
			shadow_near_override: None,
			shadow_far_override: None,
		})
	}

	/// Creates a point light whose calibrated intensity and angular distribution come from a baked IES profile.
	///
	/// The associated [`crate::gameplay::Transform`] maps local `+Z` to the emission axis, local `+X`
	/// to the IES C0 tangent, and local `+Y` to the C90 tangent. `color` tints the profile with unit
	/// luminance. `dimmer` is a
	/// linear fraction from `0.0` for off through `1.0` for the measured output. The visibility pipeline
	/// resolves `ies_profile_resource_id` asynchronously and applies the image's dimmed candela scale
	/// after it reaches the GPU. Until then, the light uses its dimmed unit-luminance color as a fallback.
	///
	/// Next, submit the returned light through [`crate::gameplay::world::DefaultWorld::light_factory_mut`].
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when `color` cannot resolve to a physical chromaticity.
	///
	/// # Panics
	///
	/// Panics when `dimmer` is outside `0.0..=1.0` or `ies_profile_resource_id` is empty. The most likely
	/// cause is an invalid dimmer or missing baked `.ies` resource path.
	pub fn new_ies(
		color: LightColor,
		dimmer: f32,
		ies_profile_resource_id: impl Into<String>,
	) -> Result<Self, PhotometricError> {
		Ok(Self {
			color: color.resolve()?,
			ies_profile: Some(IesProfile::new(ies_profile_resource_id, dimmer)),
			shadow_near_override: None,
			shadow_far_override: None,
		})
	}

	/// Returns the optional IES profile that supplies this point light's intensity distribution.
	pub fn ies_profile(&self) -> Option<&IesProfile> {
		self.ies_profile.as_ref()
	}

	/// Overrides the renderer-derived near clipping distance for this light's cube shadow map.
	pub fn with_shadow_near(mut self, shadow_near: f32) -> Self {
		self.shadow_near_override = Some(shadow_near);
		self
	}

	/// Overrides the renderer-derived far clipping distance for this light's cube shadow map.
	pub fn with_shadow_far(mut self, shadow_far: f32) -> Self {
		self.shadow_far_override = Some(shadow_far);
		self
	}

	/// Overrides both renderer-derived clipping distances for this light's cube shadow map.
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

#[cfg(test)]
mod tests {
	use super::PointLight;
	use crate::rendering::lights::{IesProfile, LightColor, PhotometricIntensity};

	#[test]
	fn ies_point_keeps_its_profile() {
		let light =
			PointLight::new_ies(LightColor::Kelvin(4_500.0), 0.25, "lights/office.ies").expect("physical IES point light");

		assert_eq!(
			light.ies_profile().map(|profile| profile.resource_id()),
			Some("lights/office.ies")
		);
		assert_eq!(light.ies_profile().map(IesProfile::dimmer), Some(0.25));
	}

	#[test]
	fn point_light_keeps_shadow_range_overrides() {
		let light = PointLight::new(
			LightColor::Kelvin(4_500.0),
			PhotometricIntensity::LuminousIntensity {
				candela: 100.0,
				reference_distance_m: 1.0,
			},
		)
		.expect("physical point light")
		.with_shadow_range(0.2, 75.0)
		.with_shadow_near(0.4)
		.with_shadow_far(50.0);

		assert_eq!(light.shadow_near_override(), Some(0.4));
		assert_eq!(light.shadow_far_override(), Some(50.0));
	}
}
