/// The `DirectionalLight` struct provides photometric settings for parallel scene lighting from a distant source.
///
/// Use the associated [`crate::gameplay::Transform`] to orient sources such as the sun.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectionalLight {
	pub color: Vec3f,
}

impl DirectionalLight {
	/// Creates a directional light whose GPU color is scene illuminance in lux.
	///
	/// Submit the returned light through [`crate::gameplay::world::DefaultWorld::light_factory_mut`]
	/// to make it available to the active rendering pipeline.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color or intensity contains an invalid physical value.
	pub fn new(color: LightColor, intensity: PhotometricIntensity) -> Result<Self, PhotometricError> {
		let chromaticity = color.resolve()?;
		let lux = intensity.directional_lux()?;
		Ok(Self {
			color: Vec3f::new(chromaticity.x * lux, chromaticity.y * lux, chromaticity.z * lux),
		})
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

use maths_rs::Vec3f;

use super::{LightColor, PhotometricError, PhotometricIntensity};
use crate::{
	core::{Entity, EntityHandle},
	inspector::Inspectable,
	rendering::lights::{Light, LightClasses},
};
