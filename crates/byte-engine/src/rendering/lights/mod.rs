//! Light entities consumed by scene rendering pipelines.
//!
//! Create [`ConeLight`], [`DirectionalLight`], or [`PointLight`] values and submit them through
//! [`crate::gameplay::world::DefaultWorld::light_factory_mut`]. [`Lights`] is
//! the erased representation used by the world factory.

use crate::core::Entity;

pub mod cone;
pub mod directional;
pub mod point;

pub use cone::ConeLight;
pub use cone::ConeLight as Cone;
pub use directional::DirectionalLight;
pub use directional::DirectionalLight as Directional;
pub use point::PointLight;
pub use point::PointLight as Point;

/// The `Light` trait exists to identify the shader and storage class of a scene light.
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
	use math::Vector3;

	use super::*;
	use crate::{inspector::Inspectable, rendering::cct};

	#[test]
	fn concrete_lights_preserve_spatial_state_temperature_color_and_class() {
		let cone = ConeLight::new(
			Vector3::new(1.0, 2.0, 3.0),
			Vector3::new(0.0, -1.0, 0.0),
			3_200.0,
			15.0_f32.to_radians(),
			25.0_f32.to_radians(),
		);
		let point = PointLight::new(Vector3::new(1.0, 2.0, 3.0), 2_500.0);
		let directional = DirectionalLight::new(Vector3::new(-1.0, -2.0, -3.0), 10_000.0);

		assert_eq!(cone.position, Vector3::new(1.0, 2.0, 3.0));
		assert_eq!(cone.direction, Vector3::new(0.0, -1.0, 0.0));
		assert_eq!(cone.color, cct::rgb_from_temperature(3_200.0));
		assert_eq!(cone.class(), LightClasses::Cone);
		assert!(cone.as_string().contains("ConeLight"));

		assert_eq!(point.position, Vector3::new(1.0, 2.0, 3.0));
		assert_eq!(point.color, cct::rgb_from_temperature(2_500.0));
		assert_eq!(point.class(), LightClasses::Point);
		assert!(point.as_string().contains("PointLight"));

		assert_eq!(directional.direction, Vector3::new(-1.0, -2.0, -3.0));
		assert_eq!(directional.color, cct::rgb_from_temperature(10_000.0));
		assert_eq!(directional.class(), LightClasses::Directional);
		assert!(directional.as_string().contains("DirectionalLight"));
	}

	#[test]
	fn erased_light_conversion_preserves_the_concrete_variant_and_payload() {
		let cone = ConeLight::new(Vector3::new(0.0, 2.0, 0.0), Vector3::new(0.0, -1.0, 0.0), 4_500.0, 0.25, 0.5);
		let point = PointLight::new(Vector3::new(1.0, 0.0, 0.0), 6_600.0);
		let directional = DirectionalLight::new(Vector3::new(0.0, -1.0, 0.0), 5_000.0);

		assert!(matches!(Lights::from(cone), Lights::Cone(light) if light == cone));
		assert!(
			matches!(Lights::from(point), Lights::Point(light) if light.position == point.position && light.color == point.color)
		);
		assert!(matches!(Lights::from(directional.clone()), Lights::Direction(light) if light == directional));
	}
}
