use math::Vector3;

use super::{LightColor, PhotometricError, PhotometricIntensity};
use crate::{
	core::{Entity, EntityHandle},
	inspector::Inspectable,
	rendering::lights::{Light, LightClasses},
};

/// The `PointLight` struct provides omnidirectional scene lighting from a local
/// source, such as a light bulb.
#[derive(Debug, Clone, Copy)]
pub struct PointLight {
	pub position: Vector3,
	pub color: Vector3,
}

impl PointLight {
	/// Creates a point light whose GPU color is luminous intensity in candela.
	///
	/// Submit the returned light through [`crate::gameplay::world::DefaultWorld::light_factory_mut`]
	/// to make it available to the active rendering pipeline.
	///
	/// # Errors
	///
	/// Returns [`PhotometricError`] when the color or intensity contains an invalid physical value.
	pub fn new(position: Vector3, color: LightColor, intensity: PhotometricIntensity) -> Result<Self, PhotometricError> {
		let chromaticity = color.resolve()?;
		let candela = intensity.point_candela()?;
		Ok(Self {
			position,
			color: Vector3::new(chromaticity.x * candela, chromaticity.y * candela, chromaticity.z * candela),
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
