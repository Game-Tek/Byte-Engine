/// The `ConeLight` struct provides photometric settings for local lighting constrained to a cone.
///
/// Use it for spotlights, flashlights, and other emitters that need a soft transition between a
/// fully lit inner cone and an unlit outer cone. Cone angles are half angles measured in radians
/// from the associated [`crate::gameplay::Transform`]'s forward direction.
#[derive(Debug, Clone, PartialEq)]
pub struct ConeLight {
	/// The color, optional IES profile, and shadow-range overrides every local light shares.
	pub emission: LocalEmission,
	pub inner_angle: Radians,
	pub outer_angle: Radians,
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
		Ok(Self {
			emission: LocalEmission::uniform(chromaticity, intensity.cone_candela(inner_angle, outer_angle)?),
			inner_angle,
			outer_angle,
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
		Ok(Self {
			emission: LocalEmission::ies(color, dimmer, ies_profile_resource_id)?,
			inner_angle,
			outer_angle,
		})
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

use math::Radians;

use super::{LightColor, LocalEmission, PhotometricError, PhotometricIntensity};
use crate::{
	inspector::Inspectable,
	rendering::lights::{Light, LightClasses},
};
