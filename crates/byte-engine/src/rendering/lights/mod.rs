//! Light entities consumed by scene rendering pipelines.
//!
//! Create [`DirectionalLight`] or [`PointLight`] values and submit them through
//! [`crate::gameplay::world::DefaultWorld::light_factory_mut`]. [`Lights`] is
//! the erased representation used by the world factory.

use crate::core::Entity;

pub mod directional;
pub mod point;

pub use directional::DirectionalLight;
pub use directional::DirectionalLight as Directional;
pub use point::PointLight;
pub use point::PointLight as Point;

/// The [`Light`] trait identifies the rendering class of a light implementation.
pub trait Light {
	fn class(&self) -> LightClasses;
}

/// The [`LightClasses`] enum identifies the shader and storage layout required by
/// a light.
pub enum LightClasses {
	Directional,
	Point,
}

#[derive(Clone)]
/// The [`Lights`] enum carries supported concrete light values through world
/// creation messages.
pub enum Lights {
	Direction(DirectionalLight),
	Point(PointLight),
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
