/// The `DirectionalLight` struct provides photometric settings for parallel scene lighting from a distant source.
///
/// Use the associated [`crate::gameplay::Transform`] to orient sources such as the sun.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectionalLight {
	pub color: Vec3f,
	/// The angular radius of the light's disk as seen from the scene. It sets how quickly shadows soften with the
	/// distance between an occluder and the surface it shades, and the size of the sun disk the atmosphere sky draws.
	/// Set it with [`Self::with_angular_radius`].
	pub angular_radius: Radians,
}

impl DirectionalLight {
	/// The sun's angular radius as seen from Earth, about 0.27 degrees. New directional lights use it.
	pub const SUN_ANGULAR_RADIUS: Radians = Radians::new(0.004675);

	/// Creates a directional light whose GPU color is scene illuminance in lux.
	///
	/// Next, pass the returned light to [`crate::core::factory::Creator::create`] on
	/// the world scope ([`crate::core::message_bus::MessageScope`]) to make it available to the active rendering pipeline.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color or intensity contains an invalid physical value.
	pub fn new(color: LightColor, intensity: PhotometricIntensity) -> Result<Self, PhotometricError> {
		let chromaticity = color.resolve()?;
		let lux = intensity.directional_lux()?;
		Ok(Self {
			color: Vec3f::new(chromaticity.x * lux, chromaticity.y * lux, chromaticity.z * lux),
			angular_radius: Self::SUN_ANGULAR_RADIUS,
		})
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
}

impl Light for DirectionalLight {
	fn class(&self) -> LightClasses {
		LightClasses::Directional
	}
}

impl Inspectable for DirectionalLight {
	fn as_string(&self) -> String {
		format!("{:?}", self)
	}
}

use math::Radians;
use maths_rs::Vec3f;

use super::{LightColor, PhotometricError, PhotometricIntensity};
use crate::{
	inspector::Inspectable,
	rendering::lights::{Light, LightClasses},
};
