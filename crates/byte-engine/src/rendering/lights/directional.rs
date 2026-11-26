/// The `DirectionalLight` struct provides photometric settings for parallel scene lighting from a distant source.
///
/// Use the associated [`crate::gameplay::Transform`] to orient sources such as the sun. It keeps the color and
/// intensity it was authored with, which facet and serde read and write as a level's
/// [`directional_light` kind](/docs/reference/levels#lights) and the inspector's `DirectionalLightUpdate` payload. To
/// change it later, create it again under its own handle.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
#[facet(opaque, proxy = SerializableDirectionalLight)]
pub struct DirectionalLight {
	color: LightColor,
	intensity: PhotometricIntensity,
	/// The color and intensity resolved to RGB illuminance in lux.
	pub(crate) illuminance: Vec3f,
	/// The angular radius of the light's disk as seen from the scene. It sets how quickly shadows soften with the
	/// distance between an occluder and the surface it shades, and the size of the sun disk the atmosphere sky draws.
	/// Set it with [`Self::with_angular_radius`].
	pub(crate) angular_radius: Radians,
	/// The light's [blinds](/docs/reference/lighting#blinds) as an 8-bit unorm. Set it with [`Self::with_blinds`].
	pub(crate) blinds: u8,
}

impl DirectionalLight {
	/// The sun's angular radius as seen from Earth, about 0.27 degrees. New directional lights use it.
	pub const SUN_ANGULAR_RADIUS: Radians = Radians::new(0.004675);

	/// Creates a directional light of `color` and `intensity`, whose GPU color is scene illuminance in lux.
	///
	/// Next, pass the returned light to [`crate::core::factory::Creator::create`] on
	/// the world scope ([`crate::core::message_bus::MessageScope`]) to make it available to the active rendering pipeline.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color or intensity contains an invalid physical value, or the intensity
	/// names an IES profile, which only local lights take.
	pub fn new(color: LightColor, intensity: PhotometricIntensity) -> Result<Self, PhotometricError> {
		let chromaticity = color.resolve()?;
		let lux = intensity.directional_lux()?;
		Ok(Self {
			color,
			intensity,
			illuminance: Vec3f::new(chromaticity.x * lux, chromaticity.y * lux, chromaticity.z * lux),
			angular_radius: Self::SUN_ANGULAR_RADIUS,
			blinds: 0,
		})
	}

	/// Returns the color the light was authored with.
	pub fn color(&self) -> LightColor {
		self.color
	}

	/// Returns the intensity the light was authored with.
	pub fn intensity(&self) -> &PhotometricIntensity {
		&self.intensity
	}

	/// Returns the angular radius of the light's disk, in radians.
	pub fn angular_radius(&self) -> Radians {
		self.angular_radius
	}

	/// Returns this light with a disk of `angular_radius` radians, instead of the sun's
	/// [`Self::SUN_ANGULAR_RADIUS`].
	///
	/// A larger disk gives wider, softer penumbrae, and they still sharpen where an occluder meets the surface it
	/// shades. The atmosphere sky draws a larger, dimmer sun disk to match, because the same illuminance spreads over
	/// more of the sky.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when `angular_radius` is not finite or lies outside `0.0..π/2`.
	pub fn with_angular_radius(mut self, angular_radius: Radians) -> Result<Self, PhotometricError> {
		if !(angular_radius.is_finite()
			&& angular_radius >= Radians::new(0.0)
			&& angular_radius < Radians::new(std::f32::consts::FRAC_PI_2))
		{
			return Err(PhotometricError::new(
				"that the angular radius isn't from 0 up to π/2, as when it's in degrees or a full angle. Give the disk's radius in radians",
			));
		}
		self.angular_radius = angular_radius;
		Ok(self)
	}

	/// Returns this light with `blinds` from `0.0` through `1.0`: how much the camera must face the light to see the
	/// light it gives.
	///
	/// Use `1.0` for a backlight that lights a subject only while the camera looks toward it. The light keeps the blinds
	/// to 1/255. See [blinds](/docs/reference/lighting#blinds) for the exact weight.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when `blinds` is not finite or lies outside `0.0..=1.0`.
	pub fn with_blinds(mut self, blinds: f32) -> Result<Self, PhotometricError> {
		self.blinds = super::blinds_unorm(blinds)?;
		Ok(self)
	}

	/// Returns the light's blinds, from `0.0` through `1.0`.
	pub fn blinds(&self) -> f32 {
		super::blinds_value(self.blinds)
	}
}

/// The `SerializableDirectionalLight` struct is the checked reflection boundary of [`DirectionalLight`], in the
/// `directional_light` payload's shape. A missing `angular_radius` is the sun's, and missing `blinds` are 0.
#[derive(facet::Facet, serde::Serialize, serde::Deserialize)]
#[facet(deny_unknown_fields)]
#[serde(deny_unknown_fields)]
pub(crate) struct SerializableDirectionalLight {
	color: LightColor,
	intensity: PhotometricIntensity,
	#[facet(default)]
	#[serde(default)]
	angular_radius: Option<f32>,
	#[facet(default)]
	#[serde(default)]
	blinds: f32,
}

impl TryFrom<SerializableDirectionalLight> for DirectionalLight {
	type Error = String;

	/// Checks the payload against the light's rules before building it.
	fn try_from(value: SerializableDirectionalLight) -> Result<Self, Self::Error> {
		let light = DirectionalLight::new(value.color, value.intensity)
			.and_then(|light| light.with_blinds(value.blinds))
			.map_err(|error| error.to_string())?;
		match value.angular_radius {
			Some(radius) => light
				.with_angular_radius(Radians::new(radius))
				.map_err(|error| error.to_string()),
			None => Ok(light),
		}
	}
}

impl From<&DirectionalLight> for SerializableDirectionalLight {
	fn from(light: &DirectionalLight) -> Self {
		Self {
			color: light.color,
			intensity: light.intensity.clone(),
			angular_radius: Some(light.angular_radius.value()),
			blinds: light.blinds(),
		}
	}
}

super::serde_through_proxy!(DirectionalLight, SerializableDirectionalLight);

impl Light for DirectionalLight {
	fn class(&self) -> LightClasses {
		LightClasses::Directional
	}
}

use math::Radians;
use maths_rs::Vec3f;

use super::{LightColor, PhotometricError, PhotometricIntensity};
use crate::rendering::lights::{Light, LightClasses};
