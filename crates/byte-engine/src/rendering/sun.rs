use math::{Radians, UnitVector};
use maths_rs::Vec3f;

use crate::{gameplay::Transform, rendering::DirectionalLight};

/// The `Sun` struct is the light that passes drawing sunlight in the air, such as
/// [`super::render_passes::sky::AtmosphereSkyRenderPass`] and
/// [`super::render_passes::height_fog::ExponentialHeightFogRenderPass`], read from [`super::Sink::sun`], so every pass
/// of every window agrees on which light is the sun.
///
/// The renderer takes the newest [`DirectionalLight`] as the sun, whenever it was created, and places it with the
/// transform the frame shows. Passes keep no light or transform listener of their own.
#[derive(Clone, Copy, Debug)]
pub struct Sun {
	/// The RGB illuminance, in lux, that the sunlight delivers where it reaches the camera.
	pub illuminance: Vec3f,
	/// The angular radius of the sun's disk.
	pub angular_radius: Radians,
	/// The direction toward the sun.
	pub direction: UnitVector,
}

impl Sun {
	/// Returns the sun that `light` gives when the frame shows it with `transform`.
	pub fn new(light: &DirectionalLight, transform: &Transform) -> Self {
		Self {
			illuminance: light.color,
			angular_radius: light.angular_radius,
			// A directional light's orientation points along ray travel; scattering needs the direction toward the sun.
			direction: -math::direction_from_orientation(transform.get_orientation()),
		}
	}
}

#[cfg(test)]
mod tests {
	use math::{UnitVector, Vector};

	use crate::{
		gameplay::Transform,
		rendering::{DirectionalLight, LightColor, PhotometricIntensity},
	};

	/// Verifies that the sun points toward the light, against the direction its rays travel, and takes the light's
	/// illuminance and disk.
	#[test]
	fn sun_points_against_the_light_rays() {
		let light = DirectionalLight::new(LightColor::kelvin(6000.0), PhotometricIntensity::illuminance(100_000.0)).unwrap();
		let rays: UnitVector = Vector::new(-0.5f32, -1.0, 0.25).normalized().unwrap();
		let sun = super::Sun::new(&light, &Transform::from_rotation(math::orientation_from_direction(rays)));

		for (toward, ray) in [(sun.direction.x(), rays.x()), (sun.direction.y(), rays.y()), (sun.direction.z(), rays.z())] {
			assert!((toward + ray).abs() < 1e-5, "sun {:?}, rays {rays:?}", sun.direction);
		}
		assert_eq!(sun.illuminance, light.color);
		assert_eq!(sun.angular_radius.value(), light.angular_radius.value());
	}
}
