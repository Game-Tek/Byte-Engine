use crate::core::Entity;

pub mod directional;
pub mod point;

pub use directional::DirectionalLight;
pub use directional::DirectionalLight as Directional;
pub use point::PointLight;
pub use point::PointLight as Point;

pub trait Light {
	fn class(&self) -> LightClasses;
}

pub enum LightClasses {
	Directional,
	Point,
}

#[derive(Clone)]
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
