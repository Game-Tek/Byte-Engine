use math::Vector3;

use crate::core::entity::EntityBuilder;
use crate::core::{Entity, EntityHandle};
use crate::gameplay::{Positionable, Transformable};
use crate::inspector::Inspectable;

#[derive(Debug)]
pub struct Camera {
	position: Vector3,
	direction: Vector3,
	fov: f32,
	aspect_ratio: f32,
	aperture: f32,
	focus_distance: f32,
}

impl Camera {
	pub fn new(position: Vector3) -> Self {
		Self {
			position,
			direction: Vector3::new(0.0, 0.0, 1.0),
			fov: 45.0,
			aspect_ratio: 1.0,
			aperture: 0.0,
			focus_distance: 0.0,
		}
	}

	/// Returns the field of view of the camera
	pub fn get_fov(&self) -> f32 { self.fov }

	/// Returns the aspect ratio of the camera
	fn get_aspect_ratio(&self) -> f32 { self.aspect_ratio }

	/// Returns the aperture of the camera
	fn get_aperture(&self) -> f32 { self.aperture }

	/// Returns the focus distance of the camera
	fn get_focus_distance(&self) -> f32 { self.focus_distance }

	pub fn get_orientation(&self) -> Vector3 { self.direction }
	pub fn set_orientation(&mut self, orientation: Vector3) { self.direction = orientation; }

	pub fn get_position(&self) -> Vector3 { self.position }
	pub fn set_position(&mut self, position: Vector3) { self.position = position; }

	pub fn set_fov(&mut self, fov: f32) {
		self.fov = fov;
	}
}

impl Entity for Camera {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
    	EntityBuilder::new(self).r#as(|h| h).r#as(|h| h as EntityHandle<dyn Inspectable>)
	}
}

impl Positionable for Camera {
	fn get_position(&self) -> Vector3 { self.position }
	fn set_position(&mut self, position: Vector3) { self.position = position; }
}

impl Inspectable for Camera {
	fn as_string(&self) -> String {
    	format!("{:?}", self)
	}

	fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
    	match key {
        	"fov" => {
        		self.set_fov(value.parse().map_err(|e| format!("Invalid value: {}", e))?);
          		Ok(())
        	},
        	_ => Err(format!("Unknown key: {}", key))
    	}
	}
}
