use math::{magnitude_squared, orientation_from_direction, Quaternion, Vector3};

use crate::constants::FORWARD;
use crate::core::{Entity, EntityHandle};
use crate::inspector::Inspectable;
use crate::space::orientable::Orientable;
use crate::space::{Positionable, Transformable};

#[derive(Clone, Debug)]
/// The `Camera` struct exists as the scene-owned view source for render sinks and
/// inspection tools.
pub struct Camera {
	position: Vector3,
	orientation: Quaternion,
	fov: f32,
	aspect_ratio: f32,
	aperture: f32,
	focus_distance: f32,
}

impl Default for Camera {
	fn default() -> Self {
		Self::new()
	}
}

impl Camera {
	/// Creates a camera with a centered position and default perspective settings.
	pub fn new() -> Self {
		Self {
			position: Vector3::new(0.0, 0.0, 0.0),
			orientation: Quaternion::identity(),
			fov: 45.0,
			aspect_ratio: 1.0,
			aperture: 0.0,
			focus_distance: 0.0,
		}
	}

	/// Returns the field of view of the camera
	pub fn get_fov(&self) -> f32 {
		self.fov
	}

	/// Returns the aspect ratio of the camera
	fn get_aspect_ratio(&self) -> f32 {
		self.aspect_ratio
	}

	/// Returns the aperture of the camera
	fn get_aperture(&self) -> f32 {
		self.aperture
	}

	/// Returns the focus distance of the camera
	fn get_focus_distance(&self) -> f32 {
		self.focus_distance
	}

	/// Sets the world-space direction used to build render views from this camera.
	/// A zero vector leaves the current orientation unchanged.
	pub fn set_direction(&mut self, direction: Vector3) {
		if magnitude_squared(direction) > f32::EPSILON {
			self.orientation = orientation_from_direction(direction);
		}
	}

	/// Sets the vertical field of view used by perspective rendering.
	pub fn set_fov(&mut self, fov: f32) {
		self.fov = fov;
	}

	/// Returns the world-space direction used when creating a [`crate::rendering::View`].
	pub fn get_direction(&self) -> Vector3 {
		self.orientation * FORWARD
	}
}

impl Positionable for Camera {
	fn position(&self) -> Vector3 {
		self.position
	}

	fn set_position(&mut self, position: Vector3) {
		self.position = position;
	}
}

impl Orientable for Camera {
	fn orientation(&self) -> Quaternion {
		self.orientation
	}

	fn set_orientation(&mut self, orientation: Quaternion) {
		self.orientation = orientation;
	}
}

impl Inspectable for Camera {
	fn as_string(&self) -> String {
		format!("{:?}", self)
	}

	fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
		match key {
			"fov" => {
				self.set_fov(value.parse().map_err(|e| {
					format!("Invalid camera field value. The most likely cause is that fov is not a number: {e}")
				})?);
				Ok(())
			}
			_ => Err(format!(
				"Unknown camera field. The most likely cause is an unsupported inspector key: {key}"
			)),
		}
	}
}

#[cfg(test)]
mod tests {
	use math::{assert_vec3f_near, normalize};

	use super::*;

	#[test]
	fn defaults_form_a_valid_forward_facing_perspective_camera() {
		let camera = Camera::new();

		assert_vec3f_near!(camera.position(), Vector3::new(0.0, 0.0, 0.0));
		assert_vec3f_near!(camera.get_direction(), FORWARD);
		assert_eq!(camera.get_fov(), 45.0);
		assert_eq!(camera.get_aspect_ratio(), 1.0);
		assert_eq!(camera.get_aperture(), 0.0);
		assert_eq!(camera.get_focus_distance(), 0.0);
	}

	#[test]
	fn set_direction_rotates_forward_to_the_normalized_requested_direction() {
		let mut camera = Camera::new();
		let requested = Vector3::new(2.0, -3.0, -4.0);

		camera.set_direction(requested);

		assert_vec3f_near!(camera.get_direction(), normalize(requested));
	}

	#[test]
	fn zero_direction_does_not_destroy_an_existing_orientation() {
		let mut camera = Camera::new();
		camera.set_direction(Vector3::new(1.0, 0.0, 0.0));
		let direction = camera.get_direction();

		camera.set_direction(Vector3::new(0.0, 0.0, 0.0));

		assert_vec3f_near!(camera.get_direction(), direction);
	}

	#[test]
	fn position_orientation_and_inspector_updates_share_camera_state() {
		let mut camera = Camera::new();
		camera.set_position(Vector3::new(1.0, 2.0, 3.0));
		camera.set_orientation(Quaternion::from_axis_angle(Vector3::new(0.0, 1.0, 0.0), 0.5));
		camera.set("fov", "72.5").expect("numeric field of view");

		assert_vec3f_near!(camera.position(), Vector3::new(1.0, 2.0, 3.0));
		assert_eq!(camera.get_fov(), 72.5);
		assert!(camera.as_string().contains("72.5"));

		let invalid = camera.set("fov", "wide").expect_err("non-numeric field of view");
		assert!(invalid.contains("most likely cause"));
		assert_eq!(camera.get_fov(), 72.5);

		let unknown = camera.set("exposure", "1").expect_err("unsupported field");
		assert!(unknown.contains("most likely cause"));
		assert_eq!(camera.get_fov(), 72.5);
	}
}
