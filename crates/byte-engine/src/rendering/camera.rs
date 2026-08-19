#[derive(Clone, Debug)]
/// The `Camera` struct provides scene-owned world-space view settings to render sinks and inspection tools.
pub struct Camera {
	position: Point,
	orientation: Orientation,
	fov: Degrees,
	aspect_ratio: f32,
	aperture: f32,
	focus_distance: f32,
}

impl Camera {
	/// Creates a camera with a world-origin position and default perspective settings.
	pub fn new() -> Self {
		Self {
			position: Point::origin(),
			orientation: Orientation::identity(),
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

	/// Returns the camera's world-space position.
	pub fn position(&self) -> Point {
		self.position
	}

	/// Builds the camera with the provided world-space position.
	pub fn with_position(mut self, position: Point) -> Self {
		self.set_position(position);
		self
	}

	/// Sets the world-space position of the camera.
	pub fn set_position(&mut self, position: Point) {
		self.position = position;
	}

	/// Builds the camera with the provided checked world-space direction.
	pub fn with_direction(mut self, direction: UnitVector) -> Self {
		self.set_direction(direction);
		self
	}

	/// Sets the checked world-space direction used to build render views from this camera.
	pub fn set_direction(&mut self, direction: UnitVector) {
		self.orientation = orientation_from_direction(direction);
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

	/// Returns the checked world-space direction used when creating a [`crate::rendering::View`].
	pub fn direction(&self) -> UnitVector {
		direction_from_orientation(self.orientation)
	}
}

impl Orientable for Camera {
	fn orientation(&self) -> Orientation {
		self.orientation
	}

	fn set_orientation(&mut self, orientation: Orientation) {
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

		assert_eq!(camera.position(), Point::origin());
		assert_eq!(camera.direction(), UnitVector::z_axis());
		assert_eq!(camera.vertical_fov(), Degrees::new(45.0));
		assert_eq!(camera.aspect_ratio(), 1.0);
		assert_eq!(camera.aperture(), 0.0);
		assert_eq!(camera.focus_distance(), 0.0);
	}

	#[test]
	fn set_direction_rotates_forward_to_the_requested_checked_direction() {
		let requested = Vector::new(2.0, -3.0, -4.0).normalized().expect("nonzero direction");

		let camera = Camera::new().with_direction(requested);
		let actual = camera.direction();

		assert!((actual.x() - requested.x()).abs() < 0.000_001);
		assert!((actual.y() - requested.y()).abs() < 0.000_001);
		assert!((actual.z() - requested.z()).abs() < 0.000_001);
	}

	#[test]
	fn position_orientation_and_inspector_updates_share_camera_state() {
		let mut camera = Camera::new();
		camera.set_position(Point::new(1.0, 2.0, 3.0));
		let orientation = Orientation::try_from_axis_angle(UnitVector::<math::WorldSpace>::y_axis(), math::Radians::new(0.5))
			.expect("finite angle around a checked axis");
		<Camera as Orientable>::set_orientation(&mut camera, orientation);
		camera.set("fov", "72.5").expect("numeric field of view");

		assert_eq!(camera.position(), Point::new(1.0, 2.0, 3.0));
		assert_eq!(camera.vertical_fov(), Degrees::new(72.5));
		assert!(camera.as_string().contains("72.5"));

		let invalid = camera.set("fov", "wide").expect_err("non-numeric field of view");

		assert!(invalid.contains("most likely cause"));
		assert_eq!(camera.vertical_fov(), Degrees::new(72.5));

		let unknown = camera.set("exposure", "1").expect_err("unsupported field");

		assert!(unknown.contains("most likely cause"));
		assert_eq!(camera.vertical_fov(), Degrees::new(72.5));
	}
}

use math::{direction_from_orientation, orientation_from_direction, Degrees, Orientation, Point, UnitVector, Vector};

use crate::core::{Entity, EntityHandle};
use crate::inspector::Inspectable;
use crate::space::orientable::Orientable;
