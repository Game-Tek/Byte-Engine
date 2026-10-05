use math::{Radians, UnitVector};
use maths_rs::Vec3f;

use crate::{
	core::{
		factory::{CreateMessage, Handle},
		listener::{DefaultListener, Listener},
	},
	gameplay::transform::TransformationUpdate,
	rendering::DirectionalLight,
};

/// The `Sun` struct follows the newest [`DirectionalLight`], so every pass that draws sunlight in the air, such as
/// [`super::sky::AtmosphereSkyRenderPass`] and [`super::height_fog::ExponentialHeightFogRenderPass`], agrees on which
/// light is the sun.
pub(crate) struct Sun {
	lights: DefaultListener<CreateMessage<DirectionalLight>>,
	transforms: DefaultListener<TransformationUpdate>,
	light: Option<Handle>,
	/// The RGB illuminance in lux of the newest directional light. It stays black until a light is created.
	pub(super) illuminance: Vec3f,
	/// The angular radius of the newest directional light's disk.
	pub(super) angular_radius: Radians,
	/// The direction toward the sun, or `None` until the light's first transform arrives.
	pub(super) direction: Option<UnitVector>,
}

impl Sun {
	/// Creates a sun that adopts lights and transforms published after this call.
	pub(crate) fn new(
		lights: DefaultListener<CreateMessage<DirectionalLight>>,
		transforms: DefaultListener<TransformationUpdate>,
	) -> Self {
		Self {
			lights,
			transforms,
			light: None,
			illuminance: Vec3f::new(0.0, 0.0, 0.0),
			angular_radius: DirectionalLight::SUN_ANGULAR_RADIUS,
			direction: None,
		}
	}

	/// Adopts the newest directional light and its latest orientation. Returns the sun's new direction when it moved.
	pub(crate) fn update(&mut self) -> Option<UnitVector> {
		while let Some(message) = self.lights.read() {
			self.light = Some(message.handle());
			self.illuminance = message.data().color;
			self.angular_radius = message.data().angular_radius;
		}

		let mut moved = None;
		while let Some(message) = self.transforms.read() {
			if self.light == Some(message.handle()) {
				// Directional-light orientation points along ray travel; scattering needs the direction toward the sun.
				self.direction = Some(-math::direction_from_orientation(message.transform().get_orientation()));
				moved = self.direction;
			}
		}
		moved
	}
}
