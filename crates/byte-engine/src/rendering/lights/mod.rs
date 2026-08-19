//! Light entities consumed by scene rendering pipelines.
//!
//! Create [`ConeLight`], [`DirectionalLight`], or [`PointLight`] values and submit them through
//! [`crate::gameplay::world::DefaultWorld::light_factory_mut`]. Use [`ConeLight::new_ies`] or
//! [`PointLight::new_ies`] with an [`math::Orientation`] when a local fixture uses a measured IES
//! profile. [`Lights`] is
//! the erased representation used by the world factory.
//!
//! Light positions and photometric reference distances use meters. The renderer resolves authored
//! units on the CPU and sends scene-referred RGB lux or candela to the GPU.
//!
//! Follow the [physically based lighting guide](https://byte-engine.0x44491229.dev/docs/use/lighting)
//! to choose units and submit a light to the active world.

use crate::core::Entity;

pub mod cone;
pub mod directional;
mod ies_profile;
mod photometry;
pub mod point;

pub use cone::ConeLight;
pub use cone::ConeLight as Cone;
pub use directional::DirectionalLight;
pub use directional::DirectionalLight as Directional;
pub use ies_profile::IesProfile;
pub use photometry::{LightColor, PhotometricError, PhotometricIntensity};
pub use point::PointLight;
pub use point::PointLight as Point;

/// The `Light` trait identifies the shader and storage class of a scene light.
pub trait Light {
	fn class(&self) -> LightClasses;
}

/// The [`LightClasses`] enum identifies the shader and storage layout required by
/// a light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightClasses {
	Cone,
	Directional,
	Point,
}

#[derive(Clone)]
/// The [`Lights`] enum carries supported concrete light values through world
/// creation messages.
pub enum Lights {
	Cone(ConeLight),
	Direction(DirectionalLight),
	Point(PointLight),
}

impl From<ConeLight> for Lights {
	fn from(val: ConeLight) -> Self {
		Lights::Cone(val)
	}
}

impl From<PointLight> for Lights {
	fn from(val: PointLight) -> Self {
		Lights::Point(val)
	}
}

impl From<DirectionalLight> for Lights {
	fn from(val: DirectionalLight) -> Self {
		Lights::Direction(val)
	}
}

#[cfg(test)]
mod tests {
	use math::{Point, UnitVector, Vector};
	use maths_rs::Vec3f;

	use super::*;
	use crate::inspector::Inspectable;

	fn white() -> LightColor {
		LightColor::LinearSrgb(Vec3f::new(1.0, 1.0, 1.0))
	}

	fn candela(value: f32) -> PhotometricIntensity {
		PhotometricIntensity::LuminousIntensity {
			candela: value,
			reference_distance_m: 1.0,
		}
	}

	#[test]
	fn concrete_lights_preserve_spatial_state_temperature_color_and_class() {
		let cone = ConeLight::new(
			Point::new(1.0, 2.0, 3.0),
			-UnitVector::y_axis(),
			LightColor::Kelvin(3_200.0),
			PhotometricIntensity::LuminousFlux {
				lumens: 1_000.0,
				directional_beam_area_m2: 1.0,
			},
			15.0_f32.to_radians(),
			25.0_f32.to_radians(),
		)
		.expect("physical cone light");
		let point = PointLight::new(Point::new(1.0, 2.0, 3.0), white(), candela(250.0)).expect("physical point light");
		let directional = DirectionalLight::new(
			Vector::new(-1.0, -2.0, -3.0).normalized().expect("nonzero direction"),
			white(),
			PhotometricIntensity::Illuminance {
				lux: 10_000.0,
				measurement_distance_m: 1.0,
			},
		)
		.expect("physical directional light");

		assert_eq!(cone.position, Point::new(1.0, 2.0, 3.0));
		assert_eq!(cone.direction(), -UnitVector::y_axis());
		assert!(cone.color.x > cone.color.z);
		assert_eq!(cone.class(), LightClasses::Cone);
		assert!(cone.as_string().contains("ConeLight"));
		assert_eq!(point.position, Point::new(1.0, 2.0, 3.0));
		assert_eq!(point.color, Vec3f::new(250.0, 250.0, 250.0));
		assert_eq!(point.class(), LightClasses::Point);
		assert!(point.as_string().contains("PointLight"));
		assert_eq!(
			directional.direction,
			Vector::new(-1.0, -2.0, -3.0).normalized().expect("nonzero direction")
		);
		assert_eq!(directional.color, Vec3f::new(10_000.0, 10_000.0, 10_000.0));
		assert_eq!(directional.class(), LightClasses::Directional);
		assert!(directional.as_string().contains("DirectionalLight"));
	}

	#[test]
	fn erased_light_conversion_preserves_the_concrete_variant_and_payload() {
		let cone = ConeLight::new(
			Point::new(0.0, 2.0, 0.0),
			-UnitVector::y_axis(),
			white(),
			candela(100.0),
			0.25,
			0.5,
		)
		.expect("physical cone light");
		let point = PointLight::new(Point::new(1.0, 0.0, 0.0), white(), candela(100.0)).expect("physical point light");
		let directional = DirectionalLight::new(
			-UnitVector::y_axis(),
			white(),
			PhotometricIntensity::Illuminance {
				lux: 5_000.0,
				measurement_distance_m: 1.0,
			},
		)
		.expect("physical directional light");

		assert!(matches!(Lights::from(cone.clone()), Lights::Cone(light) if light == cone));
		assert!(
			matches!(Lights::from(point.clone()), Lights::Point(light) if light.position == point.position && light.color == point.color)
		);
		assert!(matches!(Lights::from(directional.clone()), Lights::Direction(light) if light == directional));
	}
}
