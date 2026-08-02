use math::Vector3;

use super::{LightColor, PhotometricError, PhotometricIntensity};
use crate::{
	core::{Entity, EntityHandle},
	inspector::Inspectable,
	rendering::lights::{Light, LightClasses},
};

/// The `DirectionalLight` struct provides parallel scene lighting from a distant
/// source, such as the sun.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectionalLight {
	pub direction: Vector3,
	pub color: Vector3,
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
	pub fn new(direction: Vector3, color: LightColor, intensity: PhotometricIntensity) -> Result<Self, PhotometricError> {
		let chromaticity = color.resolve()?;
		let lux = intensity.directional_lux()?;
		Ok(Self {
			direction,
			color: Vector3::new(chromaticity.x * lux, chromaticity.y * lux, chromaticity.z * lux),
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
