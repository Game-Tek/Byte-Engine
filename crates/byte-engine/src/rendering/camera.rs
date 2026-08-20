#[derive(Clone, Debug)]
/// The `Camera` struct provides scene-owned world-space view settings to render sinks and inspection tools.
pub struct Camera {
	fov: Degrees,
	aspect_ratio: f32,
	aperture: f32,
	focus_distance: f32,
}

impl Camera {
	/// Creates a camera with a world-origin position and default perspective settings.
	pub fn new() -> Self {
		Self {
			fov: Degrees::new(45.0),
			aspect_ratio: 1.0,
			aperture: 0.0,
			focus_distance: 0.0,
		}
	}

	/// Returns the camera's vertical field of view.
	pub fn vertical_fov(&self) -> Degrees {
		self.fov
	}

	/// Returns the camera's width-to-height aspect ratio.
	pub fn aspect_ratio(&self) -> f32 {
		self.aspect_ratio
	}

	/// Returns the camera aperture.
	pub fn aperture(&self) -> f32 {
		self.aperture
	}

	/// Returns the camera focus distance.
	pub fn focus_distance(&self) -> f32 {
		self.focus_distance
	}

	/// Sets the vertical field of view used by perspective rendering.
	pub fn with_fov(mut self, fov: Degrees) -> Self {
		self.set_fov(fov);
		self
	}

	/// Sets the vertical field of view used by perspective rendering.
	pub fn set_fov(&mut self, fov: Degrees) {
		self.fov = fov;
	}
}

impl Inspectable for Camera {
	fn as_string(&self) -> String {
		format!("{:?}", self)
	}

	fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
		match key {
			"fov" => {
				self.set_fov(Degrees::new(value.parse().map_err(|e| {
					format!("Invalid camera field value. The most likely cause is that fov is not a number: {e}")
				})?));
				Ok(())
			}
			_ => Err(format!(
				"Unknown camera field. The most likely cause is an unsupported inspector key: {key}"
			)),
		}
	}
}

impl Default for Camera {
	fn default() -> Self {
		Self::new()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn defaults_form_a_valid_forward_facing_perspective_camera() {
		let camera = Camera::new();

		assert_eq!(camera.vertical_fov(), Degrees::new(45.0));
		assert_eq!(camera.aspect_ratio(), 1.0);
		assert_eq!(camera.aperture(), 0.0);
		assert_eq!(camera.focus_distance(), 0.0);
	}
}

use math::{direction_from_orientation, orientation_from_direction, Degrees, Orientation, Point, UnitVector, Vector};

use crate::core::{Entity, EntityHandle};
use crate::inspector::Inspectable;
use crate::space::orientable::Orientable;
