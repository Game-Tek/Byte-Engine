use crate::core::Entity;

pub mod directional;
pub mod point;

pub use directional::DirectionalLight;
pub use point::PointLight;

pub use directional::DirectionalLight as Directional;
pub use point::PointLight as Point;

pub trait Light: Entity {
	fn class(&self) -> LightClasses;
}

pub enum LightClasses {
	Directional,
	Point,
}
