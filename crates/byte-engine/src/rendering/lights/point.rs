use super::{LightColor, LocalEmission, PhotometricError, PhotometricIntensity};
use crate::{
	inspector::Inspectable,
	rendering::lights::{Light, LightClasses},
};

/// The `PointLight` struct provides photometric settings for omnidirectional local emitters.
///
/// Use the associated [`crate::gameplay::Transform`] to place the light and orient an optional IES profile.
#[derive(Debug, Clone, PartialEq)]
pub struct PointLight {
	/// The color, optional IES profile, and shadow-range overrides every local light shares.
	pub emission: LocalEmission,
}

impl PointLight {
	/// Creates a point light whose GPU color is luminous intensity in candela.
	///
	/// The renderer derives cube-shadow coverage from the resolved luminous intensity. Use
	/// [`Self::with_shadow_near`], [`Self::with_shadow_far`], or [`Self::with_shadow_range`] to
	/// override that range. Next, pass the returned light to
	/// [`crate::core::factory::Creator::create`] on [`crate::gameplay::world::DefaultWorld`] to make it
	/// available to the active rendering pipeline.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color or intensity contains an invalid physical value.
	pub fn new(color: LightColor, intensity: PhotometricIntensity) -> Result<Self, PhotometricError> {
		let chromaticity = color.resolve()?;
		Ok(Self {
			emission: LocalEmission::uniform(chromaticity, intensity.point_candela()?),
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
	/// Next, pass the returned light to [`crate::core::factory::Creator::create`] on
	/// [`crate::gameplay::world::DefaultWorld`].
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
			emission: LocalEmission::ies(color, dimmer, ies_profile_resource_id)?,
		})
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
