use crate::core::Entity;

pub mod directional;
pub mod point;

pub use directional::DirectionalLight;
pub use point::PointLight;

pub use directional::DirectionalLight as Directional;
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

impl Into<Lights> for PointLight {
	fn into(self) -> Lights {
		Lights::Point(self)
	}
}

impl Into<Lights> for DirectionalLight {
	fn into(self) -> Lights {
		Lights::Direction(self)
	}
}
